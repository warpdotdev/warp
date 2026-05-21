use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use warpui::r#async::executor::Background;

use super::LogManager;
use crate::{
    path_with_suffix, rotations_sidecar_path, summaries_sidecar_path, MockSummarizer,
    RotationConfig, RotationEvent, RotationSummary,
};

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
        .register_resolved_path(log_path.clone(), executor.clone(), None, None)
        .expect("initial registration should succeed");

    drop(logger);

    let logger = manager
        .register_resolved_path(log_path.clone(), executor, None, None)
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
        .register_resolved_path(log_path.clone(), executor.clone(), None, None)
        .expect("initial registration should succeed");
    assert!(
        manager
            .register_resolved_path(log_path.clone(), executor, None, None)
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
        .register_resolved_path(log_path.clone(), executor.clone(), None, None)
        .expect("initial registration should succeed");

    // Close the channel without dropping the logger — the Arc<LogFileWriter> is still alive.
    logger.close();

    let new_logger = manager
        .register_resolved_path(log_path.clone(), executor, None, None)
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
        .register_resolved_path(log_path.clone(), executor.clone(), Some(rotation), None)
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
        .register_resolved_path(log_path.clone(), executor.clone(), None, None)
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

// ---------- rotation events + summarizer (advanced layer) ----------
//
// These cover the always-on rotation event log (`.rotations.jsonl`) and the
// optional summarizer (`.summaries.jsonl`). The advanced rotation work is
// scoped behind opt-in flags, so these tests both prove the new sidecars get
// written when configured AND pin the contract that existing callers see no
// new files when neither feature is asked for.

fn cleanup_rotation_set(log_path: &std::path::Path, max_rotation: usize) {
    let _ = std::fs::remove_file(log_path);
    for n in 1..=max_rotation {
        let _ = std::fs::remove_file(path_with_suffix(log_path, n));
    }
    let _ = std::fs::remove_file(rotations_sidecar_path(log_path));
    let _ = std::fs::remove_file(summaries_sidecar_path(log_path));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

/// When rotation is configured (no summarizer), a `.rotations.jsonl` sidecar
/// must appear and contain at least one parseable `RotationEvent` after the
/// active file rolls over.
#[test]
fn rotation_event_sidecar_is_written_when_rotation_fires() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("event-sidecar").join("server.log");

    let rotation = RotationConfig::new(256, 3).expect("construct");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), Some(rotation), None)
        .expect("registration should succeed");

    for i in 0..10 {
        logger.log(format!(
            "log line {i} padded out so each entry is comfortably over fifty bytes"
        ));
    }
    logger.close();
    drop(logger);

    let events_path = rotations_sidecar_path(&log_path);
    wait_for(2000, "rotation events sidecar to appear", || {
        if events_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            Some(())
        } else {
            None
        }
    });

    let contents = std::fs::read_to_string(&events_path).expect("read events sidecar");
    let parsed: Vec<RotationEvent> = contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line should parse as a RotationEvent"))
        .collect();
    assert!(
        !parsed.is_empty(),
        "expected at least one rotation event in {:?}, got: {:?}",
        events_path,
        contents,
    );
    // First event should match the active log path and report nonzero bytes.
    let first = &parsed[0];
    assert_eq!(first.active_log, log_path);
    assert!(first.bytes_rotated > 0);

    cleanup_rotation_set(&log_path, 3);
}

/// When both rotation and a `MockSummarizer` are configured, the
/// `.summaries.jsonl` sidecar should be populated. The mock returns
/// deterministic output, so the assertions don't depend on any model.
///
/// Note on how rotation events vs. summaries are emitted: an event is written
/// every time rotation fires; a summary is only written when a file was
/// actually discarded (i.e. we hit the `.max_rotation` cap). For the
/// thresholds chosen here, plenty of rotations occur so at least one summary
/// is produced.
#[test]
fn summary_sidecar_is_written_when_summarizer_configured_and_cap_hit() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("summary-sidecar").join("server.log");

    // Small threshold + max_rotation=2 means we hit the cap quickly and start
    // discarding (which is what triggers summarization).
    let rotation = RotationConfig::new(128, 2).expect("construct");
    let summarizer = Arc::new(MockSummarizer::default());

    let logger = manager
        .register_resolved_path(
            log_path.clone(),
            executor.clone(),
            Some(rotation),
            Some(summarizer),
        )
        .expect("registration should succeed");

    // Generous volume to guarantee multiple rotations + at least one discard.
    for i in 0..30 {
        logger.log(format!(
            "diagnostic line {i} padded out to over fifty bytes per entry to force rotation \
             quickly"
        ));
    }
    logger.close();
    drop(logger);

    let summaries_path = summaries_sidecar_path(&log_path);
    wait_for(3000, "summaries sidecar to appear", || {
        if summaries_path
            .metadata()
            .map(|m| m.len() > 0)
            .unwrap_or(false)
        {
            Some(())
        } else {
            None
        }
    });

    let contents = std::fs::read_to_string(&summaries_path).expect("read summaries");
    let parsed: Vec<RotationSummary> = contents
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line should parse as RotationSummary"))
        .collect();

    assert!(
        !parsed.is_empty(),
        "expected at least one summary in {:?}, got: {:?}",
        summaries_path,
        contents,
    );
    let first = &parsed[0];
    assert_eq!(first.model, "mock-summarizer-v0");
    assert!(
        !first.pipeline.is_empty(),
        "summary pipeline trace must be populated; got: {:?}",
        first,
    );

    cleanup_rotation_set(&log_path, 2);
}

/// Pin the opt-in contract: without rotation configured (no event log, no
/// summarizer either, since there's nothing to summarize), neither sidecar
/// file should ever exist. This protects legacy callers that haven't migrated
/// to the new register surface.
#[test]
fn no_sidecars_written_when_rotation_not_configured() {
    let mut manager = LogManager::new();
    let executor = Arc::new(Background::default());
    let log_path = temp_path("no-sidecars").join("server.log");

    let logger = manager
        .register_resolved_path(log_path.clone(), executor.clone(), None, None)
        .expect("registration should succeed");

    for i in 0..30 {
        logger.log(format!("line {i}"));
    }
    logger.close();
    drop(logger);

    wait_for(2000, "active file to be non-empty", || {
        if log_path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            Some(())
        } else {
            None
        }
    });

    assert!(
        !rotations_sidecar_path(&log_path).exists(),
        "no rotation events sidecar should exist when rotation is disabled"
    );
    assert!(
        !summaries_sidecar_path(&log_path).exists(),
        "no summary sidecar should exist when summarizer is disabled"
    );

    let _ = std::fs::remove_file(&log_path);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}
