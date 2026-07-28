use std::path::{Path, PathBuf};
#[allow(clippy::disallowed_types)]
use std::process::Child;
use std::time::Duration;
use std::{fs, thread};

use command::blocking::Command;
use instant::Instant;
use warp_core::channel::Channel;

use super::{
    CURRENT_LINK_NAME, InstallLayout, InstallLock, LOCK_FILE_NAME, LOCK_OWNER_FILE_NAME,
    VERSION_LEASES_DIR_NAME, VersionDirState, VersionLease, create_unique_staging_dir_with,
    download_endpoint, is_complete_version_dir, version_dir_state,
};
#[cfg(unix)]
use super::{
    StagedUpdate, finalize_staged_version, install_update, point_current_at, prune_old_versions,
};

const BINARY_NAME: &str = "warp-tui-dev";
const HELPER_MODE_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_MODE";
const HELPER_ROOT_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_ROOT";
const HELPER_VERSION_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_VERSION";
const HELPER_READY_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_READY";
const HELPER_RELEASE_ENV: &str = "WARP_TUI_AUTOUPDATE_HELPER_RELEASE";

fn temp_root(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("warp-tui-autoupdate-{name}-"))
        .tempdir()
        .unwrap()
}

fn layout(root: &Path, running_version: &str) -> InstallLayout {
    InstallLayout {
        root: root.to_path_buf(),
        versions_dir: root.join("versions"),
        current_link: root.join(CURRENT_LINK_NAME),
        running_version_dir: root.join("versions").join(running_version),
        binary_name: BINARY_NAME.to_owned(),
    }
}

fn create_complete_version(root: &Path, version: &str, contents: &str) -> PathBuf {
    let version_dir = root.join("versions").join(version);
    fs::create_dir_all(version_dir.join("resources")).unwrap();
    fs::write(version_dir.join(BINARY_NAME), contents).unwrap();
    fs::write(version_dir.join("resources").join("marker"), contents).unwrap();
    version_dir
}

fn lease_path(root: &Path, version: &str) -> PathBuf {
    root.join(VERSION_LEASES_DIR_NAME)
        .join(format!("{version}.lock"))
}

fn wait_for_contents(path: &Path, expected: &str, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if fs::read_to_string(path).is_ok_and(|contents| contents == expected) {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("helper exited before writing {expected:?} to {path:?}: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected:?} in {path:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_helper(
    mode: &str,
    root: &Path,
    version: &str,
    ready: &Path,
    release: Option<&Path>,
) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("lease_process_helper")
        .arg("--nocapture")
        .env(HELPER_MODE_ENV, mode)
        .env(HELPER_ROOT_ENV, root)
        .env(HELPER_VERSION_ENV, version)
        .env(HELPER_READY_ENV, ready);
    if let Some(release) = release {
        command.env(HELPER_RELEASE_ENV, release);
    }
    command.spawn().unwrap()
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
    assert_eq!(layout.binary_name, BINARY_NAME);
}

#[test]
fn rejects_unmanaged_exe_paths() {
    assert_eq!(
        InstallLayout::from_canonical_exe_path(Path::new("/home/user/.warp/tui/warp-tui-dev")),
        None
    );
    assert_eq!(
        InstallLayout::from_canonical_exe_path(Path::new("/repo/target/debug/warp-tui-dev")),
        None
    );
    assert!(
        VersionLease::acquire_for_current_process()
            .unwrap()
            .is_none()
    );
}

#[test]
fn uses_channel_specific_download_endpoints() {
    assert_eq!(
        download_endpoint(Channel::Stable),
        "/download/agent-cli/artifact"
    );
    assert_eq!(
        download_endpoint(Channel::Preview),
        "/download/agent-cli-preview/artifact"
    );
    assert_eq!(
        download_endpoint(Channel::Dev),
        "/download/agent-cli-dev/artifact"
    );
}

#[test]
fn complete_versions_require_real_binary_and_resources() {
    let root = temp_root("complete");
    let layout = layout(root.path(), "A");
    let version_dir = create_complete_version(root.path(), "A", "original");
    assert!(is_complete_version_dir(&layout, &version_dir));
    assert_eq!(
        version_dir_state(&layout, &version_dir).unwrap(),
        VersionDirState::Complete
    );

    fs::remove_dir_all(version_dir.join("resources")).unwrap();
    assert_eq!(
        version_dir_state(&layout, &version_dir).unwrap(),
        VersionDirState::Invalid
    );
    assert_eq!(
        fs::read_to_string(version_dir.join(BINARY_NAME)).unwrap(),
        "original"
    );
}

#[test]
fn staging_allocation_skips_existing_directory_without_reusing_it() {
    let root = temp_root("staging-collision");
    let versions_dir = root.path().join("versions");
    fs::create_dir(&versions_dir).unwrap();
    let stale = versions_dir.join(".staging-stale");
    let fresh = versions_dir.join(".staging-fresh");
    fs::create_dir(&stale).unwrap();
    fs::write(stale.join("marker"), "stale").unwrap();
    let mut candidates = [stale.clone(), fresh.clone()].into_iter();

    let allocated = futures::executor::block_on(create_unique_staging_dir_with(|| {
        candidates.next().unwrap()
    }))
    .unwrap();

    assert_eq!(allocated, fresh);
    assert_eq!(fs::read_to_string(stale.join("marker")).unwrap(), "stale");
    assert!(allocated.is_dir());
}

#[test]
fn managed_version_lease_creates_stable_marker() {
    let root = temp_root("lease");
    let version = "v0.2026.07.22.18.00.dev_00";
    let layout = layout(root.path(), version);
    create_complete_version(root.path(), version, "A");

    let lease = VersionLease::acquire(&layout).unwrap();
    assert!(lease_path(root.path(), version).is_file());
    assert!(layout.running_version_dir.is_dir());
    drop(lease);
    assert!(lease_path(root.path(), version).is_file());
}

#[cfg(unix)]
#[test]
fn finalized_unlaunched_version_is_marked_and_reclaimed() {
    let root = temp_root("finalized-marker");
    let layout = layout(root.path(), "A");
    create_complete_version(root.path(), "A", "A");
    point_current_at(&layout, "A").unwrap();

    let staging_dir = root.path().join("versions/.staging-B");
    let payload_dir = staging_dir.join("payload");
    fs::create_dir_all(payload_dir.join("resources")).unwrap();
    fs::write(payload_dir.join(BINARY_NAME), "B").unwrap();
    fs::write(payload_dir.join("resources/marker"), "B").unwrap();
    let staged = StagedUpdate {
        staging_dir,
        payload_dir,
    };
    let version_dir = root.path().join("versions/B");

    finalize_staged_version(&layout, "B", staged, &version_dir).unwrap();
    assert!(lease_path(root.path(), "B").is_file());

    create_complete_version(root.path(), "C", "C");
    point_current_at(&layout, "C").unwrap();
    prune_old_versions(&layout, "C");
    assert!(!version_dir.exists());
}

#[cfg(unix)]
#[test]
fn completed_version_is_reused_and_invalid_version_is_not_replaced() {
    let root = temp_root("immutable");
    create_complete_version(root.path(), "A", "running");
    let target = create_complete_version(root.path(), "C", "original");
    let layout = layout(root.path(), "A");
    point_current_at(&layout, "A").unwrap();

    futures::executor::block_on(install_update(layout.clone(), "C".to_owned())).unwrap();
    assert_eq!(
        fs::read_to_string(target.join(BINARY_NAME)).unwrap(),
        "original"
    );
    assert_eq!(
        fs::read_to_string(target.join("resources/marker")).unwrap(),
        "original"
    );

    fs::remove_dir_all(&target).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join(BINARY_NAME), "partial").unwrap();
    point_current_at(&layout, "A").unwrap();
    let error =
        futures::executor::block_on(install_update(layout.clone(), "C".to_owned())).unwrap_err();
    assert!(format!("{error:#}").contains("refusing to replace incomplete or invalid"));
    assert_eq!(
        fs::read_to_string(target.join(BINARY_NAME)).unwrap(),
        "partial"
    );
    assert!(super::current_points_at(&layout, "A"));
}

#[cfg(unix)]
#[test]
fn live_versions_are_retained_and_reclaimed_after_exit() {
    let root = temp_root("gc");
    create_complete_version(root.path(), "A", "A");
    create_complete_version(root.path(), "B", "B");
    create_complete_version(root.path(), "C", "C");
    create_complete_version(root.path(), "legacy", "legacy");
    let layout = layout(root.path(), "C");
    point_current_at(&layout, "C").unwrap();

    let a_ready = root.path().join("a-ready");
    let a_release = root.path().join("a-release");
    let b_ready = root.path().join("b-ready");
    let b_release = root.path().join("b-release");
    let mut a = spawn_helper("hold-lease", root.path(), "A", &a_ready, Some(&a_release));
    let mut b = spawn_helper("hold-lease", root.path(), "B", &b_ready, Some(&b_release));
    wait_for_contents(&a_ready, "locked", &mut a);
    wait_for_contents(&b_ready, "locked", &mut b);

    prune_old_versions(&layout, "C");
    assert!(root.path().join("versions/A").is_dir());
    assert!(root.path().join("versions/B").is_dir());
    assert!(root.path().join("versions/C").is_dir());
    assert!(root.path().join("versions/legacy").is_dir());

    fs::write(&a_release, "").unwrap();
    assert!(a.wait().unwrap().success());
    prune_old_versions(&layout, "C");
    assert!(!root.path().join("versions/A").exists());
    assert!(lease_path(root.path(), "A").is_file());
    assert!(root.path().join("versions/B").is_dir());

    fs::write(&b_release, "").unwrap();
    assert!(b.wait().unwrap().success());
    prune_old_versions(&layout, "C");
    assert!(!root.path().join("versions/B").exists());
    assert!(root.path().join("versions/C").is_dir());
    assert!(root.path().join("versions/legacy").is_dir());
}

#[cfg(unix)]
#[test]
fn current_version_is_rechecked_before_gc_deletion() {
    let root = temp_root("gc-current");
    create_complete_version(root.path(), "A", "A");
    create_complete_version(root.path(), "C", "C");
    fs::create_dir_all(root.path().join(VERSION_LEASES_DIR_NAME)).unwrap();
    fs::write(lease_path(root.path(), "A"), "").unwrap();
    let layout = layout(root.path(), "C");
    point_current_at(&layout, "A").unwrap();

    prune_old_versions(&layout, "C");
    assert!(root.path().join("versions/A").is_dir());
}

#[test]
fn startup_fails_closed_if_gc_wins_the_lease_race() {
    let root = temp_root("gc-wins");
    let version_dir = create_complete_version(root.path(), "A", "A");
    fs::create_dir_all(root.path().join(VERSION_LEASES_DIR_NAME)).unwrap();
    let lease_path = lease_path(root.path(), "A");
    let lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lease_path)
        .unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&lease).unwrap();

    let ready = root.path().join("race-result");
    let mut child = spawn_helper("attempt-lease", root.path(), "A", &ready, None);
    wait_for_contents(&ready, "starting", &mut child);
    fs::remove_dir_all(version_dir).unwrap();
    fs4::fs_std::FileExt::unlock(&lease).unwrap();
    drop(lease);

    wait_for_contents(&ready, "error", &mut child);
    assert!(child.wait().unwrap().success());
}

#[test]
fn directory_install_lock_is_cross_process_and_token_owned() {
    let root = temp_root("install-lock");
    let ready = root.path().join("lock-ready");
    let release = root.path().join("lock-release");
    let mut child = spawn_helper(
        "hold-install-lock",
        root.path(),
        "unused",
        &ready,
        Some(&release),
    );
    wait_for_contents(&ready, "locked", &mut child);
    assert!(InstallLock::acquire(root.path()).unwrap().is_none());

    fs::write(&release, "").unwrap();
    assert!(child.wait().unwrap().success());
    let lock = InstallLock::acquire(root.path()).unwrap().unwrap();
    fs::write(
        root.path().join(LOCK_FILE_NAME).join(LOCK_OWNER_FILE_NAME),
        "successor",
    )
    .unwrap();
    drop(lock);
    assert!(root.path().join(LOCK_FILE_NAME).is_dir());
}

#[test]
fn install_lock_migrates_stale_legacy_file_and_directory() {
    for representation in ["file", "directory"] {
        let root = temp_root(representation);
        let lock_path = root.path().join(LOCK_FILE_NAME);
        if representation == "file" {
            fs::write(&lock_path, "legacy").unwrap();
        } else {
            fs::create_dir(&lock_path).unwrap();
            fs::write(lock_path.join(LOCK_OWNER_FILE_NAME), "stale").unwrap();
        }

        let lock = InstallLock::acquire_with_stale_age(root.path(), Duration::ZERO)
            .unwrap()
            .unwrap();
        assert!(lock_path.is_dir());
        assert_ne!(
            fs::read_to_string(lock_path.join(LOCK_OWNER_FILE_NAME)).unwrap(),
            "stale"
        );
        drop(lock);
        assert!(!lock_path.exists());
    }
}

#[test]
fn fresh_legacy_install_lock_is_contention() {
    let root = temp_root("fresh-legacy-lock");
    fs::write(root.path().join(LOCK_FILE_NAME), "legacy").unwrap();
    assert!(InstallLock::acquire(root.path()).unwrap().is_none());
}

#[test]
fn lease_process_helper() {
    let Ok(mode) = std::env::var(HELPER_MODE_ENV) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os(HELPER_ROOT_ENV).unwrap());
    let version = std::env::var(HELPER_VERSION_ENV).unwrap();
    let ready = PathBuf::from(std::env::var_os(HELPER_READY_ENV).unwrap());

    match mode.as_str() {
        "hold-lease" => {
            let lease = VersionLease::acquire(&layout(&root, &version)).unwrap();
            fs::write(&ready, "locked").unwrap();
            let release = PathBuf::from(std::env::var_os(HELPER_RELEASE_ENV).unwrap());
            while !release.exists() {
                thread::sleep(Duration::from_millis(10));
            }
            drop(lease);
        }
        "attempt-lease" => {
            fs::write(&ready, "starting").unwrap();
            let result = VersionLease::acquire(&layout(&root, &version));
            fs::write(&ready, if result.is_ok() { "acquired" } else { "error" }).unwrap();
        }
        "hold-install-lock" => {
            let lock = InstallLock::acquire(&root).unwrap().unwrap();
            fs::write(&ready, "locked").unwrap();
            let release = PathBuf::from(std::env::var_os(HELPER_RELEASE_ENV).unwrap());
            while !release.exists() {
                thread::sleep(Duration::from_millis(10));
            }
            drop(lock);
        }
        mode => panic!("unknown helper mode {mode}"),
    }
}
