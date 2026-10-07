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
use instant::Instant;
use tempfile::TempDir;
use warp_cli::agent::{RepositoryForge, RepositoryHeadRef, RepositoryIdentity};
use warp_cli::environment_checkout::EnvironmentCheckoutArgs;
use warp_core::features::FeatureFlag;

use super::super::environment_checkout_protocol::{
    CheckoutBatch, CheckoutFailureKind, CheckoutOutcome, CheckoutReport, CheckoutRequest,
};
use super::{
    CachedCheckout, Git, attempt_cached_checkout, capture, mirror_key, optional_mirror_root, run,
};

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
                report_file: directory.path().join("report.json"),
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
            let report: CheckoutReport =
                serde_json::from_slice(&fs::read(&args.report_file).unwrap()).unwrap();
            assert_eq!(
                report
                    .outcomes
                    .iter()
                    .map(|outcome| (outcome.request_index, outcome.failure))
                    .collect::<Vec<_>>(),
                vec![
                    (0, Some(CheckoutFailureKind::RemoveOrigin)),
                    (1, Some(CheckoutFailureKind::RemoveOrigin))
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
fn checkout_io_failures_include_operation_and_underlying_error() {
    let directory = TempDir::new().unwrap();
    let fixture = Fixture {
        root: directory.path().to_owned(),
    };
    let batch = fixture.batch(vec![fixture.request("missing-parent", None)]);
    let target = batch.working_dir.join("missing-parent");
    for remove_origins_only in [false, true] {
        let (expected_kind, context, error) = if remove_origins_only {
            (
                CheckoutFailureKind::RemoveOrigin,
                "could not inspect origin cleanup target",
                fs::symlink_metadata(&target).unwrap_err(),
            )
        } else {
            (
                CheckoutFailureKind::Clone,
                "could not create checkout target",
                fs::create_dir(&target).unwrap_err(),
            )
        };
        let results = block_on(Compat::new(super::checkout_batch(
            &batch,
            None,
            remove_origins_only,
        )))
        .unwrap();
        assert_eq!(results[0].failure, Some(expected_kind));
        assert!(results[0].diagnostics.contains(context));
        assert!(results[0].diagnostics.contains(&error.to_string()));
    }
}

#[test]
fn git_failures_include_operation_and_status_without_output() {
    fixture_test(
        "git_failures_include_operation_and_status_without_output",
        |fixture| {
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    fixture.root.join("gitconfig").to_str().unwrap(),
                    "alias.quiet-failure",
                    "!exit 23",
                ],
            );
            let mut git = Git::default();
            assert!(
                block_on(Compat::new(git.run(
                    "quiet-failure",
                    &fixture.work(),
                    &["quiet-failure"]
                )))
                .is_err()
            );
            assert!(git.diagnostics.contains("Git quiet-failure failed"));
            assert!(git.diagnostics.contains("23"));
        },
    );
}

#[test]
fn helper_report_times_every_checkout_and_omits_unresolved_heads() {
    fixture_test(
        "helper_report_times_every_checkout_and_omits_unresolved_heads",
        |fixture| {
            let empty = fixture.work().join("empty");
            fs::create_dir(&empty).unwrap();
            git(&empty, &["init", "--quiet"]);
            let directory = TempDir::new().unwrap();
            let mut args = EnvironmentCheckoutArgs {
                requests_file: directory.path().join("requests.json"),
                report_file: directory.path().join("report.json"),
                remove_origins_only: false,
            };
            let batch = fixture.batch(vec![
                fixture.request("empty", None),
                fixture.request(
                    "pinned",
                    Some(RepositoryHeadRef::CommitSha(fixture.pinned())),
                ),
                fixture.request("default", None),
            ]);
            fs::write(&args.requests_file, serde_json::to_vec(&batch).unwrap()).unwrap();
            run(&args).unwrap();
            let report: CheckoutReport =
                serde_json::from_slice(&fs::read(&args.report_file).unwrap()).unwrap();
            assert!(report.failures().next().is_none());
            assert_eq!(
                report
                    .outcomes
                    .iter()
                    .map(|outcome| outcome.resolved_head.clone())
                    .collect::<Vec<_>>(),
                vec![None, Some(fixture.pinned()), Some(fixture.base())]
            );
            // Timing is reported for every checkout, including the one with no resolvable HEAD.
            assert_eq!(report.outcomes.len(), 3);
            // An unwritable report path fails the run, since the report is the only result.
            args.report_file = directory.path().to_owned();
            assert!(run(&args).is_err());
        },
    );
}

#[test]
fn credential_identity_parser_rejects_malformed_or_ambiguous_usernames() {
    for output in [
        "username=https://token@github.com\n",
        "username=first\nusername=second\n",
        "password=fixture-secret\n",
    ] {
        assert_eq!(super::git_credential_username(output), None);
    }
    assert_eq!(
        super::sanitize_git_author_name("Ada Lovelace\n"),
        Some("Ada Lovelace".to_owned())
    );
    assert_eq!(
        super::sanitize_git_author_name("Ada\npassword=fixture-secret\n"),
        None
    );
}

#[test]
fn credential_identity_queries_discard_failed_output_and_time_out() {
    fixture_test(
        "credential_identity_queries_discard_failed_output_and_time_out",
        |fixture| {
            let config = fixture.root.join("gitconfig");
            let query = || {
                block_on(Compat::new(super::identity_query(
                    &fixture.work(),
                    &["credential", "fill"],
                    Some("protocol=https\nhost=github.com\n\n"),
                )))
            };
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    config.to_str().unwrap(),
                    "alias.identity-fixture",
                    "!f() { printf 'username=fixture\\npassword=fixture-only-secret-not-a-real-token\\n'; printf 'fixture-only-secret-not-a-real-token' >&2; exit 1; }; f",
                ],
            );
            assert!(
                block_on(Compat::new(super::identity_query(
                    &fixture.work(),
                    &["identity-fixture"],
                    None
                )))
                .is_none()
            );
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    config.to_str().unwrap(),
                    "credential.helper",
                    "!f() { sleep 10; }; f",
                ],
            );
            let started = Instant::now();
            assert!(query().is_none());
            assert!(started.elapsed() >= super::IDENTITY_QUERY_TIMEOUT);
            assert!(started.elapsed() < Duration::from_secs(5));
        },
    );
}

#[cfg(unix)]
#[test]
fn distinct_sources_run_concurrently_with_a_bounded_active_count() {
    fixture_test(
        "distinct_sources_run_concurrently_with_a_bounded_active_count",
        |fixture| {
            use std::os::unix::fs::PermissionsExt as _;
            // Every pack the origin serves runs this hook, which records when it starts and ends
            // so that overlapping transfers can be counted.
            let activity = fixture.root.join("activity");
            let hook = fixture.root.join("record-transfer");
            fs::write(
                &hook,
                format!(
                    "#!/bin/sh\necho start >> '{path}'\nsleep 1\necho end >> '{path}'\nexec \"$@\"\n",
                    path = activity.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    fixture.root.join("gitconfig").to_str().unwrap(),
                    "uploadpack.packObjectsHook",
                    hook.to_str().unwrap(),
                ],
            );
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
            let root = optional_mirror_root();
            let results = checkout_batch(&fixture.batch(requests), root.as_deref()).unwrap();
            assert_succeeded(&results);
            let mut active = 0;
            let mut maximum = 0;
            for line in fs::read_to_string(&activity).unwrap().lines() {
                match line {
                    "start" => {
                        active += 1;
                        maximum = maximum.max(active);
                    }
                    "end" => active -= 1,
                    other => panic!("unexpected activity line {other:?}"),
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
) -> anyhow::Result<Vec<CheckoutOutcome>> {
    block_on(Compat::new(super::checkout_batch(
        batch,
        mirror_root,
        false,
    )))
}

fn assert_succeeded(outcomes: &[CheckoutOutcome]) {
    assert!(
        outcomes.iter().all(|outcome| outcome.failure.is_none()),
        "{outcomes:?}"
    );
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
        let _mirrors = FeatureFlag::GitMirrorCache.override_enabled(true);
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
            let results = checkout_batch(
                &fixture.batch(vec![fixture.request("good-clone", None), bad]),
                None,
            )
            .unwrap();
            assert!(results[0].failure.is_none());
            assert_eq!(results[1].failure, Some(CheckoutFailureKind::Clone));
        },
    );
}

struct Fixture {
    root: PathBuf,
}

/// Whether the checkout at `target` is a partial (blobless) clone. The network path makes one and
/// the mirror path does not, which tells the two apart.
fn is_partial_clone(target: &Path) -> bool {
    fs::read_to_string(target.join(".git/config"))
        .unwrap()
        .contains("promisor")
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
    fn prepare(&self, request: CheckoutRequest, cached: bool) {
        let root = cached.then(optional_mirror_root).flatten();
        let results = checkout_batch(&self.batch(vec![request]), root.as_deref()).unwrap();
        assert_succeeded(&results);
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

/// Runs `test` against a fixture. The test body executes in a re-run of the test process that
/// is isolated from the user's git configuration; the outer invocation only sets that up.
fn fixture_test(name: &str, test: impl FnOnce(&Fixture)) {
    if let Some(root) = std::env::var_os("WARP_TEST_CHECKOUT_ROOT") {
        let _mirrors = FeatureFlag::GitMirrorCache.override_enabled(true);
        test(&Fixture { root: root.into() });
        return;
    }
    let tmp = TempDir::new().unwrap();
    run_isolated(&create_fixture(tmp.path()), name);
}

/// Creates the origin repository, its seed history, and the git config that maps the canonical
/// clone URL to the origin.
fn create_fixture(root: &Path) -> Fixture {
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
    git(&fixture.seed(), &["config", "commit.gpgsign", "false"]);
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
    fixture
}

/// Builds the command that reruns test `name` in a fresh process whose git configuration and
/// mirror cache point at this fixture.
fn isolated_test_command(fixture: &Fixture, name: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    #[cfg(windows)]
    command.creation_flags(0);
    command
        .args([
            "--exact",
            &format!("ai::agent_sdk::driver::environment_checkout::tests::{name}"),
            "--nocapture",
        ])
        .env("WARP_TEST_CHECKOUT_ROOT", &fixture.root)
        .env("GIT_CONFIG_GLOBAL", fixture.root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("WARP_ISOLATION_PLATFORM", "namespace")
        .env("WARP_BUILD_CACHE_ROOT", fixture.root.join("cache"))
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    command
}

fn run_isolated(fixture: &Fixture, name: &str) {
    let output = isolated_test_command(fixture, name).output().unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("fixture-only-secret-not-a-real-token")
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("fixture-only-secret-not-a-real-token")
    );
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
            fixture.prepare(fixture.request("cold", None), true);
            let cold = fixture.work().join("cold");
            assert!(!is_partial_clone(&cold));
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
            fixture.prepare(fixture.request("warm", None), true);
            let warm = fixture.work().join("warm");
            assert!(!is_partial_clone(&warm));
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
fn mirror_tracks_all_refs_and_cached_checkout_survives_commit_graphs() {
    fixture_test(
        "mirror_tracks_all_refs_and_cached_checkout_survives_commit_graphs",
        |fixture| {
            fixture.prepare(fixture.request("cold", None), true);
            let mirror = fixture.mirror();
            assert_eq!(
                git(&mirror, &["config", "--local", "remote.origin.fetch"]),
                "+refs/*:refs/*"
            );
            assert_eq!(
                git(&mirror, &["config", "--local", "remote.origin.mirror"]),
                "true"
            );
            assert_eq!(
                git(&mirror, &["rev-parse", "refs/pinned/base"]),
                fixture.pinned()
            );
            git(
                &mirror,
                &["commit-graph", "write", "--reachable", "--split"],
            );
            fixture.prepare(fixture.request("warm", None), true);
            let warm = fixture.work().join("warm");
            assert!(!is_partial_clone(&warm));
            assert_eq!(git(&warm, &["rev-parse", "HEAD"]), fixture.base());
            assert_eq!(git(&warm, &["branch", "--show-current"]), "main");
            assert_eq!(
                git(&warm, &["symbolic-ref", "refs/remotes/origin/HEAD"]),
                "refs/remotes/origin/main"
            );
            fixture.full_objects(&warm);
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
            fixture.prepare(fixture.request("partial", None), true);
            assert_eq!(
                git(&fixture.mirror(), &["rev-parse", "--is-bare-repository"]),
                "true"
            );
            git(
                &fixture.mirror(),
                &["config", "remote.origin.promisor", "true"],
            );
            fixture.prepare(fixture.request("filtered", None), true);
            assert!(
                !fs::read_to_string(fixture.mirror().join("config"))
                    .unwrap()
                    .contains("promisor")
            );
            assert!(!is_partial_clone(&fixture.work().join("filtered")));
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
            fixture.prepare(fixture.request("credential", None), true);
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
            fixture.prepare(fixture.request("fallback", None), true);
            let target = fixture.work().join("fallback");
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), pinned);
            assert!(is_partial_clone(&target));
        },
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn failed_cached_attempt_cleans_only_its_new_checkout() {
    fixture_test(
        "failed_cached_attempt_cleans_only_its_new_checkout",
        |fixture| {
            git(
                &fixture.origin(),
                &["update-ref", "refs/tags/v1", &fixture.pinned()],
            );
            // Only the network path resolves a branch head that names a tag, so the cached attempt
            // fails after it has created and populated its checkout.
            fixture.prepare(
                fixture.request(
                    "recovered",
                    Some(RepositoryHeadRef::Branch("v1".to_owned())),
                ),
                true,
            );
            let recovered = fixture.work().join("recovered");
            assert_eq!(git(&recovered, &["rev-parse", "HEAD"]), fixture.pinned());
            assert!(is_partial_clone(&recovered));

            let request = fixture.request("raced", None);
            let target = fixture.work().join("raced");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("KEEP"), "pre-existing target").unwrap();
            let outcome = block_on(Compat::new(attempt_cached_checkout(
                &request,
                CANONICAL_URL,
                &target,
                &fixture.mirror(),
                &mut Git::default(),
            )))
            .unwrap();
            assert_eq!(outcome, CachedCheckout::Failed);
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
                let results = checkout_batch(
                    &fixture.batch(vec![request.clone(), second, bad]),
                    root.as_deref(),
                )
                .unwrap();
                assert_succeeded(&results[..2]);
                assert_eq!(results[2].failure, Some(CheckoutFailureKind::Checkout));
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
            let results = checkout_batch(
                &fixture.batch(vec![
                    fixture.request("sha", Some(RepositoryHeadRef::Branch("missing".to_owned()))),
                ]),
                None,
            )
            .unwrap();
            assert_eq!(results[0].failure, Some(CheckoutFailureKind::Checkout));
            assert_eq!(git(&target, &["rev-parse", "HEAD"]), base);
        },
    );
}

#[test]
fn helper_writes_typed_failures_and_rejects_invalid_requests_before_work() {
    fixture_test(
        "helper_writes_typed_failures_and_rejects_invalid_requests_before_work",
        |fixture| {
            let credentials = fixture.root.join("identity-credentials");
            fs::write(
                &credentials,
                "https://fixture:fixture-only-secret-not-a-real-token@github.com\n",
            )
            .unwrap();
            git(
                &fixture.root,
                &[
                    "config",
                    "--file",
                    fixture.root.join("gitconfig").to_str().unwrap(),
                    "credential.helper",
                    &format!(
                        "store --file=\"{}\"",
                        credentials.to_str().unwrap().replace('\\', "/")
                    ),
                ],
            );
            let directory = TempDir::new().unwrap();
            let args = EnvironmentCheckoutArgs {
                requests_file: directory.path().join("requests.json"),
                report_file: directory.path().join("report.json"),
                remove_origins_only: false,
            };
            let batch = fixture.batch(vec![
                fixture.request("good", None),
                fixture.request("bad", Some(RepositoryHeadRef::Branch("missing".to_owned()))),
                fixture.request(
                    "another-bad",
                    Some(RepositoryHeadRef::Branch("missing".to_owned())),
                ),
            ]);
            fs::write(&args.requests_file, serde_json::to_vec(&batch).unwrap()).unwrap();
            assert!(run(&args).is_err());
            let report: CheckoutReport =
                serde_json::from_slice(&fs::read(&args.report_file).unwrap()).unwrap();
            assert_eq!(report.outcomes.len(), 3);
            // A failed batch reports no heads, since nothing downstream should trust them.
            assert!(
                report
                    .outcomes
                    .iter()
                    .all(|outcome| outcome.resolved_head.is_none())
            );
            let failures = report.failures().collect::<Vec<_>>();
            assert_eq!(failures.len(), 2);
            assert_eq!(failures[0].request_index, 1);
            assert_eq!(failures[0].failure, Some(CheckoutFailureKind::Checkout));
            assert_eq!(failures[1].request_index, 2);
            let identity = report.identity_diagnostics.as_ref().unwrap();
            assert_eq!(identity.credentials.len(), 1);
            assert_eq!(identity.credentials[0].host, "github.com");
            assert_eq!(identity.credentials[0].username.as_deref(), Some("fixture"));
            assert!(
                !serde_json::to_string(&report)
                    .unwrap()
                    .contains("fixture-only-secret-not-a-real-token")
            );
            fs::write(&args.requests_file, b"{").unwrap();
            let error = run(&args).unwrap_err().to_string();
            assert!(error.contains("invalid checkout JSON"));
            assert!(error.contains("EOF while parsing an object at line 1 column 1"));
            let mut invalid = serde_json::to_value(&batch).unwrap();
            let sentinel = "fixture-only-secret-not-a-real-token";
            invalid["repositories"][0]["source"]["code_forge"] = serde_json::json!(format!(
                "https://fixture:{sentinel}@example.com/{}?token=AKIAIOSFODNN7EXAMPLE",
                "x".repeat(8192)
            ));
            fs::write(&args.requests_file, serde_json::to_vec(&invalid).unwrap()).unwrap();
            let error = run(&args).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("unknown variant"));
            assert!(message.contains("line 1 column"));
            assert!(message.len() <= 4096);
            let diagnostic = format!("{error:?}");
            assert!(!diagnostic.contains(sentinel));
            assert!(!diagnostic.contains("AKIAIOSFODNN7EXAMPLE"));
        },
    );
}

#[cfg(unix)]
#[test]
fn top_level_mirror_symlinks_are_rebuilt_without_following_targets() {
    fixture_test(
        "top_level_mirror_symlinks_are_rebuilt_without_following_targets",
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
            fs::remove_dir_all(fixture.root.join("cache/git-mirrors")).unwrap();
            symlink(&outside, fixture.root.join("cache/git-mirrors")).unwrap();
            assert!(optional_mirror_root().is_none());
        },
    );
}

#[test]
fn output_capture_retains_the_failure_tail_without_partial_secret_lines() {
    let text = format!(
        "{}://user:fixture-only-secret-not-a-real-token@example.com\nfatal: repository not found",
        "x".repeat(128 * 1024)
    );
    let mut reader = text.as_bytes();
    let (captured, truncated) = block_on(capture(&mut reader)).unwrap();
    assert!(truncated);
    assert!(reader.is_empty());
    assert_eq!(captured, "fatal: repository not found");
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

#[test]
fn inherited_credentials_authenticate_without_leaking() {
    fixture_test(
        "inherited_credentials_authenticate_without_leaking",
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
            let cached = cfg!(any(target_os = "linux", target_os = "macos"));
            let root = cached.then(optional_mirror_root).flatten();
            let results = checkout_batch(&batch, root.as_deref()).unwrap();
            assert_succeeded(&results);
            assert!(remote.received.load(Ordering::SeqCst));
            let target = fixture.work().join("authenticated");
            for value in [
                serde_json::to_string(&batch).unwrap(),
                serde_json::to_string(&CheckoutReport::default()).unwrap(),
                fs::read_to_string(target.join(".git/config")).unwrap(),
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
