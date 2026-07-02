use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};

use super::{
    checked_recently, record_check, InstallLayout, InstallLock, UpdateCheckState,
    CHECK_STATE_FILE_NAME,
};

/// Creates a unique, empty temp directory for a test.
fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "warp-tui-autoupdate-test-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn layout_for_root(root: &Path) -> InstallLayout {
    InstallLayout::from_canonical_exe_path(
        &root.join("versions/v0.2026.01.01.00.00.dev_00/warp-tui-dev"),
    )
    .unwrap()
}

#[test]
fn detects_managed_install_layout() {
    let exe = Path::new("/home/user/.warp/tui/versions/v0.2026.01.01.00.00.dev_00/warp-tui-dev");
    let layout = InstallLayout::from_canonical_exe_path(exe).unwrap();

    assert_eq!(layout.root, Path::new("/home/user/.warp/tui"));
    assert_eq!(
        layout.versions_dir,
        Path::new("/home/user/.warp/tui/versions")
    );
    assert_eq!(
        layout.current_link,
        Path::new("/home/user/.warp/tui/current")
    );
    assert_eq!(
        layout.running_version_dir,
        Path::new("/home/user/.warp/tui/versions/v0.2026.01.01.00.00.dev_00")
    );
    assert_eq!(layout.binary_name, "warp-tui-dev");
}

#[test]
fn rejects_unmanaged_exe_paths() {
    // Legacy flat install: the binary sits directly under the install root.
    assert_eq!(
        InstallLayout::from_canonical_exe_path(Path::new("/home/user/.warp/tui/warp-tui-dev")),
        None
    );
    // A plain cargo build in target/.
    assert_eq!(
        InstallLayout::from_canonical_exe_path(Path::new("/repo/target/debug/warp-tui-dev")),
        None
    );
}

#[test]
fn check_throttle_round_trips_through_the_state_file() {
    let root = temp_root("throttle");
    let layout = layout_for_root(&root);

    // No state file yet: not throttled.
    assert!(!checked_recently(&layout));

    // A just-recorded check throttles subsequent ones.
    record_check(&layout);
    assert!(checked_recently(&layout));

    // An old check does not.
    let stale = UpdateCheckState {
        last_checked_at: Utc::now() - Duration::days(2),
    };
    fs::write(
        root.join(CHECK_STATE_FILE_NAME),
        serde_json::to_string(&stale).unwrap(),
    )
    .unwrap();
    assert!(!checked_recently(&layout));

    // Corrupt state is treated as "never checked".
    fs::write(root.join(CHECK_STATE_FILE_NAME), "not json").unwrap();
    assert!(!checked_recently(&layout));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn install_lock_is_exclusive_and_released_on_drop() {
    let root = temp_root("lock");

    let lock = InstallLock::acquire(&root).unwrap();
    assert!(lock.is_some());

    // A second acquisition fails while the lock is held.
    assert!(InstallLock::acquire(&root).unwrap().is_none());

    // Dropping the lock releases it.
    drop(lock);
    assert!(InstallLock::acquire(&root).unwrap().is_some());

    let _ = fs::remove_dir_all(&root);
}
