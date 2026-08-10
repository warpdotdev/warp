use super::*;

/// APP-5243 / WARP-CLIENT-DEV-XT3: an empty path must never reach the platform watcher, because
/// macOS turns it into a null-`CFError` release that traps and kills the process.
#[test]
fn empty_paths_never_reach_the_platform_watcher() {
    assert!(ensure_watchable_path(Path::new("")).is_err());

    let directory = std::env::temp_dir();
    assert_eq!(ensure_watchable_path(&directory).ok(), Some(directory));
}
