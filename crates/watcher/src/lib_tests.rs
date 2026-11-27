use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

struct WatchCleanupLogger(AtomicUsize);

impl log::Log for WatchCleanupLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() == log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if record.level() == log::Level::Warn
            && record
                .args()
                .to_string()
                .starts_with("Failed to remove repo watcher")
        {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {}
}

#[test]
fn removing_an_unregistered_watch_does_not_warn() {
    static LOGGER: WatchCleanupLogger = WatchCleanupLogger(AtomicUsize::new(0));
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let (events, _) = async_channel::unbounded();
    let (commands, rx) = channel();
    let watcher = BackgroundFileWatcher::new(
        Duration::from_millis(10),
        WatcherEventHandler { tx: events },
        rx,
    )
    .unwrap();
    let (response, result) = oneshot::channel();
    commands
        .send(BackgroundFileWatcherCommand::RemovePath {
            path: std::env::temp_dir(),
            response,
        })
        .unwrap();
    drop(commands);

    watcher.run();

    let error = futures::executor::block_on(result).unwrap().unwrap_err();
    assert!(matches!(
        error.downcast_ref::<notify::Error>().unwrap().kind,
        notify::ErrorKind::WatchNotFound
    ));
    assert_eq!(LOGGER.0.load(Ordering::Relaxed), 0);
}

/// APP-5243 / WARP-CLIENT-DEV-XT3: an empty path must never reach the platform watcher, because
/// macOS turns it into a null-`CFError` release that traps and kills the process.
#[test]
fn empty_paths_never_reach_the_platform_watcher() {
    assert!(ensure_watchable_path(Path::new("")).is_err());

    let directory = std::env::temp_dir();
    assert_eq!(ensure_watchable_path(&directory).ok(), Some(directory));
}
