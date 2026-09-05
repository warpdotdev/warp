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

/// A directory subtree with a file and a nested directory (itself containing
/// a file), built up front the way a full `AddDirectorySubtree` payload
/// would be, rather than one path at a time.
fn create_nested_dir_entry() -> Entry {
    Entry::Directory(DirectoryEntry {
        path: std_path("/repo/src"),
        children: vec![
            create_file_entry("/repo/src/main.rs"),
            Entry::Directory(DirectoryEntry {
                path: std_path("/repo/src/nested"),
                children: vec![create_file_entry("/repo/src/nested/util.rs")],
                ignored: false,
                loaded: true,
            }),
        ],
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
fn test_insert_full_subtree_in_one_call() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let src = std_path("/repo/src");
    let main_rs = std_path("/repo/src/main.rs");
    let nested = std_path("/repo/src/nested");
    let util_rs = std_path("/repo/src/nested/util.rs");

    tree.insert_entry_at_path(Arc::new(src.clone()), create_nested_dir_entry());

    assert!(matches!(
        tree.get(&src),
        Some(FileTreeEntryState::Directory(_))
    ));
    assert!(matches!(
        tree.get(&main_rs),
        Some(FileTreeEntryState::File(_))
    ));
    assert!(matches!(
        tree.get(&nested),
        Some(FileTreeEntryState::Directory(_))
    ));
    assert!(matches!(
        tree.get(&util_rs),
        Some(FileTreeEntryState::File(_))
    ));

    let src_children: std::collections::HashSet<_> = tree
        .child_paths(&src)
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        src_children,
        std::collections::HashSet::from([
            main_rs.as_str().to_string(),
            nested.as_str().to_string()
        ])
    );

    let nested_children: Vec<_> = tree.child_paths(&nested).collect();
    assert_eq!(nested_children.len(), 1);
    assert_eq!(nested_children[0].as_str(), util_rs.as_str());

    // The `insert_entry_at_path` parent fix-up should have linked `src`
    // itself into the root's children.
    let root_children: Vec<_> = tree.child_paths(&std_path("/repo")).collect();
    assert!(root_children.iter().any(|p| p.as_str() == src.as_str()));
}

#[test]
fn test_insert_entry_at_path_overwrites_children_set_without_union() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let dir = std_path("/repo/dir");
    let old_child = std_path("/repo/dir/old.txt");
    let new_child = std_path("/repo/dir/new.txt");

    tree.insert_entry_at_path(
        Arc::new(dir.clone()),
        Entry::Directory(DirectoryEntry {
            path: dir.clone(),
            children: vec![create_file_entry("/repo/dir/old.txt")],
            ignored: false,
            loaded: true,
        }),
    );
    assert!(tree.get(&old_child).is_some());

    // Insert again at the same path with a disjoint child set, without
    // removing first. The children set for `dir` should be fully replaced
    // by the new call, matching the overwrite (not union) semantics of the
    // previous `extend()`-based merge.
    tree.insert_entry_at_path(
        Arc::new(dir.clone()),
        Entry::Directory(DirectoryEntry {
            path: dir.clone(),
            children: vec![create_file_entry("/repo/dir/new.txt")],
            ignored: false,
            loaded: true,
        }),
    );

    let children: Vec<_> = tree
        .child_paths(&dir)
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(children, vec![new_child.as_str().to_string()]);
    // `old.txt`'s state_map entry is orphaned rather than removed, matching
    // the previous behavior: `extend()` never touched it since the second
    // insert's child store didn't contain that key.
    assert!(tree.get(&old_child).is_some());
}

#[test]
fn test_insert_childless_directory_leaves_existing_children_untouched() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let dir = std_path("/repo/dir");
    let existing_child = std_path("/repo/dir/existing.txt");

    tree.insert_entry_at_path(
        Arc::new(dir.clone()),
        Entry::Directory(DirectoryEntry {
            path: dir.clone(),
            children: vec![create_file_entry("/repo/dir/existing.txt")],
            ignored: false,
            loaded: true,
        }),
    );

    // Re-insert `dir` as a childless directory. Since this entry contributes
    // no children, `dir`'s existing children set must be left as-is instead
    // of being cleared.
    tree.insert_entry_at_path(Arc::new(dir.clone()), create_dir_entry("/repo/dir"));

    let children: Vec<_> = tree
        .child_paths(&dir)
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(children, vec![existing_child.as_str().to_string()]);
}

#[test]
fn test_entry_count_entries_and_populated_directories() {
    let subtree = create_nested_dir_entry();
    // `src` + `main.rs` + `nested` + `nested/util.rs`.
    assert_eq!(subtree.count_entries(), 4);
    // `src` and `nested` both have children.
    assert_eq!(subtree.count_populated_directories(), 2);

    let file = create_file_entry("/repo/file.txt");
    assert_eq!(file.count_entries(), 1);
    assert_eq!(file.count_populated_directories(), 0);

    // A childless directory contributes an entry but not a populated
    // directory: it never gets a `parent_to_child_map` children-set entry.
    let empty_dir = create_dir_entry("/repo/empty");
    assert_eq!(empty_dir.count_entries(), 1);
    assert_eq!(empty_dir.count_populated_directories(), 0);
}

#[test]
fn test_count_populated_directories_ignores_wide_childless_directories() {
    // One populated root with many childless subdirectories: only the root
    // contributes a populated-directory count, even though every child is a
    // directory entry.
    let subtree = Entry::Directory(DirectoryEntry {
        path: std_path("/repo/src"),
        children: vec![
            create_dir_entry("/repo/src/empty1"),
            create_dir_entry("/repo/src/empty2"),
            create_dir_entry("/repo/src/empty3"),
        ],
        ignored: false,
        loaded: true,
    });

    assert_eq!(subtree.count_entries(), 4);
    assert_eq!(subtree.count_populated_directories(), 1);
}

#[test]
fn test_insert_wide_childless_directories_links_only_the_populated_root() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root));

    let src = std_path("/repo/src");
    let empty1 = std_path("/repo/src/empty1");
    let empty2 = std_path("/repo/src/empty2");
    let empty3 = std_path("/repo/src/empty3");

    tree.insert_entry_at_path(
        Arc::new(src.clone()),
        Entry::Directory(DirectoryEntry {
            path: src.clone(),
            children: vec![
                create_dir_entry("/repo/src/empty1"),
                create_dir_entry("/repo/src/empty2"),
                create_dir_entry("/repo/src/empty3"),
            ],
            ignored: false,
            loaded: true,
        }),
    );

    for empty in [&empty1, &empty2, &empty3] {
        assert!(tree.get(empty).is_some());
        // Childless directories don't get a children-set entry, so they
        // report no children rather than an empty-but-present one.
        assert_eq!(tree.child_paths(empty).count(), 0);
    }

    let src_children: std::collections::HashSet<_> = tree
        .child_paths(&src)
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        src_children,
        std::collections::HashSet::from([
            empty1.as_str().to_string(),
            empty2.as_str().to_string(),
            empty3.as_str().to_string(),
        ])
    );
}

#[test]
fn test_insert_links_to_parent_whether_or_not_parent_already_has_children() {
    let root = std_path("/repo");
    let mut tree = FileTreeEntry::new_for_directory(Arc::new(root.clone()));

    // `/repo` has no `parent_to_child_map` key yet: this insert must both
    // create it and link `first` into it.
    let first = std_path("/repo/first");
    tree.insert_entry_at_path(Arc::new(first.clone()), create_dir_entry("/repo/first"));
    let children_after_first: Vec<_> = tree.child_paths(&root).collect();
    assert_eq!(children_after_first.len(), 1);
    assert_eq!(children_after_first[0].as_str(), first.as_str());

    // `/repo` already has a `parent_to_child_map` key from the insert above:
    // this insert must add to it rather than assuming it needs creating.
    let second = std_path("/repo/second");
    tree.insert_entry_at_path(Arc::new(second.clone()), create_dir_entry("/repo/second"));
    let children_after_second: std::collections::HashSet<_> = tree
        .child_paths(&root)
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        children_after_second,
        std::collections::HashSet::from([first.as_str().to_string(), second.as_str().to_string()])
    );
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
