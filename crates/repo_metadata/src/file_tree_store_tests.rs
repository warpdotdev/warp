use std::sync::Arc;

use warp_util::standardized_path::StandardizedPath;

use crate::entry::{DirectoryEntry, Entry, FileId, FileMetadata};
use crate::file_tree_store::{FileTreeEntry, FileTreeEntryState};

fn std_path(s: &str) -> StandardizedPath {
    StandardizedPath::try_new(s).expect("test path should be valid")
}

fn create_file_entry(path: &str) -> Entry {
    let sp = std_path(path);
    Entry::File(FileMetadata {
        path: sp.clone(),
        file_id: FileId::new(),
        extension: sp.extension().map(|s| s.to_owned()),
        ignored: false,
    })
}

fn create_dir_entry(path: &str) -> Entry {
    Entry::Directory(DirectoryEntry {
        path: std_path(path),
        children: vec![],
        ignored: false,
        loaded: true,
    })
}

#[test]
fn test_remove_file() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root.clone()));

    let file = std_path("/repo/file.txt");
    let file_entry = create_file_entry("/repo/file.txt");

    tree.insert_entry_at_path(Arc::new(file.clone()), file_entry);

    assert!(tree.get(&file).is_some());

    tree.remove(&file);

    assert!(tree.get(&file).is_none());
}

#[test]
fn test_remove_directory_with_children() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let dir = std_path("/repo/src");
    let file = std_path("/repo/src/main.rs");

    tree.insert_entry_at_path(Arc::new(dir.clone()), create_dir_entry("/repo/src"));
    tree.insert_entry_at_path(
        Arc::new(file.clone()),
        create_file_entry("/repo/src/main.rs"),
    );

    assert!(tree.get(&dir).is_some());
    assert!(tree.get(&file).is_some());

    tree.remove(&dir);

    assert!(tree.get(&dir).is_none());
    assert!(tree.get(&file).is_none());
}

#[test]
fn test_rename_file() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let old = std_path("/repo/old.txt");
    let new = std_path("/repo/new.txt");

    tree.insert_entry_at_path(Arc::new(old.clone()), create_file_entry("/repo/old.txt"));

    assert!(tree.get(&old).is_some());

    tree.rename_path(&old, &new);

    assert!(tree.get(&old).is_none());

    let new_entry = tree.get(&new);
    assert!(new_entry.is_some());
    if let Some(FileTreeEntryState::File(f)) = new_entry {
        assert_eq!(f.path.as_str(), "/repo/new.txt");
    } else {
        panic!("Expected file entry");
    }
}

#[test]
fn test_rename_directory_recursive() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let old_dir = std_path("/repo/old_src");
    let new_dir = std_path("/repo/new_src");
    let child = std_path("/repo/old_src/main.rs");
    let new_child = std_path("/repo/new_src/main.rs");

    tree.insert_entry_at_path(Arc::new(old_dir.clone()), create_dir_entry("/repo/old_src"));
    tree.insert_entry_at_path(
        Arc::new(child.clone()),
        create_file_entry("/repo/old_src/main.rs"),
    );

    assert!(tree.get(&old_dir).is_some());
    assert!(tree.get(&child).is_some());

    let result = tree.rename_path(&old_dir, &new_dir);
    assert!(result, "Rename should succeed");

    assert!(tree.get(&old_dir).is_none(), "Old directory should be gone");
    assert!(tree.get(&child).is_none(), "Old child should be gone");

    let new_dir_entry = tree.get(&new_dir);
    assert!(new_dir_entry.is_some(), "New directory should exist");
    if let Some(FileTreeEntryState::Directory(d)) = new_dir_entry {
        assert_eq!(d.path.as_str(), "/repo/new_src");
    } else {
        panic!("Expected directory entry");
    }

    let new_child_entry = tree.get(&new_child);
    assert!(new_child_entry.is_some(), "New child should exist");
    if let Some(FileTreeEntryState::File(f)) = new_child_entry {
        assert_eq!(f.path.as_str(), "/repo/new_src/main.rs");
    } else {
        panic!("Expected file entry");
    }
}

#[test]
fn test_remove_nested_children_recursively() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let foo = std_path("/repo/foo");
    let bar = std_path("/repo/foo/bar");
    let bazz = std_path("/repo/foo/bar/bazz");
    let buzz = std_path("/repo/foo/bar/bazz/buzz.rs");

    tree.insert_entry_at_path(Arc::new(foo.clone()), create_dir_entry("/repo/foo"));
    tree.insert_entry_at_path(Arc::new(bar.clone()), create_dir_entry("/repo/foo/bar"));
    tree.insert_entry_at_path(
        Arc::new(bazz.clone()),
        create_dir_entry("/repo/foo/bar/bazz"),
    );
    tree.insert_entry_at_path(
        Arc::new(buzz.clone()),
        create_file_entry("/repo/foo/bar/bazz/buzz.rs"),
    );

    assert!(tree.get(&foo).is_some());
    assert!(tree.get(&bar).is_some());
    assert!(tree.get(&bazz).is_some());
    assert!(tree.get(&buzz).is_some());

    tree.remove(&foo);

    assert!(tree.get(&foo).is_none(), "foo should be gone");
    assert!(tree.get(&bar).is_none(), "bar should be gone");
    assert!(tree.get(&bazz).is_none(), "bazz should be gone");
    assert!(tree.get(&buzz).is_none(), "buzz.rs should be gone");
}

#[test]
fn test_rename_directory_parent_child_link_consistency() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let old_dir = std_path("/repo/old_src");
    let new_dir = std_path("/repo/new_src");
    let child = std_path("/repo/old_src/main.rs");

    tree.insert_entry_at_path(Arc::new(old_dir.clone()), create_dir_entry("/repo/old_src"));
    tree.insert_entry_at_path(
        Arc::new(child.clone()),
        create_file_entry("/repo/old_src/main.rs"),
    );

    let result = tree.rename_path(&old_dir, &new_dir);
    assert!(result, "Rename should succeed");

    let children: Vec<_> = tree.child_paths(&new_dir).collect();
    let new_child = std_path("/repo/new_src/main.rs");

    assert_eq!(children.len(), 1, "New directory should have 1 child");
    assert_eq!(
        children[0].as_str(),
        new_child.as_str(),
        "Child path in parent_to_child_map should match the renamed child path"
    );
}

/// Regression test for APP-4835: `ensure_parent_directories_exist` must not
/// deep-clone the whole store (via `Arc::make_mut`) when every ancestor already
/// exists, even while the store is shared with another holder. It must still
/// copy-on-write (leaving the shared snapshot untouched) when it genuinely
/// inserts a missing ancestor.
#[test]
fn test_ensure_parent_directories_no_cow_when_ancestors_exist() {
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(std_path("/repo")));
    tree.insert_entry_at_path(Arc::new(std_path("/repo/a")), create_dir_entry("/repo/a"));
    tree.insert_entry_at_path(
        Arc::new(std_path("/repo/a/b")),
        create_dir_entry("/repo/a/b"),
    );

    // Share the underlying store with a reader snapshot so `Arc::make_mut`
    // would clone if it were called.
    let shared = tree.clone();
    tree.ensure_parent_directories_exist(&std_path("/repo/a/b"));
    assert!(
        Arc::ptr_eq(&tree.state_map, &shared.state_map),
        "no copy-on-write should occur when all ancestors already exist"
    );

    // Now insert missing ancestors: this must copy-on-write, forking from the
    // shared snapshot without mutating it.
    let shared_before_insert = tree.clone();
    tree.ensure_parent_directories_exist(&std_path("/repo/x/y"));
    assert!(
        !Arc::ptr_eq(&tree.state_map, &shared_before_insert.state_map),
        "copy-on-write should occur when inserting missing ancestors"
    );
    assert!(tree.get(&std_path("/repo/x")).is_some());
    assert!(tree.get(&std_path("/repo/x/y")).is_some());
    assert!(
        shared_before_insert.get(&std_path("/repo/x")).is_none(),
        "the shared snapshot must not observe the new directories"
    );
}
