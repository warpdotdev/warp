use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use futures_lite::future;

use super::spacectl::{SpacectlCacheMode, SpacectlMount, SpacectlMountResponse};
use super::{
    CacheFileSystem, CacheInvocationScope, CacheMountScope, CacheSetupError, PlatformCacheMode,
    RepoCacheKey, Repository, SHARED_SCRATCH_CWD, SpacectlClient, build_mount_plan,
    run_cache_setup,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum AdapterCall {
    Detected {
        cwd: PathBuf,
        root: PathBuf,
    },
    Shared {
        cwd: PathBuf,
        root: PathBuf,
        modes: Vec<String>,
    },
}

#[derive(Default)]
struct FakeResponse {
    input_modes: Vec<String>,
    environment: BTreeMap<String, String>,
    mounts: Vec<(String, bool)>,
}

#[derive(Default)]
struct FakeSpacectlClient {
    responses: Mutex<BTreeMap<AdapterCall, Result<FakeResponse, String>>>,
    pending: Mutex<BTreeSet<AdapterCall>>,
    calls: Mutex<Vec<AdapterCall>>,
}

impl FakeSpacectlClient {
    fn set_response(&self, call: AdapterCall, result: Result<FakeResponse, &str>) {
        self.responses
            .lock()
            .unwrap()
            .insert(call, result.map_err(str::to_owned));
    }

    fn calls(&self) -> Vec<AdapterCall> {
        self.calls.lock().unwrap().clone()
    }

    async fn response_for(
        &self,
        call: AdapterCall,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        self.calls.lock().unwrap().push(call.clone());
        if self.pending.lock().unwrap().contains(&call) {
            return future::pending().await;
        }
        let response = self
            .responses
            .lock()
            .unwrap()
            .remove(&call)
            .unwrap_or_else(|| Ok(FakeResponse::default()))
            .map_err(CacheSetupError::Operation)?;
        let input_modes = response
            .input_modes
            .into_iter()
            .map(|mode| {
                SpacectlCacheMode::new(mode)
                    .map_err(|error| CacheSetupError::Operation(error.to_string()))
            })
            .collect::<Result<_, _>>()?;
        let mounts = response
            .mounts
            .into_iter()
            .map(|(mode, cache_hit)| {
                Ok(SpacectlMount {
                    mode: SpacectlCacheMode::new(mode)
                        .map_err(|error| CacheSetupError::Operation(error.to_string()))?,
                    cache_path: "/private/cache".to_owned(),
                    mount_path: "/private/mount".to_owned(),
                    cache_hit,
                })
            })
            .collect::<Result<_, CacheSetupError>>()?;
        Ok(SpacectlMountResponse {
            input_modes,
            add_envs: response.environment,
            disk_usage: None,
            mounts,
        })
    }
}

#[async_trait]
impl SpacectlClient for FakeSpacectlClient {
    async fn mount_detected_cache(
        &self,
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        self.response_for(AdapterCall::Detected {
            cwd: cwd.to_path_buf(),
            root: cache_root.to_path_buf(),
        })
        .await
    }

    async fn mount_cache(
        &self,
        modes: &[SpacectlCacheMode],
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        self.response_for(AdapterCall::Shared {
            cwd: cwd.to_path_buf(),
            root: cache_root.to_path_buf(),
            modes: modes
                .iter()
                .map(SpacectlCacheMode::as_str)
                .map(str::to_owned)
                .collect(),
        })
        .await
    }
}

struct FakeCacheFileSystem {
    create_failures: Mutex<BTreeSet<PathBuf>>,
    empty_failure: bool,
    created: Mutex<Vec<PathBuf>>,
    emptied: Mutex<Vec<PathBuf>>,
}

impl FakeCacheFileSystem {
    fn new() -> Self {
        Self {
            create_failures: Mutex::new(BTreeSet::new()),
            empty_failure: false,
            created: Mutex::new(Vec::new()),
            emptied: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CacheFileSystem for FakeCacheFileSystem {
    async fn create_dir_all(&self, path: &Path) -> Result<(), CacheSetupError> {
        self.created.lock().unwrap().push(path.to_path_buf());
        if self.create_failures.lock().unwrap().contains(path) {
            Err(CacheSetupError::Operation(
                "root creation failed".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    async fn prepare_empty_dir(&self, path: &Path) -> Result<(), CacheSetupError> {
        self.emptied.lock().unwrap().push(path.to_path_buf());
        if self.empty_failure {
            Err(CacheSetupError::Operation(
                "scratch creation failed".to_owned(),
            ))
        } else {
            Ok(())
        }
    }
}

fn repo(forge_host: &str, owner: &str, name: &str) -> Repository {
    Repository::new(
        forge_host,
        owner,
        name,
        PathBuf::from("/workspace").join(name),
    )
}

fn repo_call(source: &Repository) -> AdapterCall {
    AdapterCall::Detected {
        cwd: source.checkout_path.clone(),
        root: PathBuf::from("/cache/build")
            .join("repos")
            .join(RepoCacheKey::for_repo(source).unwrap().as_str()),
    }
}

fn shared_call(modes: &[&str]) -> AdapterCall {
    AdapterCall::Shared {
        cwd: PathBuf::from(SHARED_SCRATCH_CWD),
        root: PathBuf::from("/cache/build/shared"),
        modes: modes.iter().map(|mode| (*mode).to_owned()).collect(),
    }
}

fn response(
    input_modes: &[&str],
    environment: &[(&str, &str)],
    mounts: &[(&str, bool)],
) -> FakeResponse {
    FakeResponse {
        input_modes: input_modes.iter().map(|mode| (*mode).to_owned()).collect(),
        environment: map(environment),
        mounts: mounts
            .iter()
            .map(|(mode, hit)| ((*mode).to_owned(), *hit))
            .collect(),
    }
}

fn map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn repository_cache_keys_are_validated() {
    assert!(RepoCacheKey::new("a".repeat(64)).is_ok());
    assert!(RepoCacheKey::new("a".repeat(63)).is_err());
    assert!(RepoCacheKey::new("A".repeat(64)).is_err());
    assert!(RepoCacheKey::new("g".repeat(64)).is_err());
}

#[test]
fn repo_cache_key_is_stable_and_uses_full_source_identity() {
    let github = repo("github.com", "warpdotdev", "warp");
    let github_again = repo("github.com", "warpdotdev", "warp");
    let gitlab = repo("gitlab.com", "warpdotdev", "warp");
    let other_owner = repo("github.com", "other", "warp");
    assert_eq!(
        RepoCacheKey::for_repo(&github).unwrap().as_str(),
        "70eb5a4d6f843ac6c0758e79fc8c4988f499801e390c2060c360c456ab858e89"
    );
    assert_eq!(
        RepoCacheKey::for_repo(&github).unwrap(),
        RepoCacheKey::for_repo(&github_again).unwrap()
    );
    assert_ne!(
        RepoCacheKey::for_repo(&github).unwrap(),
        RepoCacheKey::for_repo(&gitlab).unwrap()
    );
    assert_ne!(
        RepoCacheKey::for_repo(&github).unwrap(),
        RepoCacheKey::for_repo(&other_owner).unwrap()
    );
}

#[test]
fn plan_orders_repositories_by_key_independent_of_input_order_and_checkout_path() {
    let first = repo("github.com", "warpdotdev", "warp");
    let second = repo("gitlab.com", "warpdotdev", "warp");
    let forward = build_mount_plan(&[first.clone(), second.clone()], Path::new("/cache/build"));
    let reverse = build_mount_plan(&[second, first], Path::new("/cache/build"));

    let forward_keys = forward
        .repositories
        .iter()
        .map(|invocation| match &invocation.scope {
            CacheMountScope::Repository { key, .. } => key.clone(),
            CacheMountScope::Shared { .. } => panic!("repository scope expected"),
        })
        .collect::<Vec<_>>();
    let reverse_keys = reverse
        .repositories
        .iter()
        .map(|invocation| match &invocation.scope {
            CacheMountScope::Repository { key, .. } => key.clone(),
            CacheMountScope::Shared { .. } => panic!("repository scope expected"),
        })
        .collect::<Vec<_>>();
    assert_eq!(forward_keys, reverse_keys);
    assert!(forward_keys[0] < forward_keys[1]);
    assert_eq!(forward.scratch_cwd, PathBuf::from(SHARED_SCRATCH_CWD));
    assert!(!forward.scratch_cwd.starts_with("/workspace"));
}

#[tokio::test]
async fn executes_one_call_per_repo_then_one_sorted_deduplicated_shared_call() {
    let github = repo("github.com", "warpdotdev", "warp");
    let gitlab = repo("gitlab.com", "warpdotdev", "warp");
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(
        repo_call(&github),
        Ok(response(
            &["rust", "go", "go"],
            &[],
            &[("go", true), ("go", false)],
        )),
    );
    adapter.set_response(
        repo_call(&gitlab),
        Ok(response(&["xcode", "go"], &[], &[("xcode", true)])),
    );
    adapter.set_response(
        shared_call(&["apt", "go", "rust", "xcode"]),
        Ok(response(
            &["apt", "go", "rust", "xcode"],
            &[],
            &[("apt", false)],
        )),
    );
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[gitlab.clone(), github.clone()],
        Path::new("/cache/build"),
        Some(PlatformCacheMode::Apt),
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;

    let mut expected_repo_calls = vec![repo_call(&github), repo_call(&gitlab)];
    expected_repo_calls.sort();
    expected_repo_calls.push(shared_call(&["apt", "go", "rust", "xcode"]));
    assert_eq!(adapter.calls(), expected_repo_calls);
    assert_eq!(outcome.invocations.len(), 3);
    assert!(
        outcome
            .invocations
            .last()
            .unwrap()
            .modes
            .contains_key("rust")
    );
    assert_eq!(
        outcome.invocations[0].modes["go"].cache_hits
            + outcome.invocations[1].modes["go"].cache_hits,
        1
    );
    assert_eq!(
        outcome.invocations[0].modes["go"].cache_misses
            + outcome.invocations[1].modes["go"].cache_misses,
        1
    );
    assert_eq!(
        outcome.invocations.last().unwrap().modes["rust"].cache_hits,
        0
    );
}

#[tokio::test]
async fn failed_repo_does_not_block_later_repo_and_contributes_no_shared_modes() {
    let first = repo("github.com", "warpdotdev", "warp");
    let second = repo("github.com", "warpdotdev", "warp-server");
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(repo_call(&first), Err("missing spacectl"));
    adapter.set_response(
        repo_call(&second),
        Ok(response(&["go"], &[("REPO_ENV", "fallback")], &[])),
    );
    adapter.set_response(shared_call(&["go"]), Err("shared malformed JSON"));
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[first.clone(), second.clone()],
        Path::new("/cache/build"),
        None,
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;

    let calls = adapter.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls.contains(&repo_call(&first)));
    assert!(calls.contains(&repo_call(&second)));
    assert_eq!(calls.last(), Some(&shared_call(&["go"])));
    assert!(outcome.is_error());
    assert!(outcome.invocations[0].is_error || outcome.invocations[1].is_error);
    assert_eq!(outcome.environment, map(&[("REPO_ENV", "fallback")]));
}

#[tokio::test]
async fn shared_success_is_authoritative_and_removes_repo_only_environment_keys() {
    let source = repo("github.com", "warpdotdev", "warp");
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(
        repo_call(&source),
        Ok(response(
            &["xcode"],
            &[("XCODE_CACHE", "repo"), ("REPO_ONLY", "present")],
            &[],
        )),
    );
    adapter.set_response(
        shared_call(&["xcode"]),
        Ok(response(&["xcode"], &[("XCODE_CACHE", "shared")], &[])),
    );
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[source],
        Path::new("/cache/build"),
        None,
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(outcome.environment, map(&[("XCODE_CACHE", "shared")]));
}

#[tokio::test]
async fn shared_failure_uses_ascending_repo_overlay() {
    let first = repo("github.com", "warpdotdev", "warp");
    let second = repo("gitlab.com", "warpdotdev", "warp");
    let mut keyed = [first.clone(), second.clone()]
        .into_iter()
        .map(|source| (RepoCacheKey::for_repo(&source).unwrap(), source))
        .collect::<Vec<_>>();
    keyed.sort_by(|(left, _), (right, _)| left.cmp(right));
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(
        repo_call(&keyed[0].1),
        Ok(response(
            &["xcode"],
            &[("CONFLICT", "first"), ("FIRST_ONLY", "yes")],
            &[],
        )),
    );
    adapter.set_response(
        repo_call(&keyed[1].1),
        Ok(response(
            &["xcode"],
            &[("CONFLICT", "second"), ("SECOND_ONLY", "yes")],
            &[],
        )),
    );
    adapter.set_response(shared_call(&["xcode"]), Err("shared failed"));
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[second, first],
        Path::new("/cache/build"),
        None,
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(
        outcome.environment,
        map(&[
            ("CONFLICT", "second"),
            ("FIRST_ONLY", "yes"),
            ("SECOND_ONLY", "yes")
        ])
    );
}

#[tokio::test]
async fn root_and_scratch_failures_mark_only_affected_entries_and_skip_their_calls() {
    let first = repo("github.com", "warpdotdev", "warp");
    let second = repo("github.com", "warpdotdev", "warp-server");
    let first_call = repo_call(&first);
    let AdapterCall::Detected {
        root: first_root, ..
    } = &first_call
    else {
        unreachable!()
    };
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(repo_call(&second), Ok(response(&["go"], &[], &[])));
    let mut runtime = FakeCacheFileSystem::new();
    runtime
        .create_failures
        .lock()
        .unwrap()
        .insert(first_root.clone());
    runtime.empty_failure = true;

    let outcome = run_cache_setup(
        &[first, second.clone()],
        Path::new("/cache/build"),
        None,
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;

    assert!(!adapter.calls().contains(&first_call));
    assert!(adapter.calls().contains(&repo_call(&second)));
    assert!(!adapter.calls().contains(&shared_call(&["go"])));
    assert_eq!(outcome.invocations.len(), 3);
    assert!(outcome.invocations[0].is_error || outcome.invocations[1].is_error);
    assert!(outcome.invocations.last().unwrap().is_error);
    assert_eq!(
        runtime.emptied.lock().unwrap().as_slice(),
        &[PathBuf::from(SHARED_SCRATCH_CWD)]
    );
}

#[tokio::test]
async fn timeout_is_nonfatal_and_marks_partial_error() {
    let source = repo("github.com", "warpdotdev", "warp");
    let adapter = FakeSpacectlClient::default();
    adapter.pending.lock().unwrap().insert(repo_call(&source));
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[source],
        Path::new("/cache/build"),
        None,
        &adapter,
        &runtime,
        Duration::ZERO,
    )
    .await;

    assert!(outcome.is_error());
    assert!(outcome.invocations[0].is_error);
}

#[tokio::test]
async fn installed_platform_mode_creates_one_shared_call_with_zero_repositories() {
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(shared_call(&["brew"]), Ok(response(&["brew"], &[], &[])));
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[],
        Path::new("/cache/build"),
        Some(PlatformCacheMode::Brew),
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(adapter.calls(), vec![shared_call(&["brew"])]);
    assert_eq!(outcome.invocations.len(), 1);
    assert!(matches!(
        outcome.invocations[0].scope,
        CacheInvocationScope::Shared
    ));
}

#[tokio::test]
async fn event_report_contains_only_opaque_repo_key_and_modes_not_raw_paths_or_names() {
    let source = repo("github.com", "private-owner", "private-repo");
    let adapter = FakeSpacectlClient::default();
    adapter.set_response(
        repo_call(&source),
        Ok(response(&["go"], &[], &[("go", true)])),
    );
    adapter.set_response(
        shared_call(&["go"]),
        Ok(response(&["go"], &[], &[("go", false)])),
    );
    let runtime = FakeCacheFileSystem::new();

    let outcome = run_cache_setup(
        &[source],
        Path::new("/cache/build"),
        None,
        &adapter,
        &runtime,
        Duration::from_secs(1),
    )
    .await;
    let debug = format!("{outcome:?}");

    assert!(!debug.contains("private-owner"));
    assert!(!debug.contains("private-repo"));
    assert!(!debug.contains("/workspace"));
    assert!(!debug.contains("/cache/build"));
    assert!(debug.contains("repo_key"));
}
