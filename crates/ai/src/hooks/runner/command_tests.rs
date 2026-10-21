#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::executor::block_on;
use instant::Instant;
use tempfile::TempDir;

use super::*;
use crate::hooks::test_util::{test_binding_id, test_source_id};

fn runner() -> CommandHookRunner {
    CommandHookRunner::new(Arc::new(|| None))
}

/// Writes an executable `#!/bin/sh` script and returns its path.
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn invocation(cwd: &Path, command: &str, stdin: &[u8], timeout: Duration) -> HookInvocation {
    HookInvocation {
        binding: test_binding_id(test_source_id("hooks.json"), "beforeShellExecution", 0),
        command: command.to_owned(),
        cwd: cwd.to_path_buf(),
        env: Vec::new(),
        stdin: stdin.to_vec(),
        timeout,
        cancel: Arc::new(AtomicBool::new(false)),
    }
}

fn run(runner: &CommandHookRunner, invocation: HookInvocation) -> HookProcessOutcome {
    block_on(runner.run(invocation))
}

fn process_is_alive(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // Signal 0 only checks that the pid exists.
    kill(Pid::from_raw(pid), None).is_ok()
}

fn wait_until_gone(pid: i32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !process_is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn the_payload_arrives_on_stdin_and_stdout_is_captured() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        r#"cat > "$PWD/received.json"; printf '{"permission":"allow"}'"#,
    );
    let outcome = run(
        &runner(),
        invocation(
            dir.path(),
            "./hook.sh",
            br#"{"command":"ls","cwd":"/tmp"}"#,
            Duration::from_secs(10),
        ),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(0));
    assert_eq!(outcome.stdout, br#"{"permission":"allow"}"#);
    assert_eq!(outcome.stderr_tail, "");
    assert!(outcome.duration > Duration::ZERO);
    let received = fs::read_to_string(dir.path().join("received.json")).expect("stdin was written");
    assert_eq!(received, r#"{"command":"ls","cwd":"/tmp"}"#);
}

#[test]
fn exit_codes_and_stderr_are_reported() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        "echo 'first line' >&2; echo 'second line' >&2; exit 3",
    );
    let outcome = run(
        &runner(),
        invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10)),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(3));
    assert!(outcome.stdout.is_empty());
    assert_eq!(outcome.stderr_tail, "first line\nsecond line\n");
}

#[test]
fn stderr_keeps_only_the_tail() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        "i=0; while [ $i -lt 1000 ]; do printf 'line %04d\\n' $i >&2; i=$((i+1)); done",
    );
    let outcome = run(
        &runner(),
        invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10)),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(0));
    assert!(outcome.stderr_tail.len() <= STDERR_TAIL_LIMIT);
    assert!(outcome.stderr_tail.ends_with("line 0999\n"));
}

#[test]
fn a_signalled_process_is_reported_by_signal() {
    let dir = TempDir::new().expect("tempdir");
    let outcome = run(
        &runner(),
        invocation(dir.path(), "kill -TERM $$", b"{}", Duration::from_secs(10)),
    );
    assert_eq!(outcome.exit, ProcessExit::Signal(15));
}

#[test]
fn the_command_runs_in_the_given_directory_with_the_expected_environment() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        r#"printf '%s\n%s\n%s\n%s\n' "$PWD" "$PATH" "$WARP_HOOK_PROTOCOL_VERSION" "$HOOK_TEST_VAR""#,
    );
    let runner = CommandHookRunner::new(Arc::new(|| Some(OsString::from("/custom/bin"))));
    let mut invocation = invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10));
    invocation.env = vec![(
        OsString::from("HOOK_TEST_VAR"),
        OsString::from("from-invocation"),
    )];
    let outcome = run(&runner, invocation);
    assert_eq!(outcome.exit, ProcessExit::Code(0));
    let lines = String::from_utf8(outcome.stdout).expect("utf-8");
    let lines = lines.lines().collect::<Vec<_>>();
    let canonical_dir = dir.path().canonicalize().expect("canonical tempdir");
    assert_eq!(
        Path::new(lines[0]).canonicalize().expect("cwd"),
        canonical_dir
    );
    assert_eq!(lines[1], "/custom/bin");
    assert_eq!(lines[2], HOOK_PROTOCOL_VERSION.to_string());
    assert_eq!(lines[3], "from-invocation");
}

#[test]
fn without_a_captured_path_the_process_path_is_inherited() {
    let dir = TempDir::new().expect("tempdir");
    write_script(dir.path(), "hook.sh", r#"printf '%s' "$PATH""#);
    let outcome = run(
        &runner(),
        invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10)),
    );
    let expected = std::env::var("PATH").expect("the test process has a PATH");
    assert_eq!(String::from_utf8(outcome.stdout).expect("utf-8"), expected);
}

#[test]
fn the_command_string_is_run_by_the_shell_as_written() {
    let dir = TempDir::new().expect("tempdir");
    let outcome = run(
        &runner(),
        invocation(
            dir.path(),
            "echo one && echo two | tr a-z A-Z",
            b"{}",
            Duration::from_secs(10),
        ),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(0));
    assert_eq!(outcome.stdout, b"one\nTWO\n");
}

#[test]
fn a_timeout_kills_the_whole_process_group() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        r#"sleep 30 &
echo "$$ $!" > "$PWD/pids"
wait"#,
    );
    let started = Instant::now();
    let outcome = run(
        &runner(),
        invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(1)),
    );
    assert_eq!(outcome.exit, ProcessExit::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the runner returned promptly after the timeout"
    );

    let pids = fs::read_to_string(dir.path().join("pids")).expect("the script recorded its pids");
    let pids = pids
        .split_whitespace()
        .map(|pid| pid.parse::<i32>().expect("a pid"))
        .collect::<Vec<_>>();
    assert_eq!(pids.len(), 2);
    for pid in pids {
        assert!(wait_until_gone(pid), "process {pid} survived the timeout");
    }
}

#[test]
fn setting_the_cancel_flag_stops_a_running_hook() {
    let dir = TempDir::new().expect("tempdir");
    write_script(dir.path(), "hook.sh", r#"echo $$ > "$PWD/pid"; sleep 30"#);
    let invocation = invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(30));
    let cancel = invocation.cancel.clone();
    let flipper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        cancel.store(true, Ordering::Release);
    });
    let outcome = run(&runner(), invocation);
    flipper.join().expect("flipper thread");
    assert_eq!(outcome.exit, ProcessExit::Cancelled);
    let pid = fs::read_to_string(dir.path().join("pid"))
        .expect("the script recorded its pid")
        .trim()
        .parse::<i32>()
        .expect("a pid");
    assert!(wait_until_gone(pid), "process {pid} survived cancellation");
}

#[test]
fn a_cancel_flag_set_before_the_run_spawns_nothing() {
    let dir = TempDir::new().expect("tempdir");
    let mut invocation = invocation(
        dir.path(),
        r#"touch "$PWD/ran""#,
        b"{}",
        Duration::from_secs(10),
    );
    invocation.cancel = Arc::new(AtomicBool::new(true));
    let outcome = run(&runner(), invocation);
    assert_eq!(outcome.exit, ProcessExit::Cancelled);
    assert!(!dir.path().join("ran").exists());
}

#[test]
fn a_missing_shell_is_a_spawn_failure() {
    let dir = TempDir::new().expect("tempdir");
    let runner = runner().with_shell_program("/nonexistent/hook-shell");
    let outcome = run(
        &runner,
        invocation(dir.path(), "true", b"{}", Duration::from_secs(10)),
    );
    assert!(
        matches!(outcome.exit, ProcessExit::SpawnFailed(_)),
        "got {:?}",
        outcome.exit
    );
}

#[test]
fn a_missing_working_directory_is_a_spawn_failure() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist");
    let outcome = run(
        &runner(),
        invocation(&missing, "true", b"{}", Duration::from_secs(10)),
    );
    assert!(
        matches!(outcome.exit, ProcessExit::SpawnFailed(_)),
        "got {:?}",
        outcome.exit
    );
}

#[test]
fn a_hook_that_floods_stderr_before_reading_stdin_does_not_deadlock() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        r#"head -c 300000 /dev/zero | tr '\0' x >&2
cat > /dev/null
printf '{"permission":"allow"}'"#,
    );
    let payload = vec![b'y'; 300_000];
    let outcome = run(
        &runner(),
        invocation(dir.path(), "./hook.sh", &payload, Duration::from_secs(20)),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(0));
    assert_eq!(outcome.stdout, br#"{"permission":"allow"}"#);
    assert_eq!(outcome.stderr_tail.len(), STDERR_TAIL_LIMIT);
}

#[test]
fn a_descendant_that_keeps_the_pipes_open_does_not_turn_an_exit_into_a_timeout() {
    let dir = TempDir::new().expect("tempdir");
    write_script(
        dir.path(),
        "hook.sh",
        r#"sleep 5 &
echo "$!" > "$PWD/child"
printf '{"agent_message":"no"}'
exit 2"#,
    );
    let started = Instant::now();
    let outcome = run(
        &runner(),
        invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(4)),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(2));
    assert_eq!(outcome.stdout, br#"{"agent_message":"no"}"#);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the exit was reported after the pipe grace, not the invocation timeout"
    );

    // The hook's own exit decided the outcome; what it left running is not the runner's to kill.
    let child = fs::read_to_string(dir.path().join("child"))
        .expect("the script recorded its child")
        .trim()
        .parse::<i32>()
        .expect("a pid");
    assert!(
        process_is_alive(child),
        "the background child was left alone"
    );
}

#[test]
fn a_hook_that_never_reads_stdin_still_finishes() {
    let dir = TempDir::new().expect("tempdir");
    let payload = vec![b'y'; 300_000];
    let outcome = run(
        &runner(),
        invocation(dir.path(), "exit 2", &payload, Duration::from_secs(10)),
    );
    assert_eq!(outcome.exit, ProcessExit::Code(2));
}
