use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use cloud_object_models::CodeForge;
use futures::future;
use warp_isolation_platform::namespace::spacectl::{
    SpacectlCacheMode, SpacectlMount, SpacectlMountResponse,
};
use warp_isolation_platform::IsolationPlatformType;

use super::{
    build_cache_root, build_export_command, build_mount_plan, run_cache_setup, CacheCommandAdapter,
    CacheMountScope, CachePlatform, CacheRuntime, CacheSetupError, NonEmptyCacheModes,
    RepoCacheKey, SessionEnvironmentExporter, SHARED_SCRATCH_CWD,
};
use crate::ai::cloud_environments::SourceRepo;
use crate::terminal::shell::ShellType;

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
struct FakeCacheCommandAdapter {
    responses: Mutex<BTreeMap<AdapterCall, Result<FakeResponse, String>>>,
    pending: Mutex<BTreeSet<AdapterCall>>,
    calls: Mutex<Vec<AdapterCall>>,
}

impl FakeCacheCommandAdapter {
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
impl CacheCommandAdapter for FakeCacheCommandAdapter {
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

struct FakeCacheRuntime {
    platform: CachePlatform,
    command_exists: bool,
    command_error: bool,
    create_failures: Mutex<BTreeSet<PathBuf>>,
    empty_failure: bool,
    created: Mutex<Vec<PathBuf>>,
    emptied: Mutex<Vec<PathBuf>>,
}

impl FakeCacheRuntime {
    fn new(platform: CachePlatform, command_exists: bool) -> Self {
        Self {
            platform,
            command_exists,
            command_error: false,
            create_failures: Mutex::new(BTreeSet::new()),
            empty_failure: false,
            created: Mutex::new(Vec::new()),
            emptied: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CacheRuntime for FakeCacheRuntime {
    fn platform(&self) -> CachePlatform {
        self.platform
    }

    async fn command_exists(&self, _command: &str) -> Result<bool, CacheSetupError> {
        if self.command_error {
            Err(CacheSetupError::Operation(
                "platform probe failed".to_owned(),
            ))
        } else {
            Ok(self.command_exists)
        }
    }

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

#[derive(Default)]
struct FakeSessionEnvironmentExporter {
    calls: Mutex<Vec<BTreeMap<String, String>>>,
    fail: bool,
}

#[async_trait]
impl SessionEnvironmentExporter for FakeSessionEnvironmentExporter {
    async fn export(&self, environment: &BTreeMap<String, String>) -> Result<(), CacheSetupError> {
        self.calls.lock().unwrap().push(environment.clone());
        if self.fail {
            Err(CacheSetupError::Operation("export failed".to_owned()))
        } else {
            Ok(())
        }
    }
}

fn repo(forge: CodeForge, owner: &str, name: &str) -> SourceRepo {
    SourceRepo::new(forge, owner.to_owned(), name.to_owned())
}

fn repo_call(source: &SourceRepo) -> AdapterCall {
    AdapterCall::Detected {
        cwd: PathBuf::from("/workspace").join(&source.repo),
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
fn cache_root_requires_namespace_and_nonempty_absolute_path() {
    assert_eq!(
        build_cache_root(
            Some(IsolationPlatformType::Namespace),
            Some(OsString::from("/cache/build"))
        ),
        Some(PathBuf::from("/cache/build"))
    );
    assert_eq!(
        build_cache_root(
            Some(IsolationPlatformType::Docker),
            Some(OsString::from("/cache/build"))
        ),
        None
    );
    assert_eq!(
        build_cache_root(Some(IsolationPlatformType::Namespace), None),
        None
    );
    assert_eq!(
        build_cache_root(
            Some(IsolationPlatformType::Namespace),
            Some(OsString::new())
        ),
        None
    );
    assert_eq!(
        build_cache_root(
            Some(IsolationPlatformType::Namespace),
            Some(OsString::from("cache/build"))
        ),
        None
    );
}

#[test]
fn typed_scopes_reject_empty_shared_modes_and_validate_repo_keys() {
    assert!(NonEmptyCacheModes::new(Vec::new()).is_err());
    assert!(RepoCacheKey::new("a".repeat(64)).is_ok());
    assert!(RepoCacheKey::new("a".repeat(63)).is_err());
    assert!(RepoCacheKey::new("A".repeat(64)).is_err());
    assert!(RepoCacheKey::new("g".repeat(64)).is_err());
}

#[test]
fn repo_cache_key_is_stable_and_uses_full_source_identity() {
    let github = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let github_again = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let gitlab = repo(CodeForge::GitLab, "warpdotdev", "warp");
    let other_owner = repo(CodeForge::GitHub, "other", "warp");
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
    let first = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let second = repo(CodeForge::GitLab, "warpdotdev", "warp");
    let forward = build_mount_plan(
        &[first.clone(), second.clone()],
        Path::new("/one"),
        Path::new("/cache/build"),
    );
    let reverse = build_mount_plan(
        &[second, first],
        Path::new("/two"),
        Path::new("/cache/build"),
    );

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
    assert!(!forward.scratch_cwd.starts_with("/one"));
}

#[tokio::test]
async fn executes_one_call_per_repo_then_one_sorted_deduplicated_shared_call() {
    let github = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let gitlab = repo(CodeForge::GitLab, "warpdotdev", "warp");
    let adapter = FakeCacheCommandAdapter::default();
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
    let runtime = FakeCacheRuntime::new(CachePlatform::Linux, true);
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[gitlab.clone(), github.clone()],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
        Duration::from_secs(1),
    )
    .await;

    let mut expected_repo_calls = vec![repo_call(&github), repo_call(&gitlab)];
    expected_repo_calls.sort();
    expected_repo_calls.push(shared_call(&["apt", "go", "rust", "xcode"]));
    assert_eq!(adapter.calls(), expected_repo_calls);
    assert_eq!(outcome.invocations.len(), 3);
    assert!(outcome
        .invocations
        .last()
        .unwrap()
        .modes
        .contains_key("rust"));
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
    assert_eq!(exporter.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_repo_does_not_block_later_repo_and_contributes_no_shared_modes() {
    let first = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let second = repo(CodeForge::GitHub, "warpdotdev", "warp-server");
    let adapter = FakeCacheCommandAdapter::default();
    adapter.set_response(repo_call(&first), Err("missing spacectl"));
    adapter.set_response(
        repo_call(&second),
        Ok(response(&["go"], &[("REPO_ENV", "fallback")], &[])),
    );
    adapter.set_response(shared_call(&["go"]), Err("shared malformed JSON"));
    let runtime = FakeCacheRuntime::new(CachePlatform::Other, false);
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[first.clone(), second.clone()],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
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
    assert_eq!(
        outcome.exported_environment,
        map(&[("REPO_ENV", "fallback")])
    );
}

#[tokio::test]
async fn shared_success_is_authoritative_and_removes_repo_only_environment_keys() {
    let source = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let adapter = FakeCacheCommandAdapter::default();
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
    let runtime = FakeCacheRuntime::new(CachePlatform::Other, false);
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[source],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(
        outcome.exported_environment,
        map(&[("XCODE_CACHE", "shared")])
    );
    assert_eq!(
        exporter.calls.lock().unwrap().as_slice(),
        &[outcome.exported_environment]
    );
}

#[tokio::test]
async fn shared_failure_uses_ascending_repo_overlay_and_exports_once() {
    let first = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let second = repo(CodeForge::GitLab, "warpdotdev", "warp");
    let mut keyed = [first.clone(), second.clone()]
        .into_iter()
        .map(|source| (RepoCacheKey::for_repo(&source).unwrap(), source))
        .collect::<Vec<_>>();
    keyed.sort_by(|(left, _), (right, _)| left.cmp(right));
    let adapter = FakeCacheCommandAdapter::default();
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
    let runtime = FakeCacheRuntime::new(CachePlatform::Other, false);
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[second, first],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(
        outcome.exported_environment,
        map(&[
            ("CONFLICT", "second"),
            ("FIRST_ONLY", "yes"),
            ("SECOND_ONLY", "yes")
        ])
    );
    assert_eq!(exporter.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn root_and_scratch_failures_mark_only_affected_entries_and_skip_their_calls() {
    let first = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let second = repo(CodeForge::GitHub, "warpdotdev", "warp-server");
    let first_call = repo_call(&first);
    let AdapterCall::Detected {
        root: first_root, ..
    } = &first_call
    else {
        unreachable!()
    };
    let adapter = FakeCacheCommandAdapter::default();
    adapter.set_response(repo_call(&second), Ok(response(&["go"], &[], &[])));
    let mut runtime = FakeCacheRuntime::new(CachePlatform::Other, false);
    runtime
        .create_failures
        .lock()
        .unwrap()
        .insert(first_root.clone());
    runtime.empty_failure = true;
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[first, second.clone()],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
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
async fn timeout_and_export_failure_are_nonfatal_and_mark_partial_error() {
    let source = repo(CodeForge::GitHub, "warpdotdev", "warp");
    let adapter = FakeCacheCommandAdapter::default();
    adapter.pending.lock().unwrap().insert(repo_call(&source));
    let runtime = FakeCacheRuntime::new(CachePlatform::Other, false);
    let exporter = FakeSessionEnvironmentExporter {
        fail: true,
        ..Default::default()
    };

    let outcome = run_cache_setup(
        &[source],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
        Duration::ZERO,
    )
    .await;

    assert!(outcome.is_error());
    assert!(outcome.invocations[0].is_error);
    assert_eq!(exporter.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn installed_platform_mode_creates_one_shared_call_with_zero_repositories() {
    let adapter = FakeCacheCommandAdapter::default();
    adapter.set_response(shared_call(&["brew"]), Ok(response(&["brew"], &[], &[])));
    let runtime = FakeCacheRuntime::new(CachePlatform::MacOS, true);
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[],
        Path::new("/workspace"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(adapter.calls(), vec![shared_call(&["brew"])]);
    assert_eq!(outcome.invocations.len(), 1);
    assert!(matches!(
        outcome.invocations[0].scope,
        CacheMountScope::Shared { .. }
    ));
}

#[test]
fn export_command_is_single_safe_shell_command_and_rejects_invalid_names() {
    let command = build_export_command(
        &map(&[("ALPHA", "one"), ("CACHE_VALUE", "value with ' quote")]),
        ShellType::Bash,
    )
    .unwrap();
    assert_eq!(
        command,
        "export ALPHA='one'\nexport CACHE_VALUE='value with '\"'\"' quote'"
    );
    assert!(build_export_command(&map(&[("INVALID-NAME", "value")]), ShellType::Bash).is_err());
}

#[tokio::test]
async fn event_report_contains_only_opaque_repo_key_and_modes_not_raw_paths_or_names() {
    let source = repo(CodeForge::GitHub, "private-owner", "private-repo");
    let adapter = FakeCacheCommandAdapter::default();
    adapter.set_response(
        repo_call(&source),
        Ok(response(&["go"], &[], &[("go", true)])),
    );
    adapter.set_response(
        shared_call(&["go"]),
        Ok(response(&["go"], &[], &[("go", false)])),
    );
    let runtime = FakeCacheRuntime::new(CachePlatform::Other, false);
    let exporter = FakeSessionEnvironmentExporter::default();

    let outcome = run_cache_setup(
        &[source],
        Path::new("/workspace/secret"),
        Path::new("/cache/build"),
        &adapter,
        &runtime,
        &exporter,
        Duration::from_secs(1),
    )
    .await;
    let debug = format!("{:?}", outcome.into_event_report());

    assert!(!debug.contains("private-owner"));
    assert!(!debug.contains("private-repo"));
    assert!(!debug.contains("/workspace"));
    assert!(!debug.contains("/cache/build"));
    assert!(debug.contains("repo_key"));
}
