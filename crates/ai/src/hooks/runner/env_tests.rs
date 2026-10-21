use std::ffi::OsString;
use std::path::PathBuf;

use super::*;

fn var(name: &str, value: &str) -> (OsString, OsString) {
    (OsString::from(name), OsString::from(value))
}

fn value_of<'a>(variables: &'a [(OsString, OsString)], name: &str) -> Option<&'a str> {
    variables
        .iter()
        .find(|(existing, _)| existing == name)
        .map(|(_, value)| value.to_str().expect("utf-8"))
}

fn names_of(variables: &[(OsString, OsString)]) -> Vec<&str> {
    variables
        .iter()
        .map(|(name, _)| name.to_str().expect("utf-8"))
        .collect()
}

fn runner() -> RunnerEnvironment {
    RunnerEnvironment {
        variables: vec![
            var("PATH", "/usr/bin:/bin"),
            var("HOME", "/Users/inherited"),
            var("TERM", "xterm-256color"),
            var("LANG", "en_US.UTF-8"),
        ],
        login_path: Some(OsString::from("/opt/homebrew/bin:/usr/bin:/bin")),
        home: Some(PathBuf::from("/Users/dev")),
        protocol_version: 1,
    }
}

fn cursor_hook() -> HookIdentity {
    HookIdentity {
        format: HookFormat::Cursor,
        project_dir: PathBuf::from("/work/repo"),
        warp_version: "v0.2026.01.01".to_owned(),
        run_id: None,
        user_email: None,
        remote: false,
    }
}

fn session(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn build(runner: RunnerEnvironment, hook: HookIdentity) -> Vec<(OsString, OsString)> {
    build_with_session(runner, hook, Vec::new())
}

fn build_with_session(
    runner: RunnerEnvironment,
    hook: HookIdentity,
    session_env: Vec<(String, String)>,
) -> Vec<(OsString, OsString)> {
    build_environment(EnvironmentInput {
        runner,
        hook,
        session_env,
    })
}

#[test]
fn the_identity_variables_and_cursor_aliases_are_set() {
    let variables = build(runner(), cursor_hook());
    assert_eq!(value_of(&variables, "WARP_PROJECT_DIR"), Some("/work/repo"));
    assert_eq!(value_of(&variables, "WARP_VERSION"), Some("v0.2026.01.01"));
    assert_eq!(
        value_of(&variables, "WARP_HOOK_PROTOCOL_VERSION"),
        Some("1")
    );
    assert_eq!(
        value_of(&variables, "CURSOR_PROJECT_DIR"),
        Some("/work/repo")
    );
    assert_eq!(
        value_of(&variables, "CURSOR_VERSION"),
        Some("v0.2026.01.01")
    );
    assert_eq!(
        value_of(&variables, "CLAUDE_PROJECT_DIR"),
        Some("/work/repo")
    );
    for absent in [
        "WARP_RUN_ID",
        "WARP_USER_EMAIL",
        "WARP_CODE_REMOTE",
        "CURSOR_USER_EMAIL",
        "CURSOR_CODE_REMOTE",
        "CURSOR_TRANSCRIPT_PATH",
    ] {
        assert_eq!(value_of(&variables, absent), None, "{absent}");
    }
}

#[test]
fn optional_identity_is_set_only_when_known() {
    let mut hook = cursor_hook();
    hook.run_id = Some("run-123".to_owned());
    hook.user_email = Some("dev@example.com".to_owned());
    hook.remote = true;
    let variables = build(runner(), hook);
    assert_eq!(value_of(&variables, "WARP_RUN_ID"), Some("run-123"));
    assert_eq!(
        value_of(&variables, "WARP_USER_EMAIL"),
        Some("dev@example.com")
    );
    assert_eq!(value_of(&variables, "WARP_CODE_REMOTE"), Some("true"));
    assert_eq!(
        value_of(&variables, "CURSOR_USER_EMAIL"),
        Some("dev@example.com")
    );
    assert_eq!(value_of(&variables, "CURSOR_CODE_REMOTE"), Some("true"));
    assert_eq!(value_of(&variables, "CURSOR_TRANSCRIPT_PATH"), None);
}

#[test]
fn the_host_environment_is_kept_except_for_the_scrub_list_and_prefixes() {
    let mut runner = runner();
    runner.variables.extend([
        var("WARP_API_KEY", "secret"),
        var("WARP_WORKLOAD_TOKEN", "secret"),
        var("WARP_CLOUD_AGENT_OTLP_TOKEN", "secret"),
        var(
            "WARP_CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT",
            "2026-10-21T00:00:00Z",
        ),
        var("WARP_IS_LOCAL_SHELL_SESSION", "1"),
        var("WARP_HONOR_PS1", "0"),
        var("WARP_TERMINAL_SESSION_UUID", "abc"),
        var("WARP_CLIENT_VERSION", "v0.2025.12.01"),
        var("SSH_SOCKET_DIR", "/tmp/sockets"),
        var("WARP_METADATA_TOKEN", "also-secret"),
        var("WARP_METADATA_", "prefix-only"),
        var("OZ_RUN_ID", "run-123"),
        var("WARP_SERVER_ROOT_URL", "https://example.test"),
    ]);
    let variables = build(runner, cursor_hook());
    for scrubbed in HOST_SCRUB_LIST {
        assert_eq!(value_of(&variables, scrubbed), None, "{scrubbed}");
    }
    // Named one by one, so a credential missing from the list fails here and not only in review.
    for credential in [
        "WARP_API_KEY",
        "WARP_WORKLOAD_TOKEN",
        "WARP_CLOUD_AGENT_OTLP_TOKEN",
        "WARP_CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT",
    ] {
        assert!(
            HOST_SCRUB_LIST.contains(&credential),
            "{credential} is not on the scrub list"
        );
        assert_eq!(value_of(&variables, credential), None, "{credential}");
    }
    assert_eq!(value_of(&variables, "WARP_METADATA_TOKEN"), None);
    assert_eq!(value_of(&variables, "WARP_METADATA_"), None);
    assert_eq!(value_of(&variables, "TERM"), Some("xterm-256color"));
    assert_eq!(value_of(&variables, "LANG"), Some("en_US.UTF-8"));
    assert_eq!(value_of(&variables, "OZ_RUN_ID"), Some("run-123"));
    assert_eq!(
        value_of(&variables, "WARP_SERVER_ROOT_URL"),
        Some("https://example.test")
    );
}

#[test]
fn a_protected_variable_inherited_from_the_host_is_replaced_or_removed() {
    let mut runner = runner();
    runner.variables.extend([
        var("WARP_RUN_ID", "an-outer-run"),
        var("WARP_PROJECT_DIR", "/somewhere/else"),
        var("CURSOR_TRANSCRIPT_PATH", "/tmp/transcript"),
        var("CLAUDE_PROJECT_DIR", "/stale"),
    ]);
    let variables = build(runner, cursor_hook());
    assert_eq!(value_of(&variables, "WARP_RUN_ID"), None);
    assert_eq!(value_of(&variables, "WARP_PROJECT_DIR"), Some("/work/repo"));
    assert_eq!(value_of(&variables, "CURSOR_TRANSCRIPT_PATH"), None);
    assert_eq!(
        value_of(&variables, "CLAUDE_PROJECT_DIR"),
        Some("/work/repo")
    );
}

#[test]
fn path_and_home_come_from_the_runner_when_it_has_them() {
    let variables = build(runner(), cursor_hook());
    assert_eq!(
        value_of(&variables, "PATH"),
        Some("/opt/homebrew/bin:/usr/bin:/bin")
    );
    assert_eq!(value_of(&variables, "HOME"), Some("/Users/dev"));

    let mut without = runner();
    without.login_path = None;
    without.home = None;
    let variables = build(without, cursor_hook());
    assert_eq!(value_of(&variables, "PATH"), Some("/usr/bin:/bin"));
    assert_eq!(value_of(&variables, "HOME"), Some("/Users/inherited"));
}

#[test]
fn the_hook_session_environment_is_applied_but_cannot_touch_protected_names() {
    let session_env = session(&[
        ("WARP_PROJECT_DIR", "/hijacked"),
        ("WARP_HOOK_PROTOCOL_VERSION", "99"),
        ("CURSOR_VERSION", "9.9.9"),
        ("MY_TOKEN", "from-session-start"),
        ("PATH", "/session/bin"),
        ("TERM", "dumb"),
    ]);
    let variables = build_with_session(runner(), cursor_hook(), session_env);
    assert_eq!(value_of(&variables, "WARP_PROJECT_DIR"), Some("/work/repo"));
    assert_eq!(
        value_of(&variables, "WARP_HOOK_PROTOCOL_VERSION"),
        Some("1")
    );
    assert_eq!(
        value_of(&variables, "CURSOR_VERSION"),
        Some("v0.2026.01.01")
    );
    assert_eq!(value_of(&variables, "MY_TOKEN"), Some("from-session-start"));
    // Session variables come after the host layer, so they may change PATH and the rest of it.
    assert_eq!(value_of(&variables, "PATH"), Some("/session/bin"));
    assert_eq!(value_of(&variables, "TERM"), Some("dumb"));
}

#[test]
fn a_session_variable_the_operating_system_would_reject_is_dropped() {
    let session_env = session(&[
        ("", "empty name"),
        ("A=B", "equals in name"),
        ("NUL\0NAME", "nul in name"),
        ("NUL_VALUE", "a\0b"),
        ("FINE", "kept"),
    ]);
    let variables = invocation_environment(&cursor_hook(), &session_env);
    let names = names_of(&variables);
    assert!(!names.contains(&""));
    assert!(!names.contains(&"A=B"));
    assert!(!names.iter().any(|name| name.starts_with("NUL")));
    assert_eq!(value_of(&variables, "FINE"), Some("kept"));
}

#[test]
fn every_name_appears_once() {
    let mut runner = runner();
    runner.variables.push(var("PATH", "/dup/bin"));
    let variables = build_with_session(
        runner,
        cursor_hook(),
        session(&[("TERM", "a"), ("TERM", "b")]),
    );
    let mut names = names_of(&variables);
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "{variables:?}");
    assert_eq!(value_of(&variables, "TERM"), Some("b"));
}

#[test]
fn the_two_layers_compose_to_the_whole_policy() {
    let runner = runner();
    let hook = cursor_hook();
    let session_env = session(&[("MY_TOKEN", "from-session-start")]);
    let mut composed = host_environment(&runner);
    let invocation = invocation_environment(&hook, &session_env);
    for (name, value) in invocation {
        composed.retain(|(existing, _)| *existing != name);
        composed.push((name, value));
    }
    let mut whole = build_with_session(runner, hook, session_env);
    composed.sort();
    whole.sort();
    assert_eq!(composed, whole);
}

#[test]
fn protected_names_cover_everything_the_policy_sets_itself() {
    let mut hook = cursor_hook();
    hook.run_id = Some("run".to_owned());
    hook.user_email = Some("dev@example.com".to_owned());
    hook.remote = true;
    let set_by_policy = invocation_environment(&hook, &[]);
    for (name, _) in set_by_policy {
        let name = name.to_str().expect("utf-8");
        assert!(PROTECTED_NAMES.contains(&name), "{name} is not protected");
    }
}

#[cfg(all(unix, feature = "local_fs"))]
mod subprocess {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use futures::executor::block_on;
    use tempfile::TempDir;

    use super::*;
    use crate::hooks::runner::command::CommandHookRunner;
    use crate::hooks::runner::{HookInvocation, HookRunner, ProcessExit};
    use crate::hooks::test_util::{test_binding_id, test_source_id};

    #[test]
    fn a_hook_sees_the_policy_applied_to_the_real_process_environment() {
        // SAFETY: nextest runs each test in its own process and this test spawns no other
        // threads before these calls, so nothing reads the environment concurrently.
        unsafe {
            std::env::set_var("WARP_API_KEY", "must-not-leak");
            std::env::set_var("WARP_WORKLOAD_TOKEN", "must-not-leak");
            std::env::set_var("WARP_CLOUD_AGENT_OTLP_TOKEN", "must-not-leak");
            std::env::set_var("WARP_METADATA_SECRET_TEST", "must-not-leak");
            std::env::set_var("WARP_IS_LOCAL_SHELL_SESSION", "1");
            std::env::set_var("WARP_RUN_ID", "an-outer-run");
            std::env::set_var("HOOK_ENV_TEST_KEEP", "inherited");
        }
        let dir = TempDir::new().expect("tempdir");
        let script = dir.path().join("env.sh");
        // The captured PATH below is not a real one, so `env` must be named by its full path.
        fs::write(&script, "#!/bin/sh\n/usr/bin/env\n").expect("write script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");

        let runner = CommandHookRunner::new(Arc::new(|| Some(OsString::from("/captured/bin"))))
            .with_home(Some(PathBuf::from("/custom/home")));
        let session_env = session(&[("FROM_SESSION", "yes"), ("WARP_PROJECT_DIR", "/hijacked")]);
        let outcome = block_on(runner.run(HookInvocation {
            binding: test_binding_id(test_source_id("hooks.json"), "sessionStart", 0),
            command: "./env.sh".to_owned(),
            cwd: dir.path().to_path_buf(),
            env: invocation_environment(&cursor_hook(), &session_env),
            stdin: b"{}".to_vec(),
            timeout: Duration::from_secs(10),
            cancel: Arc::new(AtomicBool::new(false)),
        }));
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        let dump = String::from_utf8(outcome.stdout).expect("utf-8");
        let seen = dump
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect::<Vec<_>>();

        assert_eq!(value_of(&seen, "WARP_API_KEY"), None);
        assert_eq!(value_of(&seen, "WARP_WORKLOAD_TOKEN"), None);
        assert_eq!(value_of(&seen, "WARP_CLOUD_AGENT_OTLP_TOKEN"), None);
        assert_eq!(value_of(&seen, "WARP_METADATA_SECRET_TEST"), None);
        assert_eq!(value_of(&seen, "WARP_IS_LOCAL_SHELL_SESSION"), None);
        assert_eq!(
            value_of(&seen, "WARP_RUN_ID"),
            None,
            "not inherited from an outer run"
        );
        assert_eq!(value_of(&seen, "HOOK_ENV_TEST_KEEP"), Some("inherited"));
        assert_eq!(value_of(&seen, "PATH"), Some("/captured/bin"));
        assert_eq!(value_of(&seen, "HOME"), Some("/custom/home"));
        assert_eq!(value_of(&seen, "WARP_HOOK_PROTOCOL_VERSION"), Some("1"));
        assert_eq!(value_of(&seen, "WARP_PROJECT_DIR"), Some("/work/repo"));
        assert_eq!(value_of(&seen, "WARP_VERSION"), Some("v0.2026.01.01"));
        assert_eq!(value_of(&seen, "CURSOR_PROJECT_DIR"), Some("/work/repo"));
        assert_eq!(value_of(&seen, "CURSOR_VERSION"), Some("v0.2026.01.01"));
        assert_eq!(value_of(&seen, "CLAUDE_PROJECT_DIR"), Some("/work/repo"));
        assert_eq!(value_of(&seen, "FROM_SESSION"), Some("yes"));
    }
}
