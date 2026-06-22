use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use warpui_core::r#async::executor::Background;

use super::LogManager;
use crate::{path_with_suffix, RotationConfig};

fn temp_path(name: &str) -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "simple-logger-tests-{name}-{}-{id}",
        std::process::id()
    ))
}
fn cleanup_log_path(log_path: &Path) {
    let _ = std::fs::remove_file(log_path);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

#[test]
fn register_resolved_path_reuses_stale_entries_after_drop() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("re-register").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), None)
        .expect("initial registration should succeed");

    drop(logger);

    let logger = manager
        .register_resolved_path(log_path.clone(), executor, None)
        .expect("stale entry should be reclaimed after the logger is dropped");
    drop(logger);
    cleanup_log_path(&log_path);
}

#[test]
fn register_resolved_path_rejects_duplicate_active_loggers() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("collision").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), None)
        .expect("initial registration should succeed");
    assert!(
        manager
            .register_resolved_path(log_path.clone(), executor, None)
            .is_err(),
        "live logger should block duplicate registration"
    );

    drop(logger);
    cleanup_log_path(&log_path);
}

#[test]
fn register_reclaims_closed_logger() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("close-reclaim").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), None)
        .expect("initial registration should succeed");

    // Close the channel without dropping the logger — the Arc<LogFileWriter> is still alive.
    logger.close();

    let new_logger = manager
        .register_resolved_path(log_path.clone(), executor, None)
        .expect("closed logger should be reclaimed even when Arc is still alive");

    drop(logger);
    drop(new_logger);
    cleanup_log_path(&log_path);
}

// ---------- end-to-end rotation through SimpleLogger ----------
//
// These tests drive a live `SimpleLogger` running on a real background
// executor and assert that, when configured with rotation, sufficient writes
// cause the active log file to roll over. They cover the integrated path
// (channel send → async write → byte counter → rotation), which the file-level
// unit tests in `lib_tests.rs` deliberately don't touch.

/// Wait until `predicate` returns `Some(value)`, polling every 10 ms. Returns
/// the value, or panics with `label` if the deadline elapses without the
/// predicate succeeding. Used to synchronize on async file writes without
/// hard-coding sleeps that pad every run with dead time.
fn wait_for<T>(deadline_ms: u64, label: &str, mut predicate: impl FnMut() -> Option<T>) -> T {
    // `instant::Instant` is the cross-target (incl. wasm) drop-in for
    // `std::time::Instant`; the rest of the workspace standardizes on it via
    // the `disallowed_types` clippy lint.
    let start = instant::Instant::now();
    let deadline = std::time::Duration::from_millis(deadline_ms);
    loop {
        if let Some(value) = predicate() {
            return value;
        }
        if start.elapsed() >= deadline {
            panic!(
                "wait_for({label}) timed out after {} ms",
                deadline.as_millis()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Repeatedly logging a moderately sized message must eventually cause the
/// active log file to be rotated to `.1` and a fresh active file to replace it.
/// This is the headline behavior #7723 asks for.
#[test]
fn simple_logger_with_rotation_rolls_active_file_over_when_threshold_exceeded() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("rotation-rolls-over").join("server.log");

    // Each log line is timestamped + the message + a newline; comfortably more
    // than 64 bytes per line. With a 256-byte threshold, five lines is enough
    // to guarantee at least one rotation.
    let rotation = RotationConfig::new(256, 3).expect("config should construct");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), Some(rotation))
        .expect("registration should succeed");

    for i in 0..10 {
        logger.log(format!(
            "log line {i} padded out so each entry is comfortably over fifty bytes"
        ));
    }
    // Closing the channel without dropping the Arc lets the background task
    // finish flushing pending writes deterministically.
    logger.close();
    drop(logger);

    // After draining, the active file should be reopened-truncated (or
    // contain just the tail end of writes), and at least one `.1` rotation
    // should exist with the rolled-over contents.
    let rotated = path_with_suffix(&log_path, 1);
    wait_for(2000, "rotated `.1` to appear", || {
        if rotated.exists() {
            Some(())
        } else {
            None
        }
    });

    assert!(
        rotated.exists(),
        "rotation should have produced {:?}",
        rotated
    );
    let rotated_contents = std::fs::read_to_string(&rotated).expect("read rotated file");
    assert!(
        rotated_contents.contains("log line"),
        "rotated file should contain log lines, got: {:?}",
        rotated_contents
    );

    // Cleanup: remove the whole rotation set if it exists.
    let _ = std::fs::remove_file(&log_path);
    for n in 1..=3 {
        let _ = std::fs::remove_file(path_with_suffix(&log_path, n));
    }
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

/// Without a rotation config, the same write volume that rotates a configured
/// logger must NOT rotate an unconfigured one. Pins the backward-compatibility
/// guarantee: existing callers (everything other than the MCP path) see
/// unchanged truncate-on-create behavior.
#[test]
fn simple_logger_without_rotation_does_not_rotate_even_at_high_volume() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("no-rotation").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), None)
        .expect("registration should succeed");

    for i in 0..50 {
        logger.log(format!(
            "log line {i} padded out so each entry is comfortably over fifty bytes"
        ));
    }
    logger.close();
    drop(logger);

    // Give the background task time to drain pending writes before we assert.
    wait_for(2000, "active file to be non-empty", || {
        if log_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            Some(())
        } else {
            None
        }
    });

    // No `.1` file ever gets created when rotation is disabled, no matter
    // how much we wrote.
    assert!(
        !path_with_suffix(&log_path, 1).exists(),
        "no rotation should occur when config is None"
    );

    let _ = std::fs::remove_file(&log_path);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

/// When `perform_rotation` fails (e.g. the rename of the active file to its
/// `.1` slot fails because that path is occupied by a non-empty directory),
/// the active log content must survive. Previously the callback fell through
/// to `open_truncated`, which destroyed the data rotation was meant to
/// preserve. The fix opens append-mode on failure and seeds the byte counter
/// from the existing file size.
#[test]
fn simple_logger_rotation_failure_preserves_active_log_content() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let dir = temp_path("rotation-failure-preserves");
    let log_path = dir.join("server.log");

    // Pre-stage `server.log.1` as a non-empty directory so the rename in
    // `perform_rotation` (step 3) will fail with a real I/O error rather than
    // succeed. This is the only error path the rotation callback observes.
    //
    // Use `max_rotation = 1` so step 2's "shift older slots up" loop is empty
    // (`1..1` is empty) — otherwise step 2 would silently rename the blocking
    // directory out of the way before step 3 ran.
    let blocked_target = path_with_suffix(&log_path, 1);
    std::fs::create_dir_all(&blocked_target).expect("create blocking dir");
    std::fs::write(blocked_target.join("placeholder"), b"sentinel").expect("populate blocking dir");

    let rotation = RotationConfig::new(256, 1).expect("config should construct");
    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), Some(rotation))
        .expect("registration should succeed");

    // Write enough to push past the 256-byte threshold and trigger rotation.
    for i in 0..10 {
        logger.log(format!(
            "log line {i} padded out so each entry is comfortably over fifty bytes"
        ));
    }
    logger.close();
    drop(logger);

    // After draining, the active log must still exist and contain the
    // pre-rotation content (because rotation could not move it aside). The
    // blocking directory must also still be intact — rotation didn't somehow
    // clobber it.
    wait_for(2000, "active log to flush", || {
        std::fs::metadata(&log_path)
            .ok()
            .filter(|m| m.len() > 0)
            .map(|_| ())
    });

    let preserved = std::fs::read_to_string(&log_path).expect("read active log");
    assert!(
        preserved.contains("log line"),
        "active log content must be preserved when rotation fails; got: {preserved:?}",
    );

    assert!(
        blocked_target.is_dir(),
        "blocking directory must remain intact after failed rotation",
    );
    assert!(
        blocked_target.join("placeholder").is_file(),
        "blocking directory's contents must be unchanged",
    );

    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_dir_all(&blocked_target);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}
