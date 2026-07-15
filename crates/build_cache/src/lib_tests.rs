use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use command::r#async::Command;
use futures::executor::block_on;
#[cfg(unix)]
use instant::Instant;
use warp_errors::ErrorExt as _;

use super::{
    CacheScope, CacheSetupError, Detection, RepoCacheKey, RepoIdentity, RepositoryCacheSource,
    aggregate_mode_stats, build_export_script, construct_plan, create_retained_scratch_directory,
    is_valid_env_name, posix_single_quote, run_command_with_timeout, setup_cache,
};
use crate::spacectl::{Mount, MountInput, MountOutput, MountResponse};

fn identity(host: &str, owner: &str, repo: &str) -> RepoIdentity {
    RepoIdentity::new(host, owner, repo)
}

fn source(root: &Path, host: &str, owner: &str, repo: &str) -> RepositoryCacheSource {
    let cwd = root.join(repo);
    fs::create_dir_all(&cwd).unwrap();
    RepositoryCacheSource {
        name: format!("{owner}/{repo}"),
        identity: identity(host, owner, repo),
        cwd,
    }
}

fn detection(source: RepositoryCacheSource, modes: &[&str]) -> Detection {
    Detection {
        key: RepoCacheKey::derive(&source.identity),
        source,
        modes: modes.iter().map(ToString::to_string).collect(),
    }
}

fn response(modes: &[&str], envs: &[(&str, &str)], mounts: &[(&str, bool)]) -> Vec<u8> {
    let modes = modes
        .iter()
        .map(|mode| format!(r#""{mode}""#))
        .collect::<Vec<_>>()
        .join(",");
    let envs = envs
        .iter()
        .map(|(name, value)| format!(r#""{name}":"{value}""#))
        .collect::<Vec<_>>()
        .join(",");
    let mounts = mounts
        .iter()
        .map(|(mode, hit)| format!(r#"{{"mode":"{mode}","cache_hit":{hit}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"input":{{"modes":[{modes}]}},"output":{{"add_envs":{{{envs}}},"mounts":[{mounts}]}}}}"#
    )
    .into_bytes()
}

fn command_args(command: &Command) -> Vec<OsString> {
    command.get_args().map(ToOwned::to_owned).collect()
}

fn is_detect(command: &Command) -> bool {
    command_args(command)
        .iter()
        .any(|arg| arg == "--dry_run=true")
}

fn is_global(command: &Command) -> bool {
    command_args(command)
        .iter()
        .any(|arg| Path::new(arg).ends_with("shared"))
}

#[test]
fn repo_cache_key_is_stable_for_canonical_identity() {
    let first = RepoCacheKey::derive(&identity(" GitHub.COM ", "WarpDev", "Warp"));
    let second = RepoCacheKey::derive(&identity("github.com", "warpdev", "warp"));
    assert_eq!(first, second);
    assert_eq!(first.as_str().len(), 64);
}

#[test]
fn repo_cache_key_distinguishes_forge_owner_and_repo() {
    let base = RepoCacheKey::derive(&identity("github.com", "warp", "client"));
    assert_ne!(
        base,
        RepoCacheKey::derive(&identity("gitlab.com", "warp", "client"))
    );
    assert_ne!(
        base,
        RepoCacheKey::derive(&identity("github.com", "other", "client"))
    );
    assert_ne!(
        base,
        RepoCacheKey::derive(&identity("github.com", "warp", "server"))
    );
}

#[test]
fn repo_cache_key_rejects_non_lowercase_64_hex() {
    assert!(RepoCacheKey::try_from("a".repeat(63)).is_err());
    assert!(RepoCacheKey::try_from("A".repeat(64)).is_err());
    assert!(RepoCacheKey::try_from("g".repeat(64)).is_err());
    assert!(RepoCacheKey::try_from("0".repeat(64)).is_ok());
}

#[test]
fn plan_orders_repository_keys_and_places_single_global_last() {
    let temp = tempfile::tempdir().unwrap();
    let left = source(temp.path(), "github.com", "z", "left");
    let right = source(temp.path(), "github.com", "a", "right");
    let plan = construct_plan(
        temp.path().join("cache"),
        vec![detection(left, &["go"]), detection(right, &["cargo"])],
        Vec::new(),
    )
    .unwrap()
    .unwrap();
    plan.validate().unwrap();
    assert!(matches!(
        plan.configurations.last().unwrap().scope,
        CacheScope::Global
    ));
    let keys = plan.configurations[..2]
        .iter()
        .map(|configuration| configuration.scope.repo_key().unwrap())
        .collect::<Vec<_>>();
    assert!(keys[0] <= keys[1]);
    assert_eq!(
        plan.configurations
            .iter()
            .filter(|configuration| matches!(configuration.scope, CacheScope::Global))
            .count(),
        1
    );
}

#[test]
fn plan_uses_only_relative_repo_and_shared_cache_directories() {
    let temp = tempfile::tempdir().unwrap();
    let repo = source(temp.path(), "github.com", "warp", "client");
    let plan = construct_plan(
        temp.path().join("cache"),
        vec![detection(repo, &["cargo"])],
        Vec::new(),
    )
    .unwrap()
    .unwrap();
    assert!(
        plan.configurations[0]
            .relative_cache_dir
            .starts_with("repos")
    );
    assert_eq!(
        plan.configurations.last().unwrap().relative_cache_dir,
        Path::new("shared")
    );
    assert!(
        plan.configurations
            .iter()
            .all(|configuration| !configuration.relative_cache_dir.is_absolute())
    );
}

#[test]
fn plan_sorts_and_deduplicates_configuration_modes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = source(temp.path(), "github.com", "warp", "client");
    let plan = construct_plan(
        temp.path().join("cache"),
        vec![detection(repo, &["cargo", "go"])],
        vec!["apt".to_owned()],
    )
    .unwrap()
    .unwrap();
    assert_eq!(plan.configurations[0].modes, ["cargo", "go"]);
    assert_eq!(
        plan.configurations.last().unwrap().modes,
        ["apt", "cargo", "go"]
    );
}

#[test]
fn global_modes_are_union_of_successful_repo_detections() {
    let temp = tempfile::tempdir().unwrap();
    let one = source(temp.path(), "github.com", "warp", "one");
    let two = source(temp.path(), "github.com", "warp", "two");
    let plan = construct_plan(
        temp.path().join("cache"),
        vec![
            detection(one, &["go", "cargo"]),
            detection(two, &["cargo", "npm"]),
        ],
        Vec::new(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        plan.configurations.last().unwrap().modes,
        ["cargo", "go", "npm"]
    );
}

#[test]
fn plan_adds_supplied_apt_global_mode() {
    let temp = tempfile::tempdir().unwrap();
    let with_apt = construct_plan(
        temp.path().join("cache-a"),
        Vec::new(),
        vec!["apt".to_owned()],
    )
    .unwrap()
    .unwrap();
    assert_eq!(with_apt.configurations.last().unwrap().modes, ["apt"]);

    let without_apt = construct_plan(temp.path().join("cache-b"), Vec::new(), Vec::new()).unwrap();
    assert!(without_apt.is_none());
}

#[test]
fn plan_adds_supplied_brew_global_mode() {
    let temp = tempfile::tempdir().unwrap();
    let with_brew = construct_plan(
        temp.path().join("cache-a"),
        Vec::new(),
        vec!["brew".to_owned()],
    )
    .unwrap()
    .unwrap();
    assert_eq!(with_brew.configurations.last().unwrap().modes, ["brew"]);

    let without_brew = construct_plan(temp.path().join("cache-b"), Vec::new(), Vec::new()).unwrap();
    assert!(without_brew.is_none());
}

#[test]
fn empty_mode_union_produces_no_executable_plan() {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        construct_plan(temp.path().join("cache"), Vec::new(), Vec::new(),)
            .unwrap()
            .is_none()
    );
}

#[test]
fn json_parse_failure_is_classified_and_does_not_abort_later_repos() {
    let temp = tempfile::tempdir().unwrap();
    let repositories = vec![
        source(temp.path(), "github.com", "warp", "one"),
        source(temp.path(), "github.com", "warp", "two"),
    ];
    let calls = Rc::new(RefCell::new(0usize));
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        repositories,
        Vec::new(),
        {
            let calls = Rc::clone(&calls);
            move |_| {
                let call = {
                    let mut count = calls.borrow_mut();
                    *count += 1;
                    *count
                };
                futures::future::ready(Ok(if call == 1 {
                    b"not-json".to_vec()
                } else {
                    response(&["cargo"], &[], &[])
                }))
            }
        },
    ));
    assert_eq!(*calls.borrow(), 4);
    assert_eq!(
        report.invocations[0].error,
        Some(CacheSetupError::JsonParseFailed)
    );
    assert!(report.plan.is_some());
}

#[test]
fn destructive_execution_uses_resolved_modes_without_redetection() {
    let temp = tempfile::tempdir().unwrap();
    let commands = Rc::new(RefCell::new(Vec::new()));
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        vec![source(temp.path(), "github.com", "warp", "client")],
        Vec::new(),
        {
            let commands = Rc::clone(&commands);
            move |command| {
                let detect = is_detect(&command);
                commands.borrow_mut().push(command_args(&command));
                futures::future::ready(Ok(if detect {
                    response(&["go", "cargo", "go"], &[], &[])
                } else {
                    response(&["cargo", "go"], &[], &[])
                }))
            }
        },
    ));
    assert!(report.plan.is_some());
    let commands = commands.borrow();
    assert_eq!(commands.len(), 3);
    for args in &commands[1..] {
        assert!(args.iter().any(|arg| arg == "--mode=cargo,go"));
        assert!(!args.iter().any(|arg| arg == "--detect=*"));
        assert!(args.iter().any(|arg| arg == "--dry_run=false"));
    }
}

#[test]
fn repo_failure_continues_and_global_still_executes() {
    let temp = tempfile::tempdir().unwrap();
    let destructive_calls = Rc::new(RefCell::new(0));
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        vec![
            source(temp.path(), "github.com", "warp", "one"),
            source(temp.path(), "github.com", "warp", "two"),
        ],
        Vec::new(),
        {
            let destructive_calls = Rc::clone(&destructive_calls);
            move |command| {
                if is_detect(&command) {
                    return futures::future::ready(Ok(response(&["cargo"], &[], &[])));
                }
                let mut calls = destructive_calls.borrow_mut();
                *calls += 1;
                if *calls == 1 {
                    futures::future::ready(Err(CacheSetupError::NonzeroExit { exit_code: Some(1) }))
                } else {
                    futures::future::ready(Ok(response(&["cargo"], &[], &[])))
                }
            }
        },
    ));
    assert_eq!(*destructive_calls.borrow(), 3);
    assert!(
        report
            .invocations
            .last()
            .is_some_and(|invocation| matches!(invocation.scope, CacheScope::Global))
    );
}

#[test]
fn spacectl_calls_are_bounded_by_two_repos_plus_one_global() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Rc::new(RefCell::new(0));
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        vec![
            source(temp.path(), "github.com", "warp", "one"),
            source(temp.path(), "github.com", "warp", "two"),
        ],
        Vec::new(),
        {
            let calls = Rc::clone(&calls);
            move |_| {
                *calls.borrow_mut() += 1;
                futures::future::ready(Ok(response(&["cargo"], &[], &[])))
            }
        },
    ));
    assert_eq!(*calls.borrow(), 5);
    assert_eq!(report.invocations.len(), 5);
}

#[test]
fn shared_success_replaces_complete_repo_env_overlay() {
    let temp = tempfile::tempdir().unwrap();
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        vec![source(temp.path(), "github.com", "warp", "client")],
        Vec::new(),
        move |command| {
            futures::future::ready(Ok(if is_detect(&command) {
                response(&["cargo"], &[], &[])
            } else if is_global(&command) {
                response(&["cargo"], &[("GLOBAL", "yes")], &[])
            } else {
                response(&["cargo"], &[("REPO", "yes")], &[])
            }))
        },
    ));
    assert_eq!(
        report.add_envs,
        BTreeMap::from([("GLOBAL".to_owned(), "yes".to_owned())])
    );
}

#[test]
fn shared_failure_keeps_canonical_repo_env_overlay() {
    let temp = tempfile::tempdir().unwrap();
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        vec![source(temp.path(), "github.com", "warp", "client")],
        Vec::new(),
        move |command| {
            futures::future::ready(if is_detect(&command) {
                Ok(response(&["cargo"], &[], &[]))
            } else if is_global(&command) {
                Err(CacheSetupError::Timeout)
            } else {
                Ok(response(&["cargo"], &[("REPO", "yes")], &[]))
            })
        },
    ));
    assert_eq!(
        report.add_envs,
        BTreeMap::from([("REPO".to_owned(), "yes".to_owned())])
    );
}

#[test]
fn repo_env_conflict_resolves_by_key_order() {
    let temp = tempfile::tempdir().unwrap();
    let repositories = vec![
        source(temp.path(), "github.com", "warp", "one"),
        source(temp.path(), "github.com", "warp", "two"),
    ];
    let keys = repositories
        .iter()
        .map(|repo| (RepoCacheKey::derive(&repo.identity), repo.cwd.clone()))
        .collect::<BTreeMap<_, _>>();
    let winning_cwd = keys.last_key_value().unwrap().1.clone();
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        repositories,
        Vec::new(),
        move |command| {
            let value = if command.get_current_dir() == Some(winning_cwd.as_path()) {
                "winner"
            } else {
                "earlier"
            };
            futures::future::ready(if is_detect(&command) {
                Ok(response(&["cargo"], &[], &[]))
            } else if is_global(&command) {
                Err(CacheSetupError::Timeout)
            } else {
                Ok(response(&["cargo"], &[("VALUE", value)], &[]))
            })
        },
    ));
    assert_eq!(report.add_envs["VALUE"], "winner");
}

#[test]
fn invalid_env_names_are_rejected_individually() {
    assert!(is_valid_env_name("VALID_1"));
    assert!(is_valid_env_name("_ALSO_VALID"));
    assert!(!is_valid_env_name(""));
    assert!(!is_valid_env_name("1INVALID"));
    assert!(!is_valid_env_name("BAD-NAME"));
    let script = build_export_script(&BTreeMap::from([
        ("BAD-NAME".to_owned(), "ignored".to_owned()),
        ("GOOD".to_owned(), "kept".to_owned()),
    ]));
    assert_eq!(script.as_deref(), Some("export GOOD='kept'"));
}

#[test]
fn hostile_env_values_are_posix_single_quote_escaped() {
    assert_eq!(
        posix_single_quote("$(touch /tmp/pwned);'quoted'"),
        "'$(touch /tmp/pwned);'\\''quoted'\\'''"
    );
}

#[test]
fn exports_are_sorted_and_built_once() {
    let script = build_export_script(&BTreeMap::from([
        ("Z_VAR".to_owned(), "z".to_owned()),
        ("A_VAR".to_owned(), "a".to_owned()),
    ]))
    .unwrap();
    assert_eq!(script, "export A_VAR='a'; export Z_VAR='z'");
    assert_eq!(script.matches("export ").count(), 2);
}

#[test]
fn hit_miss_aggregation_retains_zero_mount_modes() {
    let response = MountResponse {
        input: MountInput::default(),
        output: MountOutput {
            mounts: vec![
                Mount {
                    mode: "cargo".to_owned(),
                    cache_hit: true,
                },
                Mount {
                    mode: "cargo".to_owned(),
                    cache_hit: false,
                },
                Mount {
                    mode: "unknown".to_owned(),
                    cache_hit: true,
                },
            ],
            ..MountOutput::default()
        },
    };
    let stats = aggregate_mode_stats(&["cargo".to_owned(), "go".to_owned()], &response);
    assert_eq!(stats["cargo"].cache_hits, 1);
    assert_eq!(stats["cargo"].cache_misses, 1);
    assert_eq!(stats["go"].cache_hits, 0);
    assert_eq!(stats["go"].cache_misses, 0);
    assert_eq!(stats["unknown"].cache_hits, 1);
}

#[test]
fn scratch_directories_are_unique_0700_outside_repo_and_retained() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    let first = create_retained_scratch_directory([repo.as_path()].into_iter()).unwrap();
    let second = create_retained_scratch_directory([repo.as_path()].into_iter()).unwrap();
    assert_ne!(first, second);
    assert!(!first.starts_with(&repo));
    assert!(first.exists());
    assert!(second.exists());
    assert!(
        first
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("warp-spacectl-")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&first).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn process_runner_classifies_spawn_nonzero_and_timeout() {
    let missing = block_on(run_command_with_timeout(
        Command::new_with_process_group("/definitely/missing/spacectl"),
        Duration::from_millis(50),
    ));
    assert_eq!(missing, Err(CacheSetupError::SpawnFailed));

    let mut nonzero = Command::new_with_process_group("sh");
    nonzero.args(["-c", "exit 17"]);
    assert_eq!(
        block_on(run_command_with_timeout(nonzero, Duration::from_secs(1))),
        Err(CacheSetupError::NonzeroExit {
            exit_code: Some(17)
        })
    );

    let mut timeout = Command::new_with_process_group("sh");
    timeout.args(["-c", "sleep 1"]);
    assert_eq!(
        block_on(run_command_with_timeout(timeout, Duration::from_millis(10))),
        Err(CacheSetupError::Timeout)
    );
}

#[cfg(unix)]
#[test]
fn timeout_returns_bounded_when_descendant_keeps_stdout_open() {
    let mut command = Command::new_with_process_group("sh");
    command.args(["-c", "sleep 5 &"]);
    let started = Instant::now();
    let result = block_on(run_command_with_timeout(
        command,
        Duration::from_millis(100),
    ));

    assert_eq!(result, Err(CacheSetupError::Timeout));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn cache_setup_error_variants_have_expected_is_actionable_classification() {
    assert!(!CacheSetupError::RootCreationFailed.is_actionable());
    assert!(!CacheSetupError::SpawnFailed.is_actionable());
    assert!(!CacheSetupError::NonzeroExit { exit_code: Some(1) }.is_actionable());
    assert!(!CacheSetupError::Timeout.is_actionable());
    assert!(CacheSetupError::JsonParseFailed.is_actionable());
    assert!(CacheSetupError::EnvExportFailed.is_actionable());
}

#[test]
fn failure_categories_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    for error in [
        CacheSetupError::SpawnFailed,
        CacheSetupError::NonzeroExit {
            exit_code: Some(17),
        },
        CacheSetupError::Timeout,
    ] {
        let report = block_on(setup_cache(
            temp.path().join(format!("cache-{}", error.kind())),
            vec![source(
                temp.path(),
                "github.com",
                "warp",
                &format!("repo-{}", error.kind()),
            )],
            Vec::new(),
            {
                let error = error.clone();
                move |_| futures::future::ready(Err(error.clone()))
            },
        ));
        assert_eq!(report.invocations[0].error.as_ref(), Some(&error));
    }

    let cache_root_file = temp.path().join("cache-file");
    fs::write(&cache_root_file, "not a directory").unwrap();
    let report = block_on(setup_cache(
        cache_root_file,
        vec![source(temp.path(), "github.com", "warp", "root-failure")],
        Vec::new(),
        |_| futures::future::ready(Ok(Vec::new())),
    ));
    assert_eq!(
        report.invocations[0].error,
        Some(CacheSetupError::RootCreationFailed)
    );
}

#[test]
fn queued_executor_can_return_each_failure_category() {
    let temp = tempfile::tempdir().unwrap();
    let queue = Rc::new(RefCell::new(VecDeque::from([
        Err(CacheSetupError::JsonParseFailed),
        Err(CacheSetupError::Timeout),
    ])));
    let report = block_on(setup_cache(
        temp.path().join("cache"),
        vec![
            source(temp.path(), "github.com", "warp", "one"),
            source(temp.path(), "github.com", "warp", "two"),
        ],
        Vec::new(),
        {
            let queue = Rc::clone(&queue);
            move |_| futures::future::ready(queue.borrow_mut().pop_front().unwrap())
        },
    ));
    assert_eq!(report.invocations.len(), 2);
    assert_eq!(
        report.invocations[0].error,
        Some(CacheSetupError::JsonParseFailed)
    );
    assert_eq!(report.invocations[1].error, Some(CacheSetupError::Timeout));
}
