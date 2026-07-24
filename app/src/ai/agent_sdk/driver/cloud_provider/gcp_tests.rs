use super::*;

use std::time::Duration;

/// Regression test for the `gcloud auth login` timeout: a best-effort command
/// that runs past its timeout must be killed so it cannot outlive setup.
///
/// We run a shell that writes a `started` marker, sleeps well past the timeout,
/// then writes a `done` marker. With `kill_on_drop(true)` (set by
/// `run_best_effort_with_timeout`) the shell is killed when the timeout drops
/// the wait future, so `done` is never written. Without the kill — the old
/// behavior, where `with_timeout` only stopped waiting on `output()` while the
/// spawned process kept running — the shell survives, wakes up after the sleep,
/// and writes `done`, which this test detects.
#[cfg(unix)]
#[test]
fn best_effort_command_is_killed_on_timeout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("marker");
    let script = format!(
        "echo started > {}; sleep 2; echo done >> {}",
        marker.display(),
        marker.display()
    );
    // Build the command with separate statements: `Command::new(..).arg(..)`
    // returns `&mut Command` (arg takes `&mut self`), so a chained initializer
    // would bind a reference to a temporary rather than an owned `Command`.
    let mut command = command::r#async::Command::new("sh");
    command.arg("-c").arg(script);

    let outcome = warpui::r#async::block_on(async {
        run_best_effort_with_timeout(command, Duration::from_millis(500)).await
    });
    assert!(
        matches!(outcome, BestEffortOutcome::Timeout),
        "expected timeout, got {outcome:?}"
    );

    // Wait past the `sleep 2` so a surviving (not-killed) shell would have
    // written `done` by now.
    std::thread::sleep(Duration::from_millis(2500));

    let contents = std::fs::read_to_string(&marker).expect("read marker");
    assert!(
        contents.contains("started"),
        "marker should contain 'started': {contents:?}"
    );
    assert!(
        !contents.contains("done"),
        "shell survived the timeout and wrote 'done' (kill_on_drop not working): {contents:?}"
    );
}

/// A command that exits within the timeout completes normally (Success) and is
/// not killed.
#[cfg(unix)]
#[test]
fn best_effort_command_completes_within_timeout() {
    let outcome = warpui::r#async::block_on(async {
        run_best_effort_with_timeout(
            command::r#async::Command::new("true"),
            Duration::from_secs(5),
        )
        .await
    });
    assert!(
        matches!(outcome, BestEffortOutcome::Success),
        "expected success, got {outcome:?}"
    );
}

/// A missing binary is reported as `NotFound` rather than a generic spawn
/// failure, so the caller can treat "not installed" as expected.
#[test]
fn best_effort_command_missing_binary_is_not_found() {
    let outcome = warpui::r#async::block_on(async {
        run_best_effort_with_timeout(
            command::r#async::Command::new("definitely-not-a-real-binary-xyz"),
            Duration::from_secs(5),
        )
        .await
    });
    assert!(
        matches!(outcome, BestEffortOutcome::NotFound),
        "expected NotFound, got {outcome:?}"
    );
}
