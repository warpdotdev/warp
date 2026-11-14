use tempfile::TempDir;

use super::*;

fn create_bundle(path: &Path, executable_contents: &str) {
    let macos_dir = path.join("Contents/MacOS");
    fs::create_dir_all(&macos_dir).unwrap();
    fs::write(path.join("Contents/Info.plist"), "plist").unwrap();
    fs::write(macos_dir.join("stable"), executable_contents).unwrap();
}

#[test]
fn test_preserve_running_bundle_moves_whole_bundle() {
    let temp_dir = TempDir::new().unwrap();
    let swapped_out_bundle = temp_dir.path().join("Warp(v0.1).app");
    let destination = temp_dir.path().join(OLD_BUNDLE_NAME);
    create_bundle(&swapped_out_bundle, "running");

    future::block_on(preserve_running_bundle(&swapped_out_bundle, &destination)).unwrap();

    assert!(!swapped_out_bundle.exists());
    assert!(destination.join("Contents/Info.plist").is_file());
    assert_eq!(
        fs::read_to_string(destination.join("Contents/MacOS/stable")).unwrap(),
        "running"
    );
}

#[test]
fn test_preserve_running_bundle_keeps_existing_old_bundle() {
    let temp_dir = TempDir::new().unwrap();
    let swapped_out_bundle = temp_dir.path().join("Warp(v0.2).app");
    let destination = temp_dir.path().join(OLD_BUNDLE_NAME);
    create_bundle(&destination, "running");
    create_bundle(&swapped_out_bundle, "not running");

    future::block_on(preserve_running_bundle(&swapped_out_bundle, &destination)).unwrap();

    assert_eq!(
        fs::read_to_string(destination.join("Contents/MacOS/stable")).unwrap(),
        "running"
    );
    assert!(swapped_out_bundle.join("Contents/MacOS/stable").is_file());
}
