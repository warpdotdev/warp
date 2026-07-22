//! Tests for the size-based rotation helpers in `lib.rs`.
//!
//! The high-level `SimpleLogger::new` path runs in a background executor and
//! interleaves async file I/O with channel reads; integration coverage at that
//! layer is provided by [`crate::manager`]'s tests. The cases here exercise the
//! pure file-shuffling helpers (`perform_rotation`, `path_with_suffix`) and the
//! `RotationConfig` constructor, all of which are deterministic and don't
//! require an executor — keeping the unit tests fast and avoiding the
//! flakiness that comes with background-task synchronization.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{RotationConfig, path_with_suffix, perform_rotation};

/// Unique temp directory per test, so parallel cargo nextest runs don't
/// collide on shared state.
fn temp_dir(name: &str) -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "simple-logger-rotation-{name}-{}-{id}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("failed to create test temp dir");
    dir
}

fn write_file(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("failed to write test fixture file");
}

fn read_to_string(path: &Path) -> String {
    fs::read_to_string(path).expect("failed to read file")
}

// ---------- RotationConfig ----------

#[test]
fn rotation_config_zero_max_size_disables() {
    assert!(RotationConfig::new(0, 5).is_none());
}

#[test]
fn rotation_config_zero_max_rotation_disables() {
    assert!(RotationConfig::new(10 * 1024 * 1024, 0).is_none());
}

#[test]
fn rotation_config_both_zero_disables() {
    assert!(RotationConfig::new(0, 0).is_none());
}

#[test]
fn rotation_config_positive_values_construct() {
    let c = RotationConfig::new(1024, 5).expect("positive values should construct");
    assert_eq!(c.max_file_size_bytes(), 1024);
    assert_eq!(c.max_rotation(), 5);
}

// ---------- path_with_suffix ----------

#[test]
fn path_with_suffix_appends_dot_n_without_replacing_extension() {
    let base = Path::new("/tmp/foo/bar.log");
    let p = path_with_suffix(base, 3);
    assert_eq!(p, PathBuf::from("/tmp/foo/bar.log.3"));
}

#[test]
fn path_with_suffix_preserves_compound_extensions() {
    let base = Path::new("/tmp/foo/server.stderr.log");
    let p = path_with_suffix(base, 1);
    // Crucially this must be `server.stderr.log.1` — not `server.stderr.1`
    // (which is what `set_extension` would produce).
    assert_eq!(p, PathBuf::from("/tmp/foo/server.stderr.log.1"));
}

#[test]
fn path_with_suffix_handles_no_extension() {
    let base = Path::new("/tmp/foo/logfile");
    let p = path_with_suffix(base, 7);
    assert_eq!(p, PathBuf::from("/tmp/foo/logfile.7"));
}

// ---------- perform_rotation: file-level behavior ----------

/// The base case: there's only an active file at `base_path`. After rotation,
/// the active file should be gone (renamed to `.1`) and no other files should
/// exist.
#[tokio::test]
async fn rotate_promotes_active_file_to_dot_one_when_no_prior_rotations() {
    let dir = temp_dir("first-rotation");
    let base = dir.join("server.log");
    write_file(&base, b"hello\n");

    perform_rotation(&base, 5)
        .await
        .expect("rotation should succeed");

    assert!(!base.exists(), "active file must be gone after rotation");
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "hello\n");
    assert!(!path_with_suffix(&base, 2).exists());
}

/// With one prior rotated file, the active becomes `.1` and the existing `.1`
/// shifts to `.2`. Verifies the iteration goes from oldest to youngest (so we
/// don't clobber an unmoved file).
#[tokio::test]
async fn rotate_shifts_prior_rotations_up_by_one() {
    let dir = temp_dir("shift");
    let base = dir.join("srv.log");
    write_file(&base, b"current\n");
    write_file(&path_with_suffix(&base, 1), b"previous\n");

    perform_rotation(&base, 5)
        .await
        .expect("rotation should succeed");

    assert!(!base.exists());
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "current\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 2)), "previous\n");
    assert!(!path_with_suffix(&base, 3).exists());
}

/// At the rotation cap, the oldest rotated file (`.max_rotation`) is deleted
/// before the shift, so no file ages past `.max_rotation`. This is the
/// "automatic cleanup" property the bug-fix is meant to provide.
#[tokio::test]
async fn rotate_discards_oldest_file_when_at_cap() {
    let dir = temp_dir("cap");
    let base = dir.join("server.log");
    write_file(&base, b"current\n");
    for n in 1..=3 {
        write_file(
            &path_with_suffix(&base, n),
            format!("rotated-{n}\n").as_bytes(),
        );
    }

    perform_rotation(&base, 3)
        .await
        .expect("rotation should succeed");

    // `.3` was the oldest and is gone (its contents — "rotated-3" — are not
    // preserved anywhere). `.1` became `.2`, `.2` became `.3`, and the
    // original active file is at `.1`.
    assert!(!base.exists());
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "current\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 2)), "rotated-1\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 3)), "rotated-2\n");
    assert!(!path_with_suffix(&base, 4).exists());
}

/// With `max_rotation = 1`, the only rotated slot is `.1`. Each rotation
/// overwrites `.1` with the latest active contents and discards the prior
/// `.1`. Verifies the minimum-rotation edge case doesn't off-by-one.
#[tokio::test]
async fn rotate_with_max_rotation_one_overwrites_dot_one() {
    let dir = temp_dir("max-one");
    let base = dir.join("server.log");
    write_file(&base, b"second\n");
    write_file(&path_with_suffix(&base, 1), b"first\n");

    perform_rotation(&base, 1)
        .await
        .expect("rotation should succeed");

    assert!(!base.exists());
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "second\n");
    // The previous `.1` ("first") has been displaced and discarded; the cap is
    // `max_rotation = 1`, so there's no `.2`.
    assert!(!path_with_suffix(&base, 2).exists());
}

/// Missing intermediate rotated files (e.g. the user hasn't accumulated enough
/// rotations to fill every slot) must not cause the rotation to error. Only the
/// rename of the active file is fatal.
#[tokio::test]
async fn rotate_tolerates_missing_intermediate_files() {
    let dir = temp_dir("sparse");
    let base = dir.join("server.log");
    write_file(&base, b"current\n");
    // Skip `.1`, `.2`, `.3`; only `.4` exists.
    write_file(&path_with_suffix(&base, 4), b"old\n");

    perform_rotation(&base, 5)
        .await
        .expect("rotation should succeed despite gaps");

    assert!(!base.exists());
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "current\n");
    // `.4` shifts to `.5`. `.2`, `.3`, `.4` remain empty/missing.
    assert!(!path_with_suffix(&base, 2).exists());
    assert!(!path_with_suffix(&base, 3).exists());
    assert!(!path_with_suffix(&base, 4).exists());
    assert_eq!(read_to_string(&path_with_suffix(&base, 5)), "old\n");
}

/// Rotation when the active file doesn't exist (e.g. caller invoked rotation
/// preemptively before any writes) should not error — there's simply nothing
/// to promote. Existing rotated files still shift.
#[tokio::test]
async fn rotate_no_op_when_active_file_missing_but_still_shifts_rotated() {
    let dir = temp_dir("no-active");
    let base = dir.join("server.log");
    // No active file, but two rotated files exist.
    write_file(&path_with_suffix(&base, 1), b"one\n");
    write_file(&path_with_suffix(&base, 2), b"two\n");

    perform_rotation(&base, 5)
        .await
        .expect("rotation should succeed with no active file");

    assert!(!base.exists());
    // Without an active file, `.1` is empty (the shift moved `.1` -> `.2` and
    // `.2` -> `.3`; nothing populated `.1`).
    assert!(!path_with_suffix(&base, 1).exists());
    assert_eq!(read_to_string(&path_with_suffix(&base, 2)), "one\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 3)), "two\n");
}

/// After three consecutive rotations, the in-use file's contents propagate to
/// `.3` and the eldest contents from the first rotation are gone. This
/// exercises the iteration-direction property: a left-to-right loop would
/// clobber files mid-shift.
#[tokio::test]
async fn three_consecutive_rotations_preserve_order_and_discard_oldest() {
    let dir = temp_dir("three-rotations");
    let base = dir.join("server.log");

    write_file(&base, b"v1\n");
    perform_rotation(&base, 3).await.unwrap();
    // After rotation 1: .1 = v1
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "v1\n");

    write_file(&base, b"v2\n");
    perform_rotation(&base, 3).await.unwrap();
    // After rotation 2: .1 = v2, .2 = v1
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "v2\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 2)), "v1\n");

    write_file(&base, b"v3\n");
    perform_rotation(&base, 3).await.unwrap();
    // After rotation 3: .1 = v3, .2 = v2, .3 = v1
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "v3\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 2)), "v2\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 3)), "v1\n");

    write_file(&base, b"v4\n");
    perform_rotation(&base, 3).await.unwrap();
    // After rotation 4: v1 is discarded; .1=v4, .2=v3, .3=v2
    assert_eq!(read_to_string(&path_with_suffix(&base, 1)), "v4\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 2)), "v3\n");
    assert_eq!(read_to_string(&path_with_suffix(&base, 3)), "v2\n");
    assert!(!path_with_suffix(&base, 4).exists());
}
