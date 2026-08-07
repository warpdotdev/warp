use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;
use tempfile::TempDir;

use super::*;
use crate::paths::factory_config_file_path;

/// Serializes the `$HOME`-mutating test so the real path helpers resolve under a
/// throwaway home without racing sibling tests in the same process.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn config_path_in(dir: &TempDir) -> PathBuf {
    dir.path().join("factory").join("config.json")
}

fn read_json(path: &std::path::Path) -> Value {
    let contents = std::fs::read_to_string(path).expect("config file should exist");
    serde_json::from_str(&contents).expect("config file should be valid JSON")
}

#[test]
fn set_then_read_round_trips_the_uid_verbatim() {
    let dir = TempDir::new().expect("temp dir");
    let path = config_path_in(&dir);

    set_default_at(&path, "fac_abc123", Some("Acme Backend")).expect("write succeeds");

    let resolved = resolve_at(&path)
        .expect("read succeeds")
        .expect("a default is set");
    assert_eq!(resolved.uid, "fac_abc123");
    assert_eq!(resolved.name.as_deref(), Some("Acme Backend"));

    // The uid is persisted verbatim on disk, not just in the returned value.
    assert_eq!(read_json(&path)["default_factory_uid"], "fac_abc123");
}

#[test]
fn writes_preserve_unknown_keys() {
    let dir = TempDir::new().expect("temp dir");
    let path = config_path_in(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Seed a file with keys this version does not own, including a nested object.
    std::fs::write(
        &path,
        r#"{"default_factory_uid":"old","theme":"dark","future_pref":{"nested":true}}"#,
    )
    .unwrap();

    set_default_at(&path, "new_uid", Some("New Name")).expect("write succeeds");

    let after_set = read_json(&path);
    assert_eq!(after_set["default_factory_uid"], "new_uid");
    assert_eq!(after_set["default_factory_name"], "New Name");
    // The unrecognized keys must survive the read-modify-write unchanged.
    assert_eq!(after_set["theme"], "dark");
    assert_eq!(
        after_set["future_pref"],
        serde_json::json!({"nested": true})
    );

    // Clearing must also preserve the unknown keys while dropping the default.
    clear_default_at(&path).expect("clear succeeds");
    let after_clear = read_json(&path);
    assert!(after_clear.get("default_factory_uid").is_none());
    assert!(after_clear.get("default_factory_name").is_none());
    assert_eq!(after_clear["theme"], "dark");
    assert_eq!(
        after_clear["future_pref"],
        serde_json::json!({"nested": true})
    );
}

#[test]
fn malformed_file_is_surfaced_and_never_destroyed() {
    let dir = TempDir::new().expect("temp dir");
    let path = config_path_in(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let garbage = "{ this is not valid json ]]";
    std::fs::write(&path, garbage).unwrap();

    // Read/resolve surfaces the malformed state rather than silently ignoring it.
    assert!(matches!(
        resolve_at(&path),
        Err(FactoryConfigError::Malformed { .. })
    ));

    // A set must refuse rather than clobber a hand-edited (possibly typo'd) file.
    assert!(matches!(
        set_default_at(&path, "fac_x", None),
        Err(FactoryConfigError::Malformed { .. })
    ));
    assert!(matches!(
        clear_default_at(&path),
        Err(FactoryConfigError::Malformed { .. })
    ));

    // The bytes on disk are exactly what the user left there.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), garbage);
}

#[test]
fn absent_file_is_a_silent_no_op() {
    let dir = TempDir::new().expect("temp dir");
    let path = config_path_in(&dir);

    assert_eq!(resolve_at(&path).expect("read succeeds"), None);
    // Clearing an absent default creates nothing.
    clear_default_at(&path).expect("clear is a no-op");
    assert!(!path.exists(), "no file should be created as a side effect");
}

#[test]
fn advisory_name_never_participates_in_resolution() {
    let dir = TempDir::new().expect("temp dir");
    let path = config_path_in(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    // A stale/contradictory name must not override the authoritative uid.
    std::fs::write(
        &path,
        r#"{"default_factory_uid":"fac_real","default_factory_name":"Totally Different Factory"}"#,
    )
    .unwrap();
    let resolved = resolve_at(&path).unwrap().expect("uid resolves");
    assert_eq!(resolved.uid, "fac_real");

    // A name with no (or empty) uid cannot conjure a default.
    std::fs::write(&path, r#"{"default_factory_name":"Only A Name"}"#).unwrap();
    assert_eq!(resolve_at(&path).unwrap(), None);
    std::fs::write(
        &path,
        r#"{"default_factory_uid":"","default_factory_name":"Only A Name"}"#,
    )
    .unwrap();
    assert_eq!(resolve_at(&path).unwrap(), None);
}

#[test]
fn real_config_path_round_trips_at_the_channel_aware_location_under_temp_home() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let home = TempDir::new().expect("temp home");
    let previous_home = std::env::var_os("HOME");
    // SAFETY: the lock serializes env mutation and HOME is restored below.
    unsafe {
        std::env::set_var("HOME", home.path());
    }

    let path = factory_config_file_path().expect("path resolves");
    // Only exercise the real write when the HOME override is actually honored, so
    // this can never touch a developer's real ~/.warp* directory.
    if path.starts_with(home.path()) {
        assert_eq!(path.file_name().unwrap(), "config.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "factory");

        assert_eq!(resolve_default().expect("read succeeds"), None);
        set_default("fac_home", Some("Home Factory")).expect("write succeeds");
        assert!(
            path.exists(),
            "the file lands at the resolved real location"
        );
        let resolved = resolve_default().unwrap().expect("default is set");
        assert_eq!(resolved.uid, "fac_home");
        clear_default().expect("clear succeeds");
        assert_eq!(resolve_default().unwrap(), None);
    }

    // SAFETY: restore the prior HOME while still holding the lock.
    unsafe {
        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}
