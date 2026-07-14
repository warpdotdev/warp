use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use build_cache::{
    build_export_script, CacheScope, CacheSetupError, CacheSetupReport, InvocationReport,
    ModeCacheStats,
};
use cloud_object_models::{CodeForge, SourceRepo};
use futures::executor::block_on;
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_isolation_platform::IsolationPlatformType;

use super::{
    apply_export_with, report_degradations_with, repository_cache_source, run_command_with_timeout,
    should_setup_cache,
};

fn command_output(status: CommandExitStatus) -> CommandOutput {
    CommandOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        status,
        exit_code: None,
    }
}

#[test]
fn gate_matrix_requires_namespace_and_nonempty_root() {
    let root = OsStr::new("/cache/build");
    assert!(should_setup_cache(
        Some(IsolationPlatformType::Namespace),
        Some(root)
    ));
    assert!(!should_setup_cache(None, Some(root)));
    assert!(!should_setup_cache(
        Some(IsolationPlatformType::Docker),
        Some(root)
    ));
    assert!(!should_setup_cache(
        Some(IsolationPlatformType::Namespace),
        None
    ));
    assert!(!should_setup_cache(
        Some(IsolationPlatformType::Namespace),
        Some(OsStr::new(""))
    ));
}

#[test]
fn source_repo_maps_to_canonical_identity_and_checkout() {
    let repo = SourceRepo::new(
        CodeForge::GitLab,
        "Platform/Backend".to_owned(),
        "API".to_owned(),
    );
    let mapped = repository_cache_source(&repo, Path::new("/work"));
    assert_eq!(mapped.name, "Platform/Backend/API");
    assert_eq!(mapped.identity.forge_host, "gitlab.com");
    assert_eq!(mapped.identity.owner, "platform/backend");
    assert_eq!(mapped.identity.repo, "api");
    assert_eq!(mapped.cwd, Path::new("/work/API"));
}

#[test]
fn process_runner_classifies_spawn_nonzero_and_timeout() {
    let missing = run_command_with_timeout(
        Command::new("/definitely/missing/spacectl"),
        Duration::from_millis(50),
    );
    assert_eq!(missing, Err(CacheSetupError::SpawnFailed));

    let mut nonzero = Command::new("sh");
    nonzero.args(["-c", "exit 17"]);
    assert_eq!(
        run_command_with_timeout(nonzero, Duration::from_secs(1)),
        Err(CacheSetupError::NonzeroExit {
            exit_code: Some(17)
        })
    );

    let mut timeout = Command::new("sh");
    timeout.args(["-c", "sleep 1"]);
    assert_eq!(
        run_command_with_timeout(timeout, Duration::from_millis(10)),
        Err(CacheSetupError::Timeout)
    );
}

#[test]
fn final_valid_export_is_sent_exactly_once_through_silent_executor() {
    let script = build_export_script(&BTreeMap::from([
        ("B".to_owned(), "two".to_owned()),
        ("A".to_owned(), "one".to_owned()),
    ]))
    .unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    block_on(apply_export_with(script, {
        let calls = Rc::clone(&calls);
        move |script| {
            calls.borrow_mut().push(script);
            futures::future::ready(Ok(command_output(CommandExitStatus::Success)))
        }
    }))
    .unwrap();
    assert_eq!(
        calls.borrow().as_slice(),
        ["export A='one'; export B='two'"]
    );
}

#[test]
fn invalid_or_empty_env_map_builds_no_export() {
    assert_eq!(build_export_script(&BTreeMap::new()), None);
    assert_eq!(
        build_export_script(&BTreeMap::from([(
            "INVALID-NAME".to_owned(),
            "value".to_owned()
        )])),
        None
    );
}

#[test]
fn failed_silent_export_is_classified() {
    let result = block_on(apply_export_with("export A='one'".to_owned(), |_| {
        futures::future::ready(Ok(command_output(CommandExitStatus::Failure)))
    }));
    assert_eq!(result, Err(CacheSetupError::EnvExportFailed));
}

#[test]
fn degradation_is_reported_exactly_once_at_sink() {
    let report = CacheSetupReport {
        invocations: vec![InvocationReport {
            scope: CacheScope::Global,
            modes: vec!["cargo".to_owned()],
            dry_run: false,
            relative_cache_dir: PathBuf::from("shared"),
            response: None,
            error: Some(CacheSetupError::Timeout),
            duration: Duration::from_secs(60),
            mode_stats: BTreeMap::<String, ModeCacheStats>::new(),
        }],
        ..CacheSetupReport::default()
    };
    let observed = Rc::new(RefCell::new(VecDeque::new()));
    report_degradations_with(&report, {
        let observed = Rc::clone(&observed);
        move |error, invocation| {
            observed
                .borrow_mut()
                .push_back((error.kind(), invocation.scope.kind()));
        }
    });
    assert_eq!(
        observed.borrow().iter().copied().collect::<Vec<_>>(),
        [("timeout", "global")]
    );
}
