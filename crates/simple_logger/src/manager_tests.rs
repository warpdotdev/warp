use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use warpui::r#async::executor::Background;

use super::LogManager;

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
        .register_resolved_path(log_path.clone(), executor.clone())
        .expect("initial registration should succeed");

    drop(logger);

    let logger = manager
        .register_resolved_path(log_path.clone(), executor)
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
        .register_resolved_path(log_path.clone(), executor.clone())
        .expect("initial registration should succeed");
    assert!(
        manager
            .register_resolved_path(log_path.clone(), executor)
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
        .register_resolved_path(log_path.clone(), executor.clone())
        .expect("initial registration should succeed");

    // Close the channel without dropping the logger — the Arc<LogFileWriter> is still alive.
    logger.close();

    let new_logger = manager
        .register_resolved_path(log_path.clone(), executor)
        .expect("closed logger should be reclaimed even when Arc is still alive");

    drop(logger);
    drop(new_logger);
    cleanup_log_path(&log_path);
}
