//! Tests for the file tree snapshot module.

use std::path::Path;
use std::sync::Arc;

use sum_tree::Item;

use super::*;

/// Helper to create a test snapshot from a list of path strings.
/// Paths ending in '/' are treated as directories, others as files.
/// Paths starting with '!' are marked as ignored.
fn test_snapshot(paths: &[&str]) -> FileTreeSnapshot {
    let root = paths
        .first()
        .map(|p| p.trim_start_matches('!'))
        .unwrap_or("/");
    let root_path: Arc<Path> = Arc::from(Path::new(root.trim_end_matches('/')));
    let mut snapshot = FileTreeSnapshot::new(root_path);

    for path_str in paths {
        let ignored = path_str.starts_with('!');
        let path_str = path_str.trim_start_matches('!');
        let is_dir = path_str.ends_with('/');
        let path_str = path_str.trim_end_matches('/');
        let path: Arc<Path> = Arc::from(Path::new(path_str));

        let entry = if is_dir {
            FileEntry::directory(path, ignored, true)
        } else {
            FileEntry::file(path, ignored)
        };
        snapshot.insert_entry(entry);
    }

    snapshot
}

// =============================================================================
// FileEntry Tests
// =============================================================================

#[test]
fn test_file_entry_creation() {
    let file = FileEntry::file(Path::new("/src/main.rs"), false);
    assert!(file.is_file());
    assert!(!file.is_dir());
    assert_eq!(file.extension(), Some("rs"));
    assert!(!file.ignored);
    assert!(file.loaded);

    let dir = FileEntry::directory(Path::new("/src"), false, true);
    assert!(dir.is_dir());
    assert!(!dir.is_file());
    assert_eq!(dir.extension(), None);
    assert!(!dir.ignored);
    assert!(dir.loaded);
}

#[test]
fn test_file_entry_ignored() {
    let ignored_file = FileEntry::file(Path::new("/target/debug/main"), true);
    assert!(ignored_file.ignored);

    let ignored_dir = FileEntry::directory(Path::new("/target"), true, false);
    assert!(ignored_dir.ignored);
    assert!(!ignored_dir.loaded);
}

// =============================================================================
// FileEntrySummary Tests
// =============================================================================

#[test]
fn test_summary_for_visible_file() {
    let entry = FileEntry::file(Path::new("/src/main.rs"), false);
    let summary = entry.summary();

    assert_eq!(summary.count, 1);
    assert_eq!(summary.visible_count, 1);
    assert_eq!(summary.file_count, 1);
    assert_eq!(summary.visible_file_count, 1);
}

#[test]
fn test_summary_for_ignored_file() {
    let entry = FileEntry::file(Path::new("/target/debug/main"), true);
    let summary = entry.summary();

    assert_eq!(summary.count, 1);
    assert_eq!(summary.visible_count, 0);
    assert_eq!(summary.file_count, 1);
    assert_eq!(summary.visible_file_count, 0);
}

#[test]
fn test_summary_for_visible_directory() {
    let entry = FileEntry::directory(Path::new("/src"), false, true);
    let summary = entry.summary();

    assert_eq!(summary.count, 1);
    assert_eq!(summary.visible_count, 1);
    assert_eq!(summary.file_count, 0);
    assert_eq!(summary.visible_file_count, 0);
}

#[test]
fn test_summary_for_ignored_directory() {
    let entry = FileEntry::directory(Path::new("/target"), true, false);
    let summary = entry.summary();

    assert_eq!(summary.count, 1);
    assert_eq!(summary.visible_count, 0);
    assert_eq!(summary.file_count, 0);
    assert_eq!(summary.visible_file_count, 0);
}

#[test]
fn test_summary_add_assign() {
    let mut summary1 = FileEntrySummary {
        max_path: Arc::from(Path::new("/a")),
        count: 2,
        visible_count: 1,
        file_count: 1,
        visible_file_count: 1,
    };

    let summary2 = FileEntrySummary {
        max_path: Arc::from(Path::new("/b")),
        count: 3,
        visible_count: 2,
        file_count: 2,
        visible_file_count: 1,
    };

    summary1 += &summary2;

    assert_eq!(summary1.max_path.as_ref(), Path::new("/b"));
    assert_eq!(summary1.count, 5);
    assert_eq!(summary1.visible_count, 3);
    assert_eq!(summary1.file_count, 3);
    assert_eq!(summary1.visible_file_count, 2);
}

// =============================================================================
// PathKey Tests
// =============================================================================

#[test]
fn test_path_key_ordering() {
    let key_a = PathKey::new(Path::new("/a"));
    let key_b = PathKey::new(Path::new("/b"));
    let key_aa = PathKey::new(Path::new("/a/a"));

    assert!(key_a < key_aa);
    assert!(key_aa < key_b);
    assert!(key_a < key_b);
}

// =============================================================================
// FileTreeSnapshot Tests
// =============================================================================

#[test]
fn test_snapshot_with_root() {
    let snapshot = FileTreeSnapshot::with_root(Path::new("/project"), false, true);
    assert_eq!(snapshot.len(), 1);

    let root = snapshot.entry_for_path(Path::new("/project")).unwrap();
    assert!(root.is_dir());
    assert!(root.loaded);
}

#[test]
fn test_insert_and_lookup() {
    let snapshot = test_snapshot(&[
        "/project/",
        "/project/src/",
        "/project/src/main.rs",
        "/project/src/lib.rs",
    ]);

    assert_eq!(snapshot.len(), 4);

    let main_rs = snapshot.entry_for_path(Path::new("/project/src/main.rs"));
    assert!(main_rs.is_some());
    assert!(main_rs.unwrap().is_file());

    let src = snapshot.entry_for_path(Path::new("/project/src"));
    assert!(src.is_some());
    assert!(src.unwrap().is_dir());

    let missing = snapshot.entry_for_path(Path::new("/project/nonexistent"));
    assert!(missing.is_none());
}

#[test]
fn test_remove_entry() {
    let mut snapshot = test_snapshot(&["/project/", "/project/src/", "/project/src/main.rs"]);

    assert_eq!(snapshot.len(), 3);

    snapshot.remove_entry(Path::new("/project/src/main.rs"));
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot
        .entry_for_path(Path::new("/project/src/main.rs"))
        .is_none());
    assert!(snapshot.entry_for_path(Path::new("/project/src")).is_some());
}

#[test]
fn test_child_entries() {
    let snapshot = test_snapshot(&[
        "/project/",
        "/project/src/",
        "/project/src/main.rs",
        "/project/src/lib.rs",
        "/project/tests/",
        "/project/tests/test.rs",
        "/project/Cargo.toml",
    ]);

    let root_children: Vec<_> = snapshot.child_entries(Path::new("/project")).collect();
    assert_eq!(root_children.len(), 3);

    let child_paths: Vec<_> = root_children.iter().map(|e| e.path.as_ref()).collect();
    assert!(child_paths.contains(&Path::new("/project/src")));
    assert!(child_paths.contains(&Path::new("/project/tests")));
    assert!(child_paths.contains(&Path::new("/project/Cargo.toml")));

    // Should not include nested entries
    assert!(!child_paths.contains(&Path::new("/project/src/main.rs")));

    let src_children: Vec<_> = snapshot.child_entries(Path::new("/project/src")).collect();
    assert_eq!(src_children.len(), 2);
}

#[test]
fn test_child_entries_empty_directory() {
    let snapshot = test_snapshot(&["/project/", "/project/empty/"]);

    let children: Vec<_> = snapshot
        .child_entries(Path::new("/project/empty"))
        .collect();
    assert!(children.is_empty());
}

// =============================================================================
// Lazy Loading Tests
// =============================================================================

#[test]
fn test_is_parent_loaded_root() {
    let snapshot = FileTreeSnapshot::with_root(Path::new("/project"), false, true);
    assert!(snapshot.is_parent_loaded(Path::new("/project/src")));
}

#[test]
fn test_is_parent_loaded_unloaded_directory() {
    let mut snapshot = FileTreeSnapshot::new(Path::new("/project"));
    snapshot.insert_entry(FileEntry::directory(Path::new("/project"), false, true));
    snapshot.insert_entry(FileEntry::directory(
        Path::new("/project/collapsed"),
        false,
        false,
    ));

    // Parent /project is loaded
    assert!(snapshot.is_parent_loaded(Path::new("/project/collapsed")));
    // Parent /project/collapsed is NOT loaded
    assert!(!snapshot.is_parent_loaded(Path::new("/project/collapsed/child.txt")));
}

#[test]
fn test_is_parent_loaded_nested() {
    let snapshot = test_snapshot(&["/project/", "/project/src/", "/project/src/nested/"]);

    assert!(snapshot.is_parent_loaded(Path::new("/project/src")));
    assert!(snapshot.is_parent_loaded(Path::new("/project/src/nested")));
    assert!(snapshot.is_parent_loaded(Path::new("/project/src/nested/file.rs")));
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_single_entry() {
    let mut snapshot = FileTreeSnapshot::new(Path::new("/"));
    snapshot.insert_entry(FileEntry::file(Path::new("/only_file.txt"), false));

    assert_eq!(snapshot.len(), 1);
    assert!(snapshot
        .entry_for_path(Path::new("/only_file.txt"))
        .is_some());
}

#[test]
fn test_update_existing_entry() {
    let mut snapshot = test_snapshot(&["/project/", "/project/file.txt"]);

    // Update the file to be ignored
    let updated = FileEntry::file(Path::new("/project/file.txt"), true);
    snapshot.insert_entry(updated);

    assert_eq!(snapshot.len(), 2); // Should not duplicate
    let entry = snapshot
        .entry_for_path(Path::new("/project/file.txt"))
        .unwrap();
    assert!(entry.ignored);
}

// =============================================================================
// Filesystem Event Handling Tests
// =============================================================================

#[test]
fn test_handle_added_in_loaded_directory() {
    let mut snapshot = test_snapshot(&["/project/", "/project/src/"]);

    // Add a file to a loaded directory
    let result = snapshot.handle_added(Path::new("/project/src/new_file.rs"), false, false);
    assert!(result);
    assert!(snapshot
        .entry_for_path(Path::new("/project/src/new_file.rs"))
        .is_some());
}

#[test]
fn test_handle_added_in_unloaded_directory() {
    let mut snapshot = FileTreeSnapshot::new(Path::new("/project"));
    snapshot.insert_entry(FileEntry::directory(Path::new("/project"), false, true));
    snapshot.insert_entry(FileEntry::directory(
        Path::new("/project/collapsed"),
        false,
        false,
    ));

    // Try to add a file to an unloaded directory - should fail
    let result = snapshot.handle_added(Path::new("/project/collapsed/file.rs"), false, false);
    assert!(!result);
    assert!(snapshot
        .entry_for_path(Path::new("/project/collapsed/file.rs"))
        .is_none());
}

#[test]
fn test_handle_removed() {
    let mut snapshot = test_snapshot(&["/project/", "/project/file.txt"]);

    let result = snapshot.handle_removed(Path::new("/project/file.txt"));
    assert!(result);
    assert!(snapshot
        .entry_for_path(Path::new("/project/file.txt"))
        .is_none());
}

#[test]
fn test_handle_removed_nonexistent() {
    let mut snapshot = test_snapshot(&["/project/"]);

    let result = snapshot.handle_removed(Path::new("/project/nonexistent.txt"));
    assert!(!result);
}

/// Test helper: Handles a file/directory being renamed/moved.
/// Returns true if the rename was processed.
fn handle_renamed(
    snapshot: &mut FileTreeSnapshot,
    old_path: &Path,
    new_path: &Path,
    is_dir: bool,
    ignored: bool,
) -> bool {
    let old_loaded = snapshot.is_parent_loaded(old_path);
    let new_loaded = snapshot.is_parent_loaded(new_path);

    // Remove from old location if parent was loaded
    if old_loaded {
        snapshot.remove_entry(old_path);
    }

    // Add to new location if parent is loaded
    if new_loaded {
        let entry = if is_dir {
            FileEntry::directory(Arc::from(new_path), ignored, false)
        } else {
            FileEntry::file(Arc::from(new_path), ignored)
        };
        snapshot.insert_entry(entry);
    }

    old_loaded || new_loaded
}

#[test]
fn test_handle_renamed() {
    let mut snapshot = test_snapshot(&["/project/", "/project/old_name.txt"]);

    let result = handle_renamed(
        &mut snapshot,
        Path::new("/project/old_name.txt"),
        Path::new("/project/new_name.txt"),
        false,
        false,
    );
    assert!(result);
    assert!(snapshot
        .entry_for_path(Path::new("/project/old_name.txt"))
        .is_none());
    assert!(snapshot
        .entry_for_path(Path::new("/project/new_name.txt"))
        .is_some());
}

#[test]
fn test_expand_directory() {
    let mut snapshot = FileTreeSnapshot::new(Path::new("/project"));
    snapshot.insert_entry(FileEntry::directory(Path::new("/project"), false, true));
    snapshot.insert_entry(FileEntry::directory(
        Path::new("/project/collapsed"),
        false,
        false,
    ));

    let entry = snapshot
        .entry_for_path(Path::new("/project/collapsed"))
        .unwrap();
    assert!(!entry.loaded);

    snapshot.expand_directory(Path::new("/project/collapsed"));

    let entry = snapshot
        .entry_for_path(Path::new("/project/collapsed"))
        .unwrap();
    assert!(entry.loaded);
}
