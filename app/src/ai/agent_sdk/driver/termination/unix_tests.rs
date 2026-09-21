use std::fs;
use std::time::Duration;

use futures::executor::block_on;
use tempfile::TempDir;

use super::InterruptWatch;
use crate::ai::agent_sdk::driver::termination::Interrupt;

/// Set on the re-executed child so it takes the [`signal_lifecycle_child`] path instead
/// of running the test body again. Its value selects which lifecycle to exercise.
const SIGNAL_CHILD_ENV: &str = "WARP_AGENT_DRIVER_SIGNAL_CHILD";
/// Printed by the child once its interrupt watch is armed, so the parent knows when the
/// first signal can be delivered.
const READY_MARKER: &str = "warp-signal-test: ready";
/// Printed by the child once it starts (and, for `int-hang`, deliberately stalls) its
/// shutdown work, so the parent knows when the second signal is meaningful.
const SHUTDOWN_MARKER: &str = "warp-signal-test: shutdown_started";
/// How long an `int-hang` child stalls before giving up. Only reached when the parent
/// fails to deliver the second signal, in which case exiting keeps the test from
/// leaving an orphaned process behind.
const STUCK_SHUTDOWN_BAILOUT: Duration = Duration::from_secs(60);

fn signal_lifecycle_child() -> ! {
    let kind = std::env::var(SIGNAL_CHILD_ENV).expect("child kind");
    let expected = match kind.as_str() {
        "term" => Interrupt::Terminate,
        "int" | "int-hang" => Interrupt::Interrupt,
        other => panic!("unknown child kind {other}"),
    };

    let background = warpui::r#async::executor::Background::default();
    let mut watch = block_on(InterruptWatch::register(&background)).expect("signal watch");
    println!("{READY_MARKER}");

    let interrupt = block_on(watch.wait());
    assert_eq!(interrupt, expected);

    if kind == "int-hang" {
        println!("{SHUTDOWN_MARKER}");
        std::thread::sleep(STUCK_SHUTDOWN_BAILOUT);
        std::process::exit(1);
    }
    watch.terminate(interrupt);
}

/// Re-executes this test binary as a child running only `test_name`, delivers
/// `first_sig`, optionally verifies another signal does not terminate it, then delivers
/// `second_sig` and asserts the child died from `expected_sig`.
///
/// Signal dispositions are process-wide, so the behavior under test is only observable
/// in a dedicated process; `test_name` must therefore be the fully-qualified name of the
/// calling test.
fn spawn_signal_lifecycle_child(
    kind: &str,
    test_name: &str,
    first_sig: i32,
    non_terminating_sig: Option<i32>,
    second_sig: Option<i32>,
    expected_sig: i32,
    expect_stuck_shutdown: bool,
) {
    use std::os::unix::process::ExitStatusExt as _;

    if std::env::var_os(SIGNAL_CHILD_ENV).is_some() {
        signal_lifecycle_child();
    }

    let dir = TempDir::new().unwrap();
    let stdout_path = dir.path().join("stdout");
    let stderr_path = dir.path().join("stderr");
    let mut cmd = command::blocking::Command::new(std::env::current_exe().unwrap());
    cmd.arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .env(SIGNAL_CHILD_ENV, kind)
        .env("RUST_TEST_THREADS", "1")
        .stdout(fs::File::create(&stdout_path).unwrap())
        .stderr(fs::File::create(&stderr_path).unwrap());
    for (key, _) in std::env::vars() {
        if key.starts_with("NEXTEST") {
            cmd.env_remove(&key);
        }
    }
    let mut child = cmd.spawn().unwrap();

    let wait_for_marker = |child: &mut std::process::Child, marker: &str| {
        for _ in 0..1_000 {
            if fs::read_to_string(&stdout_path)
                .unwrap_or_default()
                .contains(marker)
            {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "signal child exited before printing {marker:?}: status={status:?} \
                     stdout={} stderr={}",
                    fs::read_to_string(&stdout_path).unwrap_or_default(),
                    fs::read_to_string(&stderr_path).unwrap_or_default()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "signal child never printed {marker:?}; stdout={} stderr={}",
            fs::read_to_string(&stdout_path).unwrap_or_default(),
            fs::read_to_string(&stderr_path).unwrap_or_default()
        );
    };

    wait_for_marker(&mut child, READY_MARKER);

    // SAFETY: `child.id()` is this child's pid; the signal is sent only to it.
    unsafe {
        libc::kill(child.id() as libc::pid_t, first_sig);
    }
    if non_terminating_sig.is_some() || second_sig.is_some() {
        wait_for_marker(&mut child, SHUTDOWN_MARKER);
    }
    if let Some(non_terminating_sig) = non_terminating_sig {
        unsafe {
            libc::kill(child.id() as libc::pid_t, non_terminating_sig);
        }
        for _ in 0..100 {
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "signal child exited after a different signal: status={status:?} stdout={} \
                     stderr={}",
                    fs::read_to_string(&stdout_path).unwrap_or_default(),
                    fs::read_to_string(&stderr_path).unwrap_or_default()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    if let Some(second_sig) = second_sig {
        unsafe {
            libc::kill(child.id() as libc::pid_t, second_sig);
        }
    }

    let status = child.wait().unwrap();
    let stdout = fs::read_to_string(&stdout_path).unwrap_or_default();
    assert_eq!(
        status.signal(),
        Some(expected_sig),
        "status={status:?} stdout={stdout} stderr={}",
        fs::read_to_string(&stderr_path).unwrap_or_default()
    );
    assert_eq!(
        stdout.contains(SHUTDOWN_MARKER),
        expect_stuck_shutdown,
        "stdout={stdout}"
    );
}

#[test]
fn sigterm_subprocess_exits_signaled() {
    spawn_signal_lifecycle_child(
        "term",
        "ai::agent_sdk::driver::termination::unix::tests::sigterm_subprocess_exits_signaled",
        libc::SIGTERM,
        None,
        None,
        libc::SIGTERM,
        false,
    );
}

#[test]
fn sigint_subprocess_exits_signaled() {
    spawn_signal_lifecycle_child(
        "int",
        "ai::agent_sdk::driver::termination::unix::tests::sigint_subprocess_exits_signaled",
        libc::SIGINT,
        None,
        None,
        libc::SIGINT,
        false,
    );
}

#[test]
fn second_sigint_kills_during_stuck_shutdown() {
    spawn_signal_lifecycle_child(
        "int-hang",
        "ai::agent_sdk::driver::termination::unix::tests::second_sigint_kills_during_stuck_shutdown",
        libc::SIGINT,
        None,
        Some(libc::SIGINT),
        libc::SIGINT,
        true,
    );
}

#[test]
fn different_signal_does_not_kill_during_stuck_shutdown() {
    spawn_signal_lifecycle_child(
        "int-hang",
        "ai::agent_sdk::driver::termination::unix::tests::\
         different_signal_does_not_kill_during_stuck_shutdown",
        libc::SIGINT,
        Some(libc::SIGTERM),
        Some(libc::SIGINT),
        libc::SIGINT,
        true,
    );
}
