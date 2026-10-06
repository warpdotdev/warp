use std::io::{BufRead as _, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{fs, thread};

use async_compat::Compat;
use command::Stdio;
use command::blocking::Command;
use futures::executor::block_on;
use tempfile::TempDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};
use warp_cli::environment_checkout::{
    CheckoutBatch, CheckoutFailureKind, CheckoutFailureReport, CheckoutRequest,
    EnvironmentCheckoutArgs,
};

use super::{Git, capture, mirror_key, optional_mirror_root, run};

const CANONICAL_URL: &str = "https://github.com/fixtures/source.git";

#[test]
fn cleanup_only_preserves_refs_and_reports_failures_without_cloning() {
    fixture_test(
        "cleanup_only_preserves_refs_and_reports_failures_without_cloning",
        |fixture| {
            fixture.prepare(fixture.request("cleanup", None), false);
            fixture.prepare(fixture.request("preserved", None), false);
            let target = fixture.work().join("cleanup");
            let packed = format!(
                "{} refs/remotes/origin/Foo\n{} refs/remotes/origin/foo\n",
                fixture.base(),
                fixture.base()
            );
            fs::write(target.join(".git/packed-refs"), &packed).unwrap();
            fs::write(target.join(".git/config.lock"), "").unwrap();
            let directory = TempDir::new().unwrap();
            let args = EnvironmentCheckoutArgs {
                requests_file: directory.path().join("requests.json"),
                failure_report: directory.path().join("failures.json"),
                remove_origins_only: true,
            };
            let batch = fixture.batch(vec![
                fixture.request(
                    "cleanup",
                    Some(RepositoryHeadRef::Branch("missing".to_owned())),
                ),
                fixture.request("not-cloned", None),
            ]);
            fs::write(&args.requests_file, serde_json::to_vec(&batch).unwrap()).unwrap();
            assert!(run(&args).is_err());
            let report: CheckoutFailureReport =
                serde_json::from_slice(&fs::read(&args.failure_report).unwrap()).unwrap();
            assert_eq!(
                report
                    .failures
                    .iter()
                    .map(|failure| (failure.request_index, failure.kind))
                    .collect::<Vec<_>>(),
                vec![
                    (0, CheckoutFailureKind::RemoveOrigin),
                    (1, CheckoutFailureKind::RemoveOrigin)
                ]
            );
            assert!(!fixture.work().join("not-cloned").exists());
            fs::remove_file(target.join(".git/config.lock")).unwrap();
            fs::write(
                &args.requests_file,
                serde_json::to_vec(&fixture.batch(vec![batch.repositories[0].clone()])).unwrap(),
            )
            .unwrap();
            run(&args).unwrap();
            run(&args).unwrap();
            assert_eq!(git(&target, &["remote"]), "");
            assert_eq!(
                fs::read_to_string(target.join(".git/packed-refs")).unwrap(),
                packed
            );
            assert_eq!(
                git(
                    &fixture.work().join("preserved"),
                    &["remote", "get-url", "origin"]
                ),
                url::Url::from_file_path(fixture.origin())
                    .unwrap()
                    .to_string()
            );
        },
    );
}

#[test]
fn distinct_sources_run_concurrently_with_a_bounded_active_count() {
    fixture_test(
        "distinct_sources_run_concurrently_with_a_bounded_active_count",
        |fixture| {
            let file_url = url::Url::from_file_path(fixture.origin())
                .unwrap()
                .to_string();
            let mut requests = Vec::new();
            for index in 0..6 {
                let mut request = fixture.request(&format!("checkout-{index}"), None);
                request.source.repo_name = format!("source-{index}");
                git(
                    &fixture.root,
                    &[
                        "config",
                        "--file",
                        fixture.root.join("gitconfig").to_str().unwrap(),
                        "--add",
                        &format!("url.{file_url}.insteadOf"),
                        &format!("https://github.com/fixtures/source-{index}.git"),
                    ],
                );
                requests.push(request);
            }
            let mut output = Vec::new();
            let root = optional_mirror_root();
            let results =
                checkout_batch(&fixture.batch(requests), root.as_deref(), &mut output).unwrap();
            assert!(results.iter().all(Result::is_ok), "{results:?}");
            let mut active = 0;
            let mut maximum = 0;
            for line in String::from_utf8(output).unwrap().lines() {
                if line.ends_with(": started") {
                    active += 1;
                    maximum = maximum.max(active);
                } else if line.ends_with(": finished") || line.ends_with(": failed") {
                    active -= 1;
                }
            }
            assert_eq!(active, 0);
            assert_eq!(maximum, super::CHECKOUT_WORKERS);
        },
    );
}

fn checkout_batch(
    batch: &CheckoutBatch,
    mirror_root: Option<&Path>,
    output: &mut impl std::io::Write,
) -> anyhow::Result<Vec<super::CheckoutResult>> {
    block_on(Compat::new(super::checkout_batch(
        batch,
        mirror_root,
        false,
        output,
    )))
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn absent_unwritable_and_non_namespace_roots_use_network_checkout() {
    if let Ok(expected) = std::env::var("WARP_TEST_MIRROR_ROOT_EXPECTED") {
        assert_eq!(optional_mirror_root().is_some(), expected == "present");
        return;
    }
    fixture_test(
        "absent_unwritable_and_non_namespace_roots_use_network_checkout",
        |fixture| {
            for scenario in ["absent", "other-platform", "unusable"] {
                let mut command = Command::new(std::env::current_exe().unwrap());
                #[cfg(windows)]
                command.creation_flags(0);
                command.args(["--exact", "ai::agent_sdk::driver::environment_checkout::tests::absent_unwritable_and_non_namespace_roots_use_network_checkout", "--nocapture"])
                .env("WARP_TEST_MIRROR_ROOT_EXPECTED", "absent");
                match scenario {
                    "absent" => {
                        command.env_remove("WARP_BUILD_CACHE_ROOT");
                    }
                    "other-platform" => {
                        command.env("WARP_ISOLATION_PLATFORM", "docker");
                    }
                    "unusable" => {
                        let file = fixture.root.join("unusable");
                        fs::write(&file, "not a directory").unwrap();
                        command.env("WARP_BUILD_CACHE_ROOT", file);
                    }
                    _ => unreachable!(),
                }
                let output = command.output().unwrap();
                assert!(
                    output.status.success(),
                    "{scenario}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let path = fixture.root.join("cache");
                let permissions = fs::metadata(&path).unwrap().permissions();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
                assert!(optional_mirror_root().is_none());
                fs::set_permissions(&path, permissions).unwrap();
            }
            fixture.prepare(fixture.request("network", None), false);
            assert_eq!(
                git(
                    &fixture.work().join("network"),
                    &["config", "remote.origin.promisor"]
                ),
                "true"
            );
            #[cfg(windows)]
            assert!(optional_mirror_root().is_none());
        },
    );
}

#[test]
fn direct_clone_failure_is_fatal_and_attributed_to_its_request() {
    fixture_test(
        "direct_clone_failure_is_fatal_and_attributed_to_its_request",
        |fixture| {
            let config = fixture.root.join("gitconfig");
            let prefix = url::Url::from_directory_path(&fixture.root)
                .unwrap()
                .to_string();
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    config.to_str().unwrap(),
                    &format!("url.{prefix}.insteadOf"),
                    "https://github.com/fixtures/",
                ],
            );
            let mut bad = fixture.request("bad-clone", None);
            bad.source.repo_name = "missing".to_owned();
            let mut output = Vec::new();
            let results = checkout_batch(
                &fixture.batch(vec![fixture.request("good-clone", None), bad]),
                None,
                &mut output,
            )
            .unwrap();
            assert!(results[0].is_ok());
            assert_eq!(
                results[1].as_ref().unwrap_err().0,
                CheckoutFailureKind::Clone
            );
            assert!(
                String::from_utf8(output)
                    .unwrap()
                    .contains("Repository 2: failed")
            );
        },
    );
}

struct Fixture {
    root: PathBuf,
}

#[test]
fn empty_remote_can_be_cloned_without_a_requested_head() {
    fixture_test(
        "empty_remote_can_be_cloned_without_a_requested_head",
        |fixture| {
            git(&fixture.origin(), &["update-ref", "-d", "refs/heads/main"]);
            fixture.prepare(fixture.request("empty", None), false);
            assert!(fixture.work().join("empty/.git").is_dir());
        },
    );
}

impl Fixture {
    fn origin(&self) -> PathBuf {
        self.root.join("origin.git")
    }
    fn work(&self) -> PathBuf {
        self.root.join("work")
    }
    fn seed(&self) -> PathBuf {
        self.root.join("seed")
    }
    fn base(&self) -> String {
        git(&self.origin(), &["rev-parse", "main"])
    }
    fn pinned(&self) -> String {
        git(&self.origin(), &["rev-parse", "refs/pinned/base"])
    }
    fn request(&self, name: &str, head: Option<RepositoryHeadRef>) -> CheckoutRequest {
        CheckoutRequest {
            source: RepositoryIdentity {
                code_forge: RepositoryForge::GitHub,
                repo_owner: "fixtures".to_owned(),
                repo_name: "source".to_owned(),
            },
            checkout_name: name.to_owned(),
            head,
            fetch_branch_only: false,
        }
    }
    fn batch(&self, repositories: Vec<CheckoutRequest>) -> CheckoutBatch {
        CheckoutBatch {
            working_dir: self.work(),
            repositories,
        }
    }
    fn mirror(&self) -> PathBuf {
        self.root
            .join("cache/git-mirrors")
            .join(mirror_key(&self.request("source", None)).as_str())
    }
    fn prepare(&self, request: CheckoutRequest, cached: bool) -> String {
        let root = cached.then(optional_mirror_root).flatten();
        let mut output = Vec::new();
        let results =
            checkout_batch(&self.batch(vec![request]), root.as_deref(), &mut output).unwrap();
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        String::from_utf8(output).unwrap()
    }
    fn full_objects(&self, target: &Path) {
        assert!(!target.join(".git/objects/info/alternates").exists());
        for line in git(target, &["rev-list", "--objects", "--all"]).lines() {
            let object = line.split(' ').next().unwrap();
            let output = Command::new("git")
                .current_dir(target)
                .env("GIT_NO_LAZY_FETCH", "1")
                .args([
                    "-c",
                    "remote.origin.promisor=false",
                    "cat-file",
                    "-e",
                    object,
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "missing reachable object {object}");
        }
    }
}

fn fixture_test(name: &str, test: impl FnOnce(&Fixture)) {
    if let Some(root) = std::env::var_os("WARP_TEST_CHECKOUT_ROOT") {
        test(&Fixture { root: root.into() });
        return;
    }
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    for directory in ["origin.git", "seed", "work", "cache"] {
        fs::create_dir(root.join(directory)).unwrap();
    }
    let fixture = Fixture {
        root: root.to_owned(),
    };
    git(&fixture.origin(), &["init", "--bare", "-b", "main"]);
    git(
        &fixture.origin(),
        &["config", "uploadpack.allowFilter", "true"],
    );
    git(
        &fixture.origin(),
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    );
    git(&fixture.seed(), &["init", "-b", "main"]);
    git(
        &fixture.seed(),
        &["config", "user.name", "Checkout Fixture"],
    );
    git(
        &fixture.seed(),
        &["config", "user.email", "checkout@example.com"],
    );
    for value in ["old blob\n", "current blob\n"] {
        fs::write(fixture.seed().join("README"), value).unwrap();
        git(&fixture.seed(), &["add", "."]);
        git(&fixture.seed(), &["commit", "-m", "history"]);
    }
    git(
        &fixture.seed(),
        &[
            "remote",
            "add",
            "origin",
            fixture.origin().to_str().unwrap(),
        ],
    );
    git(&fixture.seed(), &["push", "origin", "main"]);
    fs::write(fixture.seed().join("PINNED"), "hidden object\n").unwrap();
    git(&fixture.seed(), &["add", "."]);
    git(&fixture.seed(), &["commit", "-m", "hidden"]);
    git(
        &fixture.seed(),
        &["push", "origin", "HEAD:refs/pinned/base"],
    );
    let config = root.join("gitconfig");
    let url = url::Url::from_file_path(fixture.origin())
        .unwrap()
        .to_string();
    git(
        root,
        &[
            "config",
            "--file",
            config.to_str().unwrap(),
            &format!("url.{url}.insteadOf"),
            CANONICAL_URL,
        ],
    );
    let mut command = Command::new(std::env::current_exe().unwrap());
    #[cfg(windows)]
    command.creation_flags(0);
    command
        .args([
            "--exact",
            &format!("ai::agent_sdk::driver::environment_checkout::tests::{name}"),
            "--nocapture",
        ])
        .env("WARP_TEST_CHECKOUT_ROOT", root)
        .env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("WARP_ISOLATION_PLATFORM", "namespace")
        .env("WARP_BUILD_CACHE_ROOT", root.join("cache"))
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if name == "failed_reference_attempt_cleans_only_its_new_checkout" {
        use std::os::unix::fs::PermissionsExt as _;
        let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("git"))
            .find(|path| path.is_file())
            .unwrap();
        let directory = root.join("bin");
        fs::create_dir(&directory).unwrap();
        let wrapper = directory.join("git");
        fs::write(&wrapper, format!(
            "#!/bin/bash\n\
             if [[ \"$1\" == clone && \" $* \" == *' --reference '* && -e '{root}/fail-reference' ]]; then\n\
               rm '{root}/fail-reference'\n\
               target=\"${{@: -1}}\"\n\
               '{git}' init --quiet \"$target\"\n\
               printf partial > \"$target/PARTIAL\"\n\
               exit 1\n\
             fi\n\
             exec '{git}' \"$@\"\n",
             root = root.display(), git = real_git.display(),
        )).unwrap();
        fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let mut paths = vec![directory];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        command.env("PATH", std::env::join_paths(paths).unwrap());
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn cold_and_warm_mirrors_refresh_default_and_remain_independent() {
    fixture_test(
        "cold_and_warm_mirrors_refresh_default_and_remain_independent",
        |fixture| {
            git(
                &fixture.origin(),
                &["update-ref", "refs/tags/v1", &fixture.pinned()],
            );
            assert!(
                fixture
                    .prepare(fixture.request("cold", None), true)
                    .contains("cold cache population")
            );
            let cold = fixture.work().join("cold");
            assert_eq!(
                git(&cold, &["config", "--local", "remote.origin.url"]),
                CANONICAL_URL
            );
            assert_eq!(git(&cold, &["rev-parse", "HEAD"]), fixture.base());
            assert_eq!(
                git(&fixture.mirror(), &["rev-parse", "refs/tags/v1"]),
                fixture.pinned()
            );
            fixture.full_objects(&cold);
            let pinned = fixture.pinned();
            git(
                &fixture.origin(),
                &["update-ref", "refs/heads/trunk", &pinned],
            );
            git(&fixture.origin(), &["update-ref", "-d", "refs/heads/main"]);
            git(
                &fixture.origin(),
                &["symbolic-ref", "HEAD", "refs/heads/trunk"],
            );
            git(&fixture.origin(), &["update-ref", "-d", "refs/tags/v1"]);
            assert!(
                fixture
                    .prepare(fixture.request("warm", None), true)
                    .contains("cache hit")
            );
            let warm = fixture.work().join("warm");
            assert_eq!(git(&warm, &["rev-parse", "HEAD"]), pinned);
            assert_eq!(
                git(&fixture.mirror(), &["symbolic-ref", "HEAD"]),
                "refs/heads/trunk"
            );
            assert!(git(&fixture.mirror(), &["for-each-ref", "refs/tags"]).is_empty());
            fs::remove_dir_all(fixture.mirror()).unwrap();
            git(
                &warm,
                &[
                    "remote",
                    "set-url",
                    "origin",
                    "https://127.0.0.1:1/unreachable.git",
                ],
            );
            fixture.full_objects(&warm);
            fixture.full_objects(&cold);
        },
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn partial_mirrors_are_rebuilt_and_full_mirror_config_is_normalized() {
    fixture_test(
        "partial_mirrors_are_rebuilt_and_full_mirror_config_is_normalized",
        |fixture| {
            let root = optional_mirror_root().unwrap();
            fs::create_dir(fixture.mirror()).unwrap();
            assert!(
                fixture
                    .prepare(fixture.request("partial", None), true)
                    .contains("invalid cache; rebuilding")
            );
            git(
                &fixture.mirror(),
                &["config", "remote.origin.promisor", "true"],
            );
            assert!(
                fixture
                    .prepare(fixture.request("filtered", None), true)
                    .contains("invalid cache; rebuilding")
            );
            fs::write(
                fixture.mirror().join("objects/pack/incomplete.promisor"),
                "",
            )
            .unwrap();
            assert!(
                fixture
                    .prepare(fixture.request("promisor-pack", None), true)
                    .contains("invalid cache; rebuilding")
            );
            git(
                &fixture.mirror(),
                &[
                    "config",
                    "remote.origin.url",
                    "https://user:fake-secret@example.com/repo",
                ],
            );
            fs::write(fixture.mirror().join("KEEP"), "retained objects").unwrap();
            git(
                &fixture.mirror(),
                &[
                    "config",
                    "remote.origin.pushurl",
                    "https://user:fake-secret@example.com/push",
                ],
            );
            git(
                &fixture.mirror(),
                &["config", "http.extraHeader", "Authorization: fake-secret"],
            );
            git(&fixture.mirror(), &["config", "core.worktree", "/outside"]);
            git(
                &fixture.mirror(),
                &["config", "include.path", "/missing/config"],
            );
            assert!(
                fixture
                    .prepare(fixture.request("credential", None), true)
                    .contains("cache hit")
            );
            assert!(fixture.mirror().join("KEEP").exists());
            assert_eq!(
                git(
                    &fixture.mirror(),
                    &["config", "--local", "remote.origin.url"]
                ),
                CANONICAL_URL
            );
            assert!(
                !fs::read_to_string(fixture.mirror().join("config"))
                    .unwrap()
                    .contains("fake-secret")
            );
            let cache = fs::canonicalize(fixture.root.join("cache")).unwrap();
            assert_eq!(root.parent(), Some(cache.as_path()));
        },
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn failed_refresh_falls_back_instead_of_serving_stale_history() {
    fixture_test(
        "failed_refresh_falls_back_instead_of_serving_stale_history",
        |fixture| {
            fixture.prepare(fixture.request("initial", None), true);
            let pinned = fixture.pinned();
            git(
                &fixture.origin(),
                &["update-ref", "refs/heads/main", &pinned],
            );
            fs::remove_file(fixture.mirror().join("FETCH_HEAD")).unwrap();
            fs::create_dir(fixture.mirror().join("FETCH_HEAD")).unwrap();
            let progress = fixture.prepare(fixture.request("fallback", None), true);
            assert!(progress.contains("cache failure; using network fallback"));
            let target = fixture.work().join("fallback");
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), pinned);
            assert_eq!(git(&target, &["config", "remote.origin.promisor"]), "true");
        },
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn failed_reference_attempt_cleans_only_its_new_checkout() {
    fixture_test(
        "failed_reference_attempt_cleans_only_its_new_checkout",
        |fixture| {
            fixture.prepare(fixture.request("initial", None), true);
            fs::write(fixture.root.join("fail-reference"), "").unwrap();
            assert!(
                fixture
                    .prepare(fixture.request("recovered", None), true)
                    .contains("network fallback")
            );
            assert_eq!(
                git(
                    &fixture.work().join("recovered"),
                    &["config", "remote.origin.promisor"]
                ),
                "true"
            );
            assert!(!fixture.work().join("recovered/PARTIAL").exists());
            let request = fixture.request("raced", None);
            let target = fixture.work().join("raced");
            let mut adapter = Git::default();
            let result = block_on(Compat::new(super::checkout(
                &request,
                &fixture.work(),
                optional_mirror_root().as_deref(),
                &mut adapter,
                &|category| {
                    if category == "cache hit" {
                        fs::create_dir(&target).unwrap();
                        fs::write(target.join("KEEP"), "pre-existing target").unwrap();
                    }
                },
            )));
            assert!(result.is_err());
            assert_eq!(
                fs::read_to_string(target.join("KEEP")).unwrap(),
                "pre-existing target"
            );
        },
    );
}

#[test]
fn pins_fetch_hidden_objects_and_prefer_fresh_remote_refs() {
    fixture_test(
        "pins_fetch_hidden_objects_and_prefer_fresh_remote_refs",
        |fixture| {
            let pinned = fixture.pinned();
            let cached = cfg!(any(target_os = "linux", target_os = "macos"));
            fixture.prepare(
                fixture.request("hidden", Some(RepositoryHeadRef::CommitSha(pinned.clone()))),
                cached,
            );
            let target = fixture.work().join("hidden");
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), pinned);
            assert_eq!(git(&target, &["rev-parse", "--abbrev-ref", "HEAD"]), "HEAD");
            if cached {
                fixture.full_objects(&target);
            }
            git(&target, &["branch", "feature", &fixture.base()]);
            git(
                &fixture.origin(),
                &["update-ref", "refs/heads/feature", &pinned],
            );
            fixture.prepare(
                fixture.request(
                    "hidden",
                    Some(RepositoryHeadRef::Branch("feature".to_owned())),
                ),
                true,
            );
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), pinned);
            fs::write(target.join("KEEP"), "untouched").unwrap();
            fixture.prepare(fixture.request("hidden", None), true);
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), pinned);
            assert_eq!(
                fs::read_to_string(target.join("KEEP")).unwrap(),
                "untouched"
            );
        },
    );
}

#[test]
fn parallel_substituted_branches_preserve_alias_and_attribute_failure() {
    fixture_test(
        "parallel_substituted_branches_preserve_alias_and_attribute_failure",
        |fixture| {
            let pinned = fixture.pinned();
            git(
                &fixture.origin(),
                &["update-ref", "refs/heads/trunk", &fixture.base()],
            );
            git(
                &fixture.origin(),
                &["symbolic-ref", "HEAD", "refs/heads/trunk"],
            );
            git(
                &fixture.origin(),
                &["update-ref", "refs/heads/live", &pinned],
            );
            let modes = if cfg!(any(target_os = "linux", target_os = "macos")) {
                vec![false, true]
            } else {
                vec![false]
            };
            for cached in modes {
                let mut request = fixture.request(
                    if cached {
                        "cached-alias"
                    } else {
                        "direct-alias"
                    },
                    Some(RepositoryHeadRef::Branch("live".to_owned())),
                );
                request.fetch_branch_only = true;
                let mut second = request.clone();
                second.checkout_name.push_str("-second");
                let bad = fixture.request(
                    if cached { "cached-bad" } else { "direct-bad" },
                    Some(RepositoryHeadRef::Branch("missing".to_owned())),
                );
                let root = cached.then(optional_mirror_root).flatten();
                let mut output = Vec::new();
                let results = checkout_batch(
                    &fixture.batch(vec![request.clone(), second, bad]),
                    root.as_deref(),
                    &mut output,
                )
                .unwrap();
                assert!(results[0].is_ok() && results[1].is_ok());
                assert_eq!(
                    results[2].as_ref().unwrap_err().0,
                    CheckoutFailureKind::Checkout
                );
                let target = fixture.work().join(&request.checkout_name);
                assert_eq!(git(&target, &["rev-parse", "HEAD"]), pinned);
                assert_eq!(
                    git(&target, &["config", "remote.origin.fetch"]),
                    "+refs/heads/live:refs/remotes/origin/trunk"
                );
                assert_eq!(
                    git(&target, &["symbolic-ref", "refs/remotes/origin/HEAD"]),
                    "refs/remotes/origin/trunk"
                );
                assert_eq!(
                    git(
                        &target,
                        &["for-each-ref", "--format=%(refname)", "refs/remotes/origin"]
                    ),
                    "refs/remotes/origin/HEAD\nrefs/remotes/origin/trunk"
                );
                let output = String::from_utf8(output).unwrap();
                assert!(
                    output.find("Repository 1: finished").unwrap()
                        < output.find("Repository 2: started").unwrap()
                );
                assert!(
                    output.contains("Repository 1: started")
                        && output.contains("Repository 2: finished")
                );
                assert!(
                    output.contains("Repository 3: failed")
                        && output.contains("Repository 3 diagnostics:")
                );
                if cached {
                    fixture.full_objects(&target);
                }
            }
        },
    );
}

#[test]
fn uncached_sha_fetch_avoids_later_default_history_and_preserves_failures() {
    fixture_test(
        "uncached_sha_fetch_avoids_later_default_history_and_preserves_failures",
        |fixture| {
            let base = fixture.base();
            git(
                &fixture.origin(),
                &["update-ref", "refs/heads/main", &fixture.pinned()],
            );
            fixture.prepare(
                fixture.request("sha", Some(RepositoryHeadRef::CommitSha(base.clone()))),
                false,
            );
            let target = fixture.work().join("sha");
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), base);
            assert!(!target.join("PINNED").exists());
            assert_eq!(
                git(&target, &["config", "remote.origin.partialclonefilter"]),
                "blob:none"
            );
            let mut output = Vec::new();
            let results = checkout_batch(
                &fixture.batch(vec![
                    fixture.request("sha", Some(RepositoryHeadRef::Branch("missing".to_owned()))),
                ]),
                None,
                &mut output,
            )
            .unwrap();
            assert_eq!(
                results[0].as_ref().unwrap_err().0,
                CheckoutFailureKind::Checkout
            );
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), base);
        },
    );
}

#[test]
fn helper_writes_typed_failures_and_rejects_invalid_requests_before_work() {
    fixture_test(
        "helper_writes_typed_failures_and_rejects_invalid_requests_before_work",
        |fixture| {
            let directory = TempDir::new().unwrap();
            let args = EnvironmentCheckoutArgs {
                requests_file: directory.path().join("requests.json"),
                failure_report: directory.path().join("failures.json"),
                remove_origins_only: false,
            };
            let batch = fixture.batch(vec![
                fixture.request("good", None),
                fixture.request("bad", Some(RepositoryHeadRef::Branch("missing".to_owned()))),
            ]);
            fs::write(&args.requests_file, serde_json::to_vec(&batch).unwrap()).unwrap();
            assert!(run(&args).is_err());
            let report: CheckoutFailureReport =
                serde_json::from_slice(&fs::read(&args.failure_report).unwrap()).unwrap();
            assert_eq!(report.failures.len(), 1);
            assert_eq!(report.failures[0].request_index, 1);
            assert_eq!(report.failures[0].kind, CheckoutFailureKind::Checkout);
            fs::write(&args.requests_file, b"{").unwrap();
            assert!(
                run(&args)
                    .unwrap_err()
                    .to_string()
                    .contains("invalid checkout JSON")
            );
        },
    );
}

#[cfg(unix)]
#[test]
fn mirror_symlinks_cannot_escape_the_cache_volume() {
    fixture_test(
        "mirror_symlinks_cannot_escape_the_cache_volume",
        |fixture| {
            use std::os::unix::fs::symlink;
            optional_mirror_root().unwrap();
            let outside = fixture.root.join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("KEEP"), "untouched").unwrap();
            symlink(&outside, fixture.mirror()).unwrap();
            fixture.prepare(fixture.request("safe", None), true);
            assert_eq!(
                fs::read_to_string(outside.join("KEEP")).unwrap(),
                "untouched"
            );
            assert!(
                !fs::symlink_metadata(fixture.mirror())
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_dir_all(fixture.mirror().join("objects")).unwrap();
            symlink(&outside, fixture.mirror().join("objects")).unwrap();
            fixture.prepare(fixture.request("nested-safe", None), true);
            assert_eq!(
                fs::read_to_string(outside.join("KEEP")).unwrap(),
                "untouched"
            );
            fs::remove_dir_all(fixture.root.join("cache/git-mirrors")).unwrap();
            symlink(&outside, fixture.root.join("cache/git-mirrors")).unwrap();
            assert!(optional_mirror_root().is_none());
        },
    );
}

#[test]
fn output_capture_discards_incomplete_secrets_and_bounds_large_diagnostics() {
    let text = format!("valid line\n{}", "x".repeat(128 * 1024));
    let (captured, truncated) = block_on(capture(text.as_bytes())).unwrap();
    assert!(truncated);
    assert_eq!(captured, "valid line");
    let mut git = Git::default();
    git.record(&format!("{} AKIAIOSFODNN7EXAMPLE", "line\n".repeat(2048)));
    assert!(git.diagnostics.len() <= 4096);
    assert!(!git.diagnostics.contains("AKIAIOSFODNN7EXAMPLE"));
}

struct HttpRemote {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    received: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HttpRemote {
    fn new(root: &Path, authorization: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_received = received.clone();
        let root = root.to_owned();
        let thread = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve_git_http(stream, &root, &authorization, &worker_received)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("Git fixture accept: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            received,
            thread: Some(thread),
        }
    }
}

impl Drop for HttpRemote {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn serve_git_http(mut stream: TcpStream, root: &Path, authorization: &str, received: &AtomicBool) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut length = 0;
    let mut authenticated = false;
    let mut content_type = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse::<usize>().unwrap(),
                "authorization" => authenticated = value.trim() == authorization,
                "content-type" => content_type = value.trim().to_owned(),
                _ => {}
            }
        }
    }
    if !authenticated {
        stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"fixture\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        return;
    }
    received.store(true, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(100));
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap();
    let uri = parts.next().unwrap();
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let mut child = Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("CONTENT_TYPE", content_type)
        .env("CONTENT_LENGTH", length.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&body).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n")
        .unwrap();
    stream.write_all(&output.stdout).unwrap();
}

struct ProgressObserver {
    remote_received: Arc<AtomicBool>,
    started_before_remote: bool,
    bytes: Vec<u8>,
}

impl std::io::Write for ProgressObserver {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend(bytes);
        if String::from_utf8_lossy(&self.bytes).contains("Repository 1: started")
            && !self.remote_received.load(Ordering::SeqCst)
        {
            self.started_before_remote = true;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn inherited_credentials_authenticate_without_leaking_and_progress_is_immediate() {
    fixture_test(
        "inherited_credentials_authenticate_without_leaking_and_progress_is_immediate",
        |fixture| {
            use base64::Engine as _;
            let sentinel = "fixture-only-secret-not-a-real-token";
            let auth = format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("fixture:{sentinel}"))
            );
            let remote = HttpRemote::new(&fixture.root, auth);
            let config = fixture.root.join("gitconfig");
            let old_url = url::Url::from_file_path(fixture.origin())
                .unwrap()
                .to_string();
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    config.to_str().unwrap(),
                    "--unset-all",
                    &format!("url.{old_url}.insteadOf"),
                ],
            );
            let url = format!("http://{}/origin.git", remote.address);
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    config.to_str().unwrap(),
                    &format!("url.{url}.insteadOf"),
                    CANONICAL_URL,
                ],
            );
            let credentials = fixture.root.join("credentials");
            fs::write(
                &credentials,
                format!("http://fixture:{sentinel}@{}\n", remote.address),
            )
            .unwrap();
            let credential_path = credentials.to_str().unwrap().replace('\\', "/");
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    config.to_str().unwrap(),
                    "credential.helper",
                    &format!("store --file=\"{credential_path}\""),
                ],
            );
            let batch = fixture.batch(vec![fixture.request("authenticated", None)]);
            let mut output = ProgressObserver {
                remote_received: remote.received.clone(),
                started_before_remote: false,
                bytes: Vec::new(),
            };
            let cached = cfg!(any(target_os = "linux", target_os = "macos"));
            let root = cached.then(optional_mirror_root).flatten();
            let results = checkout_batch(&batch, root.as_deref(), &mut output).unwrap();
            assert!(results[0].is_ok(), "{results:?}");
            assert!(remote.received.load(Ordering::SeqCst));
            assert!(output.started_before_remote);
            let target = fixture.work().join("authenticated");
            for value in [
                serde_json::to_string(&batch).unwrap(),
                serde_json::to_string(&CheckoutFailureReport {
                    failures: Vec::new(),
                })
                .unwrap(),
                fs::read_to_string(target.join(".git/config")).unwrap(),
                String::from_utf8(output.bytes).unwrap(),
            ] {
                assert!(!value.contains(sentinel));
            }
            if cached {
                assert!(
                    !fs::read_to_string(fixture.mirror().join("config"))
                        .unwrap()
                        .contains(sentinel)
                );
                fixture.full_objects(&target);
            }
            fixture.prepare(fixture.request("blobless", None), false);
            let blobless = fixture.work().join("blobless");
            assert_eq!(
                git(&blobless, &["config", "remote.origin.promisor"]),
                "true"
            );
            let old_blob = git(&blobless, &["rev-parse", "HEAD~1:README"]);
            assert_eq!(git(&blobless, &["cat-file", "-p", &old_blob]), "old blob");
        },
    );
}
