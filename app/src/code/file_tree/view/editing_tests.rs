use std::sync::Arc;

use repo_metadata::file_tree_store::{FileTreeDirectoryEntryState, FileTreeEntryState};
use repo_metadata::{FileMetadata, FileTreeEntry};
use warp_util::standardized_path::StandardizedPath;

use super::{
    destination_is_vacant, move_destination, rename_noreplace, sort_entries_for_file_tree,
};

fn std_path(s: &str) -> StandardizedPath {
    StandardizedPath::try_new(s).expect("test path should be valid")
}

fn dir_state(path: &str) -> FileTreeEntryState {
    FileTreeEntryState::Directory(FileTreeDirectoryEntryState {
        path: Arc::new(std_path(path)),
        ignored: false,
        loaded: true,
    })
}

fn file_state(path: &str) -> FileTreeEntryState {
    FileTreeEntryState::File(FileMetadata::from_standardized(std_path(path), false).into())
}

#[test]
fn sort_entries_for_file_tree_is_antisymmetric_for_missing_entries() {
    let root = std_path("/repo");
    let mut entry = FileTreeEntry::new_for_directory(Arc::new(root.clone()));
    entry.insert_child_state(&root, dir_state("/repo/src"));
    entry.insert_child_state(&root, file_state("/repo/README.md"));

    let paths = [
        std_path("/repo/src"),       // present (directory)
        std_path("/repo/README.md"), // present (file)
        std_path("/repo/ghost_a"),   // missing
        std_path("/repo/ghost_b"),   // missing
    ];

    for a in &paths {
        for b in &paths {
            let ab = sort_entries_for_file_tree(a, b, &entry);
            let ba = sort_entries_for_file_tree(b, a, &entry);
            assert_eq!(
                ab.reverse(),
                ba,
                "comparator not antisymmetric for ({}, {}): cmp(a,b) = {:?}, cmp(b,a) = {:?}",
                a.as_str(),
                b.as_str(),
                ab,
                ba,
            );
        }
    }
}

#[test]
fn sort_entries_for_file_tree_sorts_without_panicking_on_missing_children() {
    let root = std_path("/repo");
    let mut entry = FileTreeEntry::new_for_directory(Arc::new(root.clone()));
    entry.insert_child_state(&root, dir_state("/repo/src"));

    // Multiple missing entries are required to reliably trigger the sort's
    // total-order violation check.
    let mut paths = [
        std_path("/repo/src"),
        std_path("/repo/ghost_a"),
        std_path("/repo/ghost_b"),
        std_path("/repo/ghost_c"),
        std_path("/repo/ghost_d"),
        std_path("/repo/ghost_e"),
    ];

    paths.sort_by(|a, b| sort_entries_for_file_tree(a, b, &entry));
}

#[test]
fn sort_entries_for_file_tree_uses_natural_order_for_numbered_files() {
    let root = std_path("/repo");
    let mut entry = FileTreeEntry::new_for_directory(Arc::new(root.clone()));
    for name in ["L1", "L2", "L3", "L10", "L11", "L12"] {
        entry.insert_child_state(&root, file_state(&format!("/repo/{name}.tsx")));
    }

    let mut paths = [
        std_path("/repo/L10.tsx"),
        std_path("/repo/L2.tsx"),
        std_path("/repo/L1.tsx"),
        std_path("/repo/L12.tsx"),
        std_path("/repo/L3.tsx"),
        std_path("/repo/L11.tsx"),
    ];
    paths.sort_by(|a, b| sort_entries_for_file_tree(a, b, &entry));

    let sorted: Vec<&str> = paths.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        sorted,
        [
            "/repo/L1.tsx",
            "/repo/L2.tsx",
            "/repo/L3.tsx",
            "/repo/L10.tsx",
            "/repo/L11.tsx",
            "/repo/L12.tsx",
        ]
    );
}

#[cfg(unix)]
#[test]
fn dangling_symlink_destination_is_occupied() {
    use std::os::unix::fs::symlink;

    let temp_dir = tempfile::tempdir().unwrap();
    let source = temp_dir.path().join("source.txt");
    let destination = temp_dir.path().join("destination.txt");
    std::fs::write(&source, "source").unwrap();
    symlink(temp_dir.path().join("missing.txt"), &destination).unwrap();

    assert!(!destination_is_vacant(&destination));
    assert!(rename_noreplace(&source, &destination).is_err());
    assert_eq!(std::fs::read_to_string(source).unwrap(), "source");
    assert!(std::fs::symlink_metadata(destination).is_ok());
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn atomic_move_does_not_overwrite_racing_destination() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source = temp_dir.path().join("source.txt");
    let destination = temp_dir.path().join("destination.txt");
    std::fs::write(&source, "source").unwrap();

    assert!(destination_is_vacant(&destination));
    std::fs::write(&destination, "racing destination").unwrap();

    assert!(rename_noreplace(&source, &destination).is_err());
    assert_eq!(std::fs::read_to_string(source).unwrap(), "source");
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "racing destination"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn case_only_rename_succeeds_without_leaving_temporary_entries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source = temp_dir.path().join("Source.txt");
    let destination = temp_dir.path().join("source.txt");
    std::fs::write(&source, "source").unwrap();

    rename_noreplace(&source, &destination).unwrap();

    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "source");
    let names: Vec<_> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [destination.file_name().unwrap()]);
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn case_only_dangling_symlink_rename_preserves_link_target() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source = temp_dir.path().join("SourceLink");
    let destination = temp_dir.path().join("sourcelink");
    let link_target = std::path::Path::new("MissingTarget");

    #[cfg(unix)]
    std::os::unix::fs::symlink(link_target, &source).unwrap();
    #[cfg(target_os = "windows")]
    std::os::windows::fs::symlink_file(link_target, &source).unwrap();

    rename_noreplace(&source, &destination).unwrap();

    assert_eq!(std::fs::read_link(&destination).unwrap(), link_target);
    let names: Vec<_> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [destination.file_name().unwrap()]);
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn case_only_rename_supports_near_name_max_source() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_name = format!("S{}.txt", "a".repeat(250));
    let destination_name = format!("s{}.txt", "a".repeat(250));
    let source = temp_dir.path().join(source_name);
    let destination = temp_dir.path().join(destination_name);
    std::fs::write(&source, "source").unwrap();

    rename_noreplace(&source, &destination).unwrap();

    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "source");
    let names: Vec<_> = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [destination.file_name().unwrap()]);
}
#[test]
fn move_destination_preserves_file_name() {
    assert_eq!(
        move_destination(&std_path("/repo/src/main.rs"), &std_path("/repo/tests")),
        Some(std_path("/repo/tests/main.rs"))
    );
}

#[test]
fn move_destination_rejects_current_parent() {
    assert_eq!(
        move_destination(&std_path("/repo/src/main.rs"), &std_path("/repo/src")),
        None
    );
}

#[test]
fn move_destination_rejects_item_and_descendants() {
    let source = std_path("/repo/src");

    assert_eq!(move_destination(&source, &source), None);
    assert_eq!(
        move_destination(&source, &std_path("/repo/src/nested")),
        None
    );
}
