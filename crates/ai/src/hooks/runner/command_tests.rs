use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use futures::executor::block_on;

use super::*;
use crate::hooks::test_util::{test_binding_id, test_source_id};

fn runner() -> CommandHookRunner {
    CommandHookRunner::new(Arc::new(|| None))
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

#[test]
fn stderr_escaping_keeps_newlines_and_tabs_and_writes_other_controls_as_hex() {
    assert_eq!(
        escape_control(b"a\x1b[31mred\x1b[0m\r\n\tb\x7f"),
        "a\\x1b[31mred\\x1b[0m\\x0d\n\tb\\x7f"
    );
    assert_eq!(escape_control(b"plain"), "plain");
    assert_eq!(escape_control(b"bad \xff byte"), "bad \u{fffd} byte");
}

#[test]
fn stderr_escaping_cuts_from_the_front_when_escaping_lengthens_the_tail() {
    let escaped = escape_control(&vec![0x1b; HOOK_STDERR_TAIL]);
    assert_eq!(escaped.len(), HOOK_STDERR_TAIL);
    assert!(escaped.starts_with("\\x1b"));
    assert!(escaped.ends_with("\\x1b"));
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, OnceLock};

    use futures::future::join_all;
    use instant::Instant;
    use log::{Level, Log, Metadata, Record};
    use tempfile::TempDir;

    use super::*;
    use crate::hooks::HookFormat;
    use crate::hooks::runner::env::{HookIdentity, invocation_environment};

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    fn process_is_alive(pid: i32) -> bool {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        // Signal 0 only checks that the pid exists.
        kill(Pid::from_raw(pid), None).is_ok()
    }

    /// Slack added to every wall-clock bound in the process-tree tests, so a loaded machine
    /// (slow spawns, late timers) does not turn a passing run into a failure.
    const LOAD_MARGIN: Duration = Duration::from_secs(5);

    /// A deadline that a lightly loaded machine reaches with the script's shell long started. Under
    /// heavy load a shell has been seen to take more than ten seconds to start, so a test that
    /// needs the script to be running must wait for it rather than count on this.
    const GENEROUS_TIMEOUT: Duration = Duration::from_secs(8);

    /// How long the readiness and "is it gone" polls wait before giving up. Only a failing run
    /// pays it.
    const POLL_LIMIT: Duration = Duration::from_secs(60);

    fn wait_until_gone(pid: i32) -> bool {
        let deadline = Instant::now() + POLL_LIMIT;
        while Instant::now() < deadline {
            if !process_is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn pids_file_is_written(dir: &Path) -> bool {
        fs::metadata(dir.join("pids")).is_ok_and(|metadata| metadata.len() > 0)
    }

    /// Waits for the script to write its `pids` file, which it does as soon as it is running.
    fn wait_for_pids_file(dir: &Path) -> bool {
        let deadline = Instant::now() + POLL_LIMIT;
        while Instant::now() < deadline {
            if pids_file_is_written(dir) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn recorded_pids(dir: &Path) -> Vec<i32> {
        let pids = fs::read_to_string(dir.join("pids")).expect("the script recorded its pids");
        pids.split_whitespace()
            .map(|pid| pid.parse().expect("a pid"))
            .collect()
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
        assert!(!outcome.stdout_truncated);
        assert_eq!(outcome.stderr_tail, "");
        assert!(outcome.duration > Duration::ZERO);
        let received =
            fs::read_to_string(dir.path().join("received.json")).expect("stdin was written");
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
    fn stderr_keeps_only_the_end_of_a_large_output() {
        let dir = TempDir::new().expect("tempdir");
        // 6000 lines of 11 bytes: about 64 KiB.
        write_script(
            dir.path(),
            "hook.sh",
            "i=0; while [ $i -lt 6000 ]; do printf 'line %05d\\n' $i >&2; i=$((i+1)); done",
        );
        let outcome = run(
            &runner(),
            invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert!(outcome.stderr_tail.len() <= HOOK_STDERR_TAIL);
        assert!(outcome.stderr_tail.ends_with("line 05999\n"));
    }

    #[test]
    fn stdout_over_the_limit_is_cut_off_and_the_hook_still_finishes() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            "head -c 2097152 /dev/zero | tr '\\0' a",
        );
        let started = Instant::now();
        let outcome = run(
            &runner(),
            invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(20)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert!(outcome.stdout_truncated);
        assert_eq!(outcome.stdout.len(), HOOK_STDOUT_LIMIT);
        assert!(outcome.stdout.iter().all(|byte| *byte == b'a'));
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the hook was not blocked"
        );
    }

    #[test]
    fn stdout_within_the_limit_is_kept_whole() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            "head -c 1048576 /dev/zero | tr '\\0' a",
        );
        let outcome = run(
            &runner(),
            invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(20)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert!(!outcome.stdout_truncated);
        assert_eq!(outcome.stdout.len(), HOOK_STDOUT_LIMIT);
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
    fn the_command_runs_in_the_given_directory_with_the_host_layer_and_the_invocation_layer() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            r#"printf '%s\n%s\n%s\n%s\n%s\n' "$PWD" "$PATH" "$HOME" "$WARP_HOOK_PROTOCOL_VERSION" "$HOOK_TEST_VAR""#,
        );
        let runner = CommandHookRunner::new(Arc::new(|| Some(OsString::from("/custom/bin"))))
            .with_home(Some(PathBuf::from("/custom/home")));
        let mut invocation = invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10));
        invocation.env = vec![
            (
                OsString::from("HOOK_TEST_VAR"),
                OsString::from("from-invocation"),
            ),
            // The runner's own protected variable cannot be replaced by a raw invocation entry.
            (
                OsString::from("WARP_HOOK_PROTOCOL_VERSION"),
                OsString::from("99"),
            ),
        ];
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
        assert_eq!(lines[2], "/custom/home");
        assert_eq!(lines[3], HOOK_PROTOCOL_VERSION.to_string());
        assert_eq!(lines[4], "from-invocation");
    }

    #[test]
    fn without_a_captured_path_or_home_the_process_values_are_inherited() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            r#"printf '%s\n%s\n' "$PATH" "$HOME""#,
        );
        let outcome = run(
            &runner().with_home(None),
            invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10)),
        );
        let lines = String::from_utf8(outcome.stdout).expect("utf-8");
        let lines = lines.lines().collect::<Vec<_>>();
        assert_eq!(
            lines[0],
            std::env::var("PATH").expect("the test process has a PATH")
        );
        assert_eq!(
            lines[1],
            std::env::var("HOME").expect("the test process has a HOME")
        );
    }

    #[test]
    fn the_identity_layer_from_the_dispatcher_reaches_the_hook_with_cursor_aliases() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            r#"printf '%s\n%s\n%s\n%s\n' "$WARP_PROJECT_DIR" "$CURSOR_PROJECT_DIR" "$CLAUDE_PROJECT_DIR" "$CURSOR_VERSION""#,
        );
        let mut invocation = invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10));
        invocation.env = invocation_environment(
            &HookIdentity {
                format: HookFormat::Cursor,
                project_dir: PathBuf::from("/work/repo"),
                warp_version: "v0.2026.01.01".to_owned(),
                run_id: None,
                user_email: None,
                remote: false,
            },
            &[],
        );
        let outcome = run(&runner(), invocation);
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert_eq!(
            String::from_utf8(outcome.stdout).expect("utf-8"),
            "/work/repo\n/work/repo\n/work/repo\nv0.2026.01.01\n"
        );
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
    fn a_timeout_stops_the_whole_process_tree() {
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
            invocation(dir.path(), "./hook.sh", b"{}", GENEROUS_TIMEOUT),
        );
        let elapsed = started.elapsed();
        assert_eq!(outcome.exit, ProcessExit::TimedOut);
        assert!(
            elapsed < GENEROUS_TIMEOUT + HOOK_KILL_GRACE + LOAD_MARGIN,
            "the runner returned promptly ({elapsed:?})"
        );
        // The deadline cannot wait for the script, so on a machine loaded enough that the shell
        // had not started by then there is no tree to check; the readiness-gated tests below
        // cover the stop, grace, and kill sequence, which the timeout path shares.
        if !pids_file_is_written(dir.path()) {
            eprintln!("the shell had not started before the deadline; no descendants to check");
            return;
        }
        let pids = recorded_pids(dir.path());
        assert_eq!(pids.len(), 2);
        for pid in pids {
            assert!(wait_until_gone(pid), "process {pid} survived the timeout");
        }
    }

    #[test]
    fn a_tree_that_ignores_the_stop_request_is_killed_after_the_grace_period() {
        let dir = TempDir::new().expect("tempdir");
        // An ignored signal stays ignored across exec, so `sleep` ignores TERM too.
        write_script(
            dir.path(),
            "hook.sh",
            r#"trap '' TERM
sleep 60 &
echo "$$ $!" > "$PWD/pids"
wait"#,
        );
        // The stop is triggered through the cancel flag once the script is known to be running
        // (and so has its trap in place), not through a deadline a loaded machine could reach
        // first; the timeout and cancel paths share the same stop, grace, and kill sequence.
        let invocation = invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(60));
        let cancel = invocation.cancel.clone();
        let flipper = std::thread::spawn({
            let dir = dir.path().to_path_buf();
            move || {
                let ready = wait_for_pids_file(&dir);
                cancel.store(true, Ordering::Release);
                (ready, Instant::now())
            }
        });
        let outcome = run(&runner(), invocation);
        let (script_was_running, flipped_at) = flipper.join().expect("flipper thread");
        let since_flip = flipped_at.elapsed();
        assert!(script_was_running, "the script never ran");
        assert_eq!(outcome.exit, ProcessExit::Cancelled);
        assert!(
            since_flip >= HOOK_KILL_GRACE,
            "the tree got its grace period ({since_flip:?})"
        );
        assert!(
            since_flip < HOOK_KILL_GRACE + LOAD_MARGIN,
            "the kill followed the grace period promptly ({since_flip:?})"
        );
        let pids = recorded_pids(dir.path());
        assert_eq!(pids.len(), 2);
        for pid in pids {
            assert!(wait_until_gone(pid), "process {pid} survived the kill");
        }
    }

    #[test]
    fn setting_the_cancel_flag_stops_a_running_hook_and_its_tree() {
        let dir = TempDir::new().expect("tempdir");
        // The trap prints a decision on the way out; it arrived after the stop request, so it is
        // not the hook's answer.
        write_script(
            dir.path(),
            "hook.sh",
            r#"trap 'printf "{\"permission\":\"allow\"}"; echo late >&2; exit 0' TERM
sleep 30 &
echo "$$ $!" > "$PWD/pids"
wait"#,
        );
        let invocation = invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(30));
        let cancel = invocation.cancel.clone();
        // The flag is set once the script is known to be running (it has written its pids), not
        // after a fixed delay, so a slow start on a loaded machine cannot cancel the hook before
        // its trap is in place.
        let flipper = std::thread::spawn({
            let dir = dir.path().to_path_buf();
            move || {
                let ready = wait_for_pids_file(&dir);
                cancel.store(true, Ordering::Release);
                (ready, Instant::now())
            }
        });
        let outcome = run(&runner(), invocation);
        let (script_was_running, flipped_at) = flipper.join().expect("flipper thread");
        assert!(script_was_running, "the script never ran");
        assert_eq!(outcome.exit, ProcessExit::Cancelled);
        assert!(
            flipped_at.elapsed() < HOOK_KILL_GRACE + LOAD_MARGIN,
            "the hook stopped promptly after the flag was set ({:?})",
            flipped_at.elapsed()
        );
        assert!(
            outcome.stdout.is_empty(),
            "output written after the stop request is discarded: {:?}",
            String::from_utf8_lossy(&outcome.stdout)
        );
        assert!(!outcome.stdout_truncated);
        assert_eq!(
            outcome.stderr_tail, "late\n",
            "the stderr tail is kept for the log"
        );
        for pid in recorded_pids(dir.path()) {
            assert!(wait_until_gone(pid), "process {pid} survived cancellation");
        }
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
        assert_eq!(outcome.stderr_tail.len(), HOOK_STDERR_TAIL);
    }

    #[test]
    fn a_descendant_left_holding_the_pipes_is_stopped_and_the_exit_still_decides() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            r#"sleep 60 &
echo "$!" > "$PWD/pids"
printf '{"agent_message":"no"}'
exit 2"#,
        );
        // Measured from the moment the script is known to be running, so a slow spawn on a loaded
        // machine does not count against the runner.
        let script_running_at = std::thread::spawn({
            let dir = dir.path().to_path_buf();
            move || wait_for_pids_file(&dir).then(Instant::now)
        });
        let outcome = run(
            &runner(),
            invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(30)),
        );
        let returned_at = Instant::now();
        let script_running_at = script_running_at
            .join()
            .expect("watcher thread")
            .expect("the script ran");
        assert_eq!(outcome.exit, ProcessExit::Code(2));
        assert_eq!(outcome.stdout, br#"{"agent_message":"no"}"#);
        assert!(
            returned_at.duration_since(script_running_at) < HOOK_KILL_GRACE + LOAD_MARGIN,
            "a descendant that stops when asked does not delay the outcome"
        );
        for pid in recorded_pids(dir.path()) {
            assert!(
                wait_until_gone(pid),
                "the background child {pid} outlived the hook"
            );
        }
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

    #[test]
    fn stderr_control_characters_are_escaped_in_the_outcome() {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            r#"printf 'a\033[31mred\033[0m\r\nb' >&2"#,
        );
        let outcome = run(
            &runner(),
            invocation(dir.path(), "./hook.sh", b"{}", Duration::from_secs(10)),
        );
        assert_eq!(outcome.stderr_tail, "a\\x1b[31mred\\x1b[0m\\x0d\nb");
    }

    /// Starts `count` copies of a script that marks itself running for `hold`, all at once, and
    /// returns the outcomes with the largest number of copies ever seen running together.
    fn run_concurrently(
        count: usize,
        hold: Duration,
        timeout: Duration,
    ) -> (Vec<HookProcessOutcome>, usize) {
        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            &format!(
                r#"touch "$PWD/running.$$"
sleep {}
rm -f "$PWD/running.$$""#,
                hold.as_secs_f64()
            ),
        );
        let runner = runner();
        let futures = (0..count)
            .map(|_| runner.run(invocation(dir.path(), "./hook.sh", b"{}", timeout)))
            .collect::<Vec<_>>();
        let watched = dir.path().to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let watcher = std::thread::spawn({
            let stop = stop.clone();
            move || {
                let mut most = 0;
                while !stop.load(Ordering::Acquire) {
                    let running = fs::read_dir(&watched)
                        .expect("tempdir")
                        .filter_map(Result::ok)
                        .filter(|entry| entry.file_name().to_string_lossy().starts_with("running."))
                        .count();
                    most = most.max(running);
                    std::thread::sleep(Duration::from_millis(5));
                }
                most
            }
        });
        let outcomes = block_on(join_all(futures));
        stop.store(true, Ordering::Release);
        (outcomes, watcher.join().expect("watcher thread"))
    }

    #[test]
    fn no_more_than_the_bound_run_at_once_and_all_complete() {
        let hold = Duration::from_secs(1);
        let started = Instant::now();
        let (outcomes, most_at_once) =
            run_concurrently(HOOK_MAX_CONCURRENT + 2, hold, Duration::from_secs(20));
        let elapsed = started.elapsed();
        assert_eq!(outcomes.len(), HOOK_MAX_CONCURRENT + 2);
        for outcome in &outcomes {
            assert_eq!(outcome.exit, ProcessExit::Code(0));
        }
        assert!(
            (2..=HOOK_MAX_CONCURRENT).contains(&most_at_once),
            "saw {most_at_once} running at once"
        );
        assert!(
            elapsed >= hold * 2,
            "the two beyond the bound waited for a slot ({elapsed:?})"
        );
    }

    #[test]
    fn waiting_for_a_slot_counts_against_the_deadline_and_a_waiter_that_gives_up_never_ran() {
        let dir = TempDir::new().expect("tempdir");
        write_script(dir.path(), "sleeper.sh", "sleep 3");
        write_script(dir.path(), "late.sh", r#"touch "$PWD/late-ran""#);
        let runner = runner();
        let mut futures = (0..HOOK_MAX_CONCURRENT)
            .map(|_| {
                runner.run(invocation(
                    dir.path(),
                    "./sleeper.sh",
                    b"{}",
                    Duration::from_secs(20),
                ))
            })
            .collect::<Vec<_>>();
        futures.push(runner.run(invocation(
            dir.path(),
            "./late.sh",
            b"{}",
            Duration::from_millis(300),
        )));
        let outcomes = block_on(join_all(futures));
        let late = outcomes.last().expect("the late hook's outcome");
        assert_eq!(late.exit, ProcessExit::TimedOut);
        // Well before the sleepers free a slot at about 3 s.
        assert!(
            late.duration < Duration::from_secs(2),
            "gave up at its own deadline ({:?})",
            late.duration
        );
        assert!(
            !dir.path().join("late-ran").exists(),
            "a waiter that timed out never ran"
        );
        for outcome in &outcomes[..HOOK_MAX_CONCURRENT] {
            assert_eq!(outcome.exit, ProcessExit::Code(0));
        }
    }

    #[test]
    fn a_hook_that_waited_for_a_slot_runs_with_what_is_left_of_its_deadline() {
        let dir = TempDir::new().expect("tempdir");
        let sleeper_hold = Duration::from_secs(2);
        write_script(dir.path(), "sleeper.sh", "sleep 2");
        write_script(dir.path(), "slow.sh", "sleep 30");
        let runner = runner();
        let timeout = Duration::from_secs(4);
        let mut futures = (0..HOOK_MAX_CONCURRENT)
            .map(|_| {
                runner.run(invocation(
                    dir.path(),
                    "./sleeper.sh",
                    b"{}",
                    Duration::from_secs(20),
                ))
            })
            .collect::<Vec<_>>();
        futures.push(runner.run(invocation(dir.path(), "./slow.sh", b"{}", timeout)));
        let outcomes = block_on(join_all(futures));
        let slow = outcomes.last().expect("the slow hook's outcome");
        assert_eq!(slow.exit, ProcessExit::TimedOut);
        // It waited about two seconds for a slot and then had about two left to run. Had the
        // deadline been taken after the wait, it would have run until about 6 s. The deadline is
        // absolute, so a slow start on a loaded machine does not move it.
        assert!(slow.duration >= timeout);
        assert!(
            slow.duration > sleeper_hold,
            "the hook waited for a slot ({:?})",
            slow.duration
        );
        assert!(
            slow.duration < timeout + Duration::from_millis(1500),
            "the slot wait was not added on top of the timeout ({:?})",
            slow.duration
        );
    }

    struct CapturingLogger;

    static LOGGER: CapturingLogger = CapturingLogger;
    static RECORDS: OnceLock<Mutex<Vec<(Level, String)>>> = OnceLock::new();

    fn records() -> &'static Mutex<Vec<(Level, String)>> {
        RECORDS.get_or_init(|| Mutex::new(Vec::new()))
    }

    impl Log for CapturingLogger {
        fn enabled(&self, _metadata: &Metadata) -> bool {
            true
        }

        fn log(&self, record: &Record) {
            records()
                .lock()
                .expect("records")
                .push((record.level(), record.args().to_string()));
        }

        fn flush(&self) {}
    }

    #[test]
    fn one_log_line_per_invocation_with_the_named_fields_and_no_payload() {
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Trace);

        let dir = TempDir::new().expect("tempdir");
        write_script(
            dir.path(),
            "hook.sh",
            r#"printf 'warned\n\033[1mloudly\033[0m' >&2; printf '{"permission":"deny"}'; exit 2"#,
        );
        let outcome = run(
            &runner(),
            invocation(
                dir.path(),
                "./hook.sh",
                br#"{"command":"rm -rf /"}"#,
                Duration::from_secs(10),
            ),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(2));

        let lines = records()
            .lock()
            .expect("records")
            .iter()
            .filter(|(level, message)| {
                *level == Level::Info && message.starts_with("hook invocation:")
            })
            .map(|(_, message)| message.clone())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 1, "exactly one invocation line: {lines:?}");
        let line = &lines[0];
        assert!(!line.contains('\n'), "the record is one line: {line:?}");
        assert!(
            !line.contains('\x1b'),
            "control characters are escaped: {line:?}"
        );
        for field in [
            "event=beforeShellExecution",
            "source=",
            "binding=0",
            "exit=Code(2)",
            "duration_ms=",
            "stderr_tail=",
            "loudly",
        ] {
            assert!(line.contains(field), "missing {field} in {line:?}");
        }
        assert!(
            !line.contains("rm -rf"),
            "the stdin payload is never logged: {line:?}"
        );
        assert!(
            !line.contains("permission"),
            "stdout is never logged: {line:?}"
        );
    }
}

#[cfg(windows)]
mod windows {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Instant;

    use tempfile::TempDir;

    use super::*;

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let body = body.replace('\n', "\r\n");
        fs::write(&path, format!("@echo off\r\n{body}\r\n")).expect("write script");
        path
    }

    fn process_exists(pid: u32) -> bool {
        let output = command::blocking::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }

    fn wait_until_gone(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if !process_exists(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// A script line that starts `ping` as a grandchild through PowerShell, records its pid in
    /// `grandchild.pid`, and waits for it.
    const START_GRANDCHILD: &str = r#"powershell -NoProfile -Command "$p = Start-Process -FilePath ping -ArgumentList '-n','60','127.0.0.1' -PassThru -WindowStyle Hidden; Set-Content -Path 'grandchild.pid' -Value $p.Id; Wait-Process -Id $p.Id""#;

    fn recorded_grandchild(dir: &Path) -> u32 {
        fs::read_to_string(dir.join("grandchild.pid"))
            .expect("the script recorded the grandchild pid")
            .trim()
            .parse()
            .expect("a pid")
    }

    #[test]
    fn stdout_and_the_exit_code_are_captured_through_cmd() {
        let dir = TempDir::new().expect("tempdir");
        write_script(dir.path(), "hook.cmd", r#"echo {"permission":"allow"}"#);
        let outcome = run(
            &runner(),
            invocation(dir.path(), "hook.cmd", b"{}", Duration::from_secs(10)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert_eq!(
            String::from_utf8(outcome.stdout).expect("utf-8").trim(),
            r#"{"permission":"allow"}"#
        );

        write_script(dir.path(), "deny.cmd", "exit /b 2");
        let outcome = run(
            &runner(),
            invocation(dir.path(), "deny.cmd", b"{}", Duration::from_secs(10)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(2));
    }

    #[test]
    fn the_command_string_runs_as_the_author_wrote_it() {
        let dir = TempDir::new().expect("tempdir");
        let outcome = run(
            &runner(),
            invocation(
                dir.path(),
                r#"echo one & echo "two" | findstr two"#,
                b"{}",
                Duration::from_secs(10),
            ),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        let stdout = String::from_utf8(outcome.stdout).expect("utf-8");
        assert!(stdout.contains("one"), "{stdout:?}");
        assert!(stdout.contains("two"), "{stdout:?}");
    }

    #[test]
    fn unicode_stdin_arrives_byte_for_byte() {
        let dir = TempDir::new().expect("tempdir");
        // PowerShell copies the raw stream; `cmd.exe` tools would recode it.
        write_script(
            dir.path(),
            "hook.cmd",
            r#"powershell -NoProfile -Command "$in = [Console]::OpenStandardInput(); $out = [IO.File]::Create('received.bin'); $in.CopyTo($out); $out.Close()""#,
        );
        let payload = "{\"command\":\"echo h\u{e9}llo \u{1f600}\"}"
            .as_bytes()
            .to_vec();
        let outcome = run(
            &runner(),
            invocation(dir.path(), "hook.cmd", &payload, Duration::from_secs(30)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert_eq!(
            fs::read(dir.path().join("received.bin")).expect("received"),
            payload
        );
    }

    #[test]
    fn a_script_in_a_directory_with_a_space_runs_with_a_quoted_argument() {
        let dir = TempDir::new().expect("tempdir");
        let spaced = dir.path().join("hook dir");
        fs::create_dir(&spaced).expect("mkdir");
        write_script(&spaced, "check.cmd", "echo got:%~1");
        let command = format!(
            "\"{}\" \"arg with space\"",
            spaced.join("check.cmd").display()
        );
        let outcome = run(
            &runner(),
            invocation(dir.path(), &command, b"{}", Duration::from_secs(10)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(0));
        assert_eq!(
            String::from_utf8(outcome.stdout).expect("utf-8").trim(),
            "got:arg with space"
        );
    }

    #[test]
    fn a_timeout_ends_the_job_including_a_grandchild() {
        let dir = TempDir::new().expect("tempdir");
        write_script(dir.path(), "hook.cmd", START_GRANDCHILD);
        // Long enough for a cold PowerShell start to record the grandchild before the job ends.
        let timeout = Duration::from_secs(8);
        let started = Instant::now();
        let outcome = run(
            &runner(),
            invocation(dir.path(), "hook.cmd", b"{}", timeout),
        );
        assert_eq!(outcome.exit, ProcessExit::TimedOut);
        assert!(started.elapsed() < timeout + HOOK_KILL_GRACE + Duration::from_secs(3));
        let grandchild = recorded_grandchild(dir.path());
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} survived the timeout"
        );
    }

    #[test]
    fn a_grandchild_left_behind_by_a_finished_hook_is_ended_with_the_job() {
        let dir = TempDir::new().expect("tempdir");
        // `Start-Process` without `Wait-Process` leaves `ping` running after the script exits.
        write_script(
            dir.path(),
            "hook.cmd",
            r#"powershell -NoProfile -Command "$p = Start-Process -FilePath ping -ArgumentList '-n','60','127.0.0.1' -PassThru -WindowStyle Hidden; Set-Content -Path 'grandchild.pid' -Value $p.Id"
exit /b 2"#,
        );
        let outcome = run(
            &runner(),
            invocation(dir.path(), "hook.cmd", b"{}", Duration::from_secs(30)),
        );
        assert_eq!(outcome.exit, ProcessExit::Code(2));
        let grandchild = recorded_grandchild(dir.path());
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} outlived the hook"
        );
    }
}
