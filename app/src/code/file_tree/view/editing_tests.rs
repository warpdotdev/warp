use std::sync::Arc;

use repo_metadata::file_tree_store::{FileTreeDirectoryEntryState, FileTreeEntryState};
use repo_metadata::{FileMetadata, FileTreeEntry};
use black_util::standardized_path::StandardizedPath;

use super::sort_entries_for_file_tree;

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
