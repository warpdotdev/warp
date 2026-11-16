use std::fs;
use std::path::Path;

use super::migrate_config_dir_via_symlinks;

fn is_symlink_to(link: &Path, expected_target: &Path) -> bool {
    match fs::read_link(link) {
        Ok(target) => target == expected_target,
        Err(_) => false,
    }
}

#[test]
fn creates_symlinks_for_top_level_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let old_dir = tmp.path().join(".warp");
    let new_dir = tmp.path().join(".warp-preview");

    fs::create_dir(&old_dir).unwrap();
    fs::write(old_dir.join("keybindings.yaml"), "bindings").unwrap();
    fs::create_dir(old_dir.join("themes")).unwrap();
    fs::write(old_dir.join("themes").join("dark.yaml"), "theme").unwrap();

    migrate_config_dir_via_symlinks(&old_dir, &new_dir);

    assert!(new_dir.is_dir());
    assert!(is_symlink_to(
        &new_dir.join("keybindings.yaml"),
        &old_dir.join("keybindings.yaml"),
    ));
    assert!(is_symlink_to(
        &new_dir.join("themes"),
        &old_dir.join("themes"),
    ));
    // Content is reachable through the symlink.
    assert_eq!(
        fs::read_to_string(new_dir.join("keybindings.yaml")).unwrap(),
        "bindings",
    );
}

#[test]
fn skips_when_new_dir_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let old_dir = tmp.path().join(".warp");
    let new_dir = tmp.path().join(".warp-preview");

    fs::create_dir(&old_dir).unwrap();
    fs::write(old_dir.join("keybindings.yaml"), "bindings").unwrap();
    fs::create_dir(&new_dir).unwrap();

    migrate_config_dir_via_symlinks(&old_dir, &new_dir);

    // new_dir should remain empty — no symlinks created.
    assert_eq!(fs::read_dir(&new_dir).unwrap().count(), 0);
}

#[test]
fn skips_when_old_dir_does_not_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let old_dir = tmp.path().join(".warp");
    let new_dir = tmp.path().join(".warp-preview");

    // old_dir intentionally not created.
    migrate_config_dir_via_symlinks(&old_dir, &new_dir);

    assert!(!new_dir.exists());
}

#[test]
fn skips_ds_store_and_dot_underscore_files() {
    let tmp = tempfile::tempdir().unwrap();
    let old_dir = tmp.path().join(".warp");
    let new_dir = tmp.path().join(".warp-preview");

    fs::create_dir(&old_dir).unwrap();
    fs::write(old_dir.join(".DS_Store"), "metadata").unwrap();
    fs::write(old_dir.join("._somefile"), "resource fork").unwrap();
    fs::write(old_dir.join("keybindings.yaml"), "bindings").unwrap();

    migrate_config_dir_via_symlinks(&old_dir, &new_dir);

    assert!(!new_dir.join(".DS_Store").exists());
    assert!(!new_dir.join("._somefile").exists());
    assert!(new_dir.join("keybindings.yaml").exists());
}

#[test]
fn skips_excluded_files() {
    let tmp = tempfile::tempdir().unwrap();
    let old_dir = tmp.path().join(".warp");
    let new_dir = tmp.path().join(".warp-preview");

    fs::create_dir(&old_dir).unwrap();
    fs::write(old_dir.join("settings.toml"), "[settings]").unwrap();
    fs::write(old_dir.join("keybindings.yaml"), "bindings").unwrap();

    migrate_config_dir_via_symlinks(&old_dir, &new_dir);

    assert!(!new_dir.join("settings.toml").exists());
    assert!(new_dir.join("keybindings.yaml").exists());
}

#[test]
fn is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let old_dir = tmp.path().join(".warp");
    let new_dir = tmp.path().join(".warp-preview");

    fs::create_dir(&old_dir).unwrap();
    fs::write(old_dir.join("keybindings.yaml"), "bindings").unwrap();

    migrate_config_dir_via_symlinks(&old_dir, &new_dir);
    // Second call should be a no-op (new_dir already exists).
    migrate_config_dir_via_symlinks(&old_dir, &new_dir);

    assert_eq!(fs::read_dir(&new_dir).unwrap().count(), 1);
}
