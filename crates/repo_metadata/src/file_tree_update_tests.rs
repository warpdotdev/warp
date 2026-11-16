use std::path::{Path, PathBuf};

use warp_util::standardized_path::StandardizedPath;

use crate::entry::{DirectoryEntry, Entry, FileId, FileMetadata};
use crate::file_tree_store::{FileTreeEntry, FileTreeEntryState};
use crate::file_tree_update::*;
use crate::local_model::LocalRepoMetadataModel;

// ── Helpers ──────────────────────────────────────────────────────────

/// Platform-appropriate absolute root for test paths.
/// On Windows `/repo` is not a valid local absolute path, so we use
/// a drive-letter prefix instead.
#[cfg(windows)]
const TEST_REPO_ROOT: &str = "C:\\repo";
#[cfg(not(windows))]
const TEST_REPO_ROOT: &str = "/repo";

/// Creates a `StandardizedPath` from a Unix-style test path like
/// `"/repo/src/main.rs"`, replacing the `/repo` prefix with the
/// platform-appropriate [`TEST_REPO_ROOT`].
fn std_path(unix_path: &str) -> StandardizedPath {
    let local = unix_path.replacen("/repo", TEST_REPO_ROOT, 1);
    StandardizedPath::try_from_local(Path::new(&local)).unwrap()
}

/// Like [`std_path`] but returns a `PathBuf` suitable for use inside
/// `FileTreeMutation` (which carries local filesystem paths).
fn mutation_path(unix_path: &str) -> PathBuf {
    PathBuf::from(unix_path.replacen("/repo", TEST_REPO_ROOT, 1))
}

fn file(path: &str) -> Entry {
    Entry::File(FileMetadata {
        path: std_path(path),
        file_id: FileId::new(),
        extension: std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_string),
        ignored: false,
    })
}

fn dir(path: &str, children: Vec<Entry>) -> Entry {
    Entry::Directory(DirectoryEntry {
        path: std_path(path),
        children,
        ignored: false,
        loaded: true,
    })
}

fn ignored_file(path: &str) -> Entry {
    Entry::File(FileMetadata {
        path: std_path(path),
        file_id: FileId::new(),
        extension: None,
        ignored: true,
    })
}

fn build_tree_from_entry(entry: Entry) -> FileTreeEntry {
    FileTreeEntry::from(entry)
}

// ── flatten_entry_metadata tests ─────────────────────────────────────

#[test]
fn flatten_single_file() {
    let entry = file("/repo/src/main.rs");
    let metadata = flatten_entry_metadata(&entry);

    assert_eq!(metadata.len(), 1);
    assert!(matches!(
        &metadata[0],
        RepoNodeMetadata::File(f) if f.path == std_path("/repo/src/main.rs")
    ));
}

#[test]
fn flatten_directory_with_children_is_depth_first_preorder() {
    let entry = dir(
        "/repo/src",
        vec![
            dir(
                "/repo/src/components",
                vec![
                    file("/repo/src/components/button.rs"),
                    file("/repo/src/components/modal.rs"),
                ],
            ),
            file("/repo/src/main.rs"),
        ],
    );

    let metadata = flatten_entry_metadata(&entry);

    assert_eq!(metadata.len(), 5);
    assert!(
        matches!(&metadata[0], RepoNodeMetadata::Directory(d) if d.path == std_path("/repo/src"))
    );
    assert!(
        matches!(&metadata[1], RepoNodeMetadata::Directory(d) if d.path == std_path("/repo/src/components"))
    );
    assert!(
        matches!(&metadata[2], RepoNodeMetadata::File(f) if f.path == std_path("/repo/src/components/button.rs"))
    );
    assert!(
        matches!(&metadata[3], RepoNodeMetadata::File(f) if f.path == std_path("/repo/src/components/modal.rs"))
    );
    assert!(
        matches!(&metadata[4], RepoNodeMetadata::File(f) if f.path == std_path("/repo/src/main.rs"))
    );
}

#[test]
fn flatten_preserves_ignored_flag() {
    let entry = dir("/repo", vec![ignored_file("/repo/secret.env")]);

    let metadata = flatten_entry_metadata(&entry);
    assert_eq!(metadata.len(), 2);
    assert!(matches!(
        &metadata[1],
        RepoNodeMetadata::File(f) if f.ignored
    ));
}

// ── apply_file_tree_mutations update-generation tests ─────────────────

#[test]
fn apply_mutations_generates_update_for_remove() {
    use crate::local_model::FileTreeMutation;

    let initial = dir("/repo", vec![file("/repo/old.rs")]);
    let mut tree = build_tree_from_entry(initial);
    let mutations = vec![FileTreeMutation::Remove(mutation_path("/repo/old.rs"))];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, true)
            .expect("update should be produced");

    assert_eq!(update.remove_entries.len(), 1);
    assert_eq!(update.remove_entries[0], std_path("/repo/old.rs"));
    assert!(update.update_entries.is_empty());
}

#[test]
fn apply_mutations_generates_update_for_add_file() {
    use crate::local_model::FileTreeMutation;

    let initial = dir("/repo", vec![dir("/repo/src", vec![])]);
    let mut tree = build_tree_from_entry(initial);
    let mutations = vec![FileTreeMutation::AddFile {
        path: mutation_path("/repo/src/new.rs"),
        is_ignored: false,
        extension: Some("rs".to_string()),
    }];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, true)
            .expect("update should be produced");

    assert!(update.remove_entries.is_empty());
    assert_eq!(update.update_entries.len(), 1);
    assert_eq!(
        update.update_entries[0].parent_path_to_replace,
        std_path("/repo/src")
    );
    assert_eq!(update.update_entries[0].subtree_metadata.len(), 1);
    assert!(matches!(
        &update.update_entries[0].subtree_metadata[0],
        RepoNodeMetadata::File(f) if f.path == std_path("/repo/src/new.rs")
            && f.extension == Some("rs".to_string())
            && !f.ignored
    ));
}

#[test]
fn apply_mutations_generates_update_for_add_directory_subtree() {
    use crate::local_model::FileTreeMutation;

    let subtree = dir(
        "/repo/src/components",
        vec![
            file("/repo/src/components/button.rs"),
            file("/repo/src/components/modal.rs"),
        ],
    );

    let initial = dir("/repo", vec![dir("/repo/src", vec![])]);
    let mut tree = build_tree_from_entry(initial);
    let mutations = vec![FileTreeMutation::AddDirectorySubtree {
        dir_path: mutation_path("/repo/src/components"),
        subtree,
    }];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, true)
            .expect("update should be produced");

    assert!(update.remove_entries.is_empty());
    assert_eq!(update.update_entries.len(), 1);
    let entry_update = &update.update_entries[0];
    assert_eq!(entry_update.parent_path_to_replace, std_path("/repo/src"));
    // 1 dir + 2 files
    assert_eq!(entry_update.subtree_metadata.len(), 3);
    assert!(matches!(
        &entry_update.subtree_metadata[0],
        RepoNodeMetadata::Directory(d) if d.path == std_path("/repo/src/components")
    ));
}

#[test]
fn apply_mutations_generates_update_for_add_empty_directory() {
    use crate::local_model::FileTreeMutation;

    let initial = dir("/repo", vec![dir("/repo/src", vec![])]);
    let mut tree = build_tree_from_entry(initial);
    let mutations = vec![FileTreeMutation::AddUnloadedDirectory {
        path: mutation_path("/repo/src/empty"),
        is_ignored: true,
    }];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, true)
            .expect("update should be produced");

    assert!(update.remove_entries.is_empty());
    assert_eq!(update.update_entries.len(), 1);
    assert!(matches!(
        &update.update_entries[0].subtree_metadata[0],
        RepoNodeMetadata::Directory(d) if d.path == std_path("/repo/src/empty")
            && d.ignored
            && !d.loaded
    ));
}

#[test]
fn apply_mutations_generates_update_for_mixed_mutations() {
    use crate::local_model::FileTreeMutation;

    let initial = dir("/repo", vec![file("/repo/old.rs")]);
    let mut tree = build_tree_from_entry(initial);
    let mutations = vec![
        FileTreeMutation::Remove(mutation_path("/repo/old.rs")),
        FileTreeMutation::AddFile {
            path: mutation_path("/repo/new.rs"),
            is_ignored: false,
            extension: Some("rs".to_string()),
        },
        FileTreeMutation::AddUnloadedDirectory {
            path: mutation_path("/repo/new_dir"),
            is_ignored: false,
        },
    ];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, true)
            .expect("update should be produced");

    assert_eq!(update.remove_entries.len(), 1);
    assert_eq!(update.update_entries.len(), 2);
}

#[test]
fn apply_mutations_returns_none_when_emit_updates_is_false() {
    use crate::local_model::FileTreeMutation;

    let initial = dir("/repo", vec![file("/repo/old.rs")]);
    let mut tree = build_tree_from_entry(initial);
    let mutations = vec![FileTreeMutation::Remove(mutation_path("/repo/old.rs"))];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, false, false);

    assert!(
        update.is_none(),
        "should return None when emit_updates is false"
    );
    assert!(
        tree.get(&std_path("/repo/old.rs")).is_none(),
        "old.rs should still be removed from the tree"
    );
}

// ── apply_repo_metadata_update tests

#[test]
fn apply_complete_update_adds_files_and_directories() {
    let initial = dir("/repo", vec![dir("/repo/src", vec![])]);
    let mut tree = build_tree_from_entry(initial);

    let update = RepoMetadataUpdate {
        repo_path: std_path("/repo"),
        remove_entries: vec![],
        update_entries: vec![FileTreeEntryUpdate {
            parent_path_to_replace: std_path("/repo/src"),
            subtree_metadata: vec![
                RepoNodeMetadata::Directory(DirectoryNodeMetadata {
                    path: std_path("/repo/src/components"),
                    ignored: false,
                    loaded: true,
                }),
                RepoNodeMetadata::File(FileNodeMetadata {
                    path: std_path("/repo/src/components/button.rs"),
                    extension: Some("rs".to_string()),
                    ignored: false,
                }),
                RepoNodeMetadata::File(FileNodeMetadata {
                    path: std_path("/repo/src/main.rs"),
                    extension: Some("rs".to_string()),
                    ignored: false,
                }),
            ],
        }],
        standing_results_delta: Default::default(),
    };

    tree.apply_repo_metadata_update(&update);

    assert!(
        matches!(tree.get(&std_path("/repo/src/components")), Some(FileTreeEntryState::Directory(d)) if d.loaded),
        "components/ should exist and be loaded"
    );
    assert!(
        tree.get(&std_path("/repo/src/components/button.rs"))
            .is_some(),
        "button.rs should exist"
    );
    assert!(
        tree.get(&std_path("/repo/src/main.rs")).is_some(),
        "main.rs should exist"
    );
    let src_children: Vec<_> = tree.child_paths(&std_path("/repo/src")).collect();
    assert_eq!(src_children.len(), 2, "src/ should have 2 children");
    let comp_children: Vec<_> = tree
        .child_paths(&std_path("/repo/src/components"))
        .collect();
    assert_eq!(comp_children.len(), 1, "components/ should have 1 child");
}

#[test]
fn apply_update_with_removals_and_additions() {
    let initial = dir(
        "/repo",
        vec![dir("/repo/src", vec![file("/repo/src/old.rs")])],
    );
    let mut tree = build_tree_from_entry(initial);

    assert!(tree.get(&std_path("/repo/src/old.rs")).is_some());

    let update = RepoMetadataUpdate {
        repo_path: std_path("/repo"),
        remove_entries: vec![std_path("/repo/src/old.rs")],
        update_entries: vec![FileTreeEntryUpdate {
            parent_path_to_replace: std_path("/repo/src"),
            subtree_metadata: vec![RepoNodeMetadata::File(FileNodeMetadata {
                path: std_path("/repo/src/new.rs"),
                extension: Some("rs".to_string()),
                ignored: false,
            })],
        }],
        standing_results_delta: Default::default(),
    };

    tree.apply_repo_metadata_update(&update);

    assert!(
        tree.get(&std_path("/repo/src/old.rs")).is_none(),
        "old.rs should be removed"
    );
    assert!(
        tree.get(&std_path("/repo/src/new.rs")).is_some(),
        "new.rs should be added"
    );
}

#[test]
fn apply_incomplete_update_missing_children_subtree() {
    let initial = dir("/repo", vec![dir("/repo/src", vec![])]);
    let mut tree = build_tree_from_entry(initial);

    let update = RepoMetadataUpdate {
        repo_path: std_path("/repo"),
        remove_entries: vec![],
        update_entries: vec![FileTreeEntryUpdate {
            parent_path_to_replace: std_path("/repo/src"),
            subtree_metadata: vec![RepoNodeMetadata::Directory(DirectoryNodeMetadata {
                path: std_path("/repo/src/components"),
                ignored: false,
                loaded: false,
            })],
        }],
        standing_results_delta: Default::default(),
    };

    tree.apply_repo_metadata_update(&update);

    let entry = tree.get(&std_path("/repo/src/components"));
    assert!(
        matches!(entry, Some(FileTreeEntryState::Directory(d)) if !d.loaded),
        "components/ should exist but be unloaded"
    );
    let children: Vec<_> = tree
        .child_paths(&std_path("/repo/src/components"))
        .collect();
    assert!(
        children.is_empty(),
        "components/ should have no children yet"
    );

    let followup = RepoMetadataUpdate {
        repo_path: std_path("/repo"),
        remove_entries: vec![],
        update_entries: vec![FileTreeEntryUpdate {
            parent_path_to_replace: std_path("/repo/src/components"),
            subtree_metadata: vec![RepoNodeMetadata::File(FileNodeMetadata {
                path: std_path("/repo/src/components/button.rs"),
                extension: Some("rs".to_string()),
                ignored: false,
            })],
        }],
        standing_results_delta: Default::default(),
    };

    tree.apply_repo_metadata_update(&followup);

    assert!(
        tree.get(&std_path("/repo/src/components/button.rs"))
            .is_some(),
        "button.rs should now exist after followup"
    );
}

#[test]
fn apply_incomplete_update_missing_parent_from_undelivered_page() {
    let initial = dir("/repo", vec![dir("/repo/src", vec![])]);
    let mut tree = build_tree_from_entry(initial);

    let update = RepoMetadataUpdate {
        repo_path: std_path("/repo"),
        remove_entries: vec![],
        update_entries: vec![FileTreeEntryUpdate {
            parent_path_to_replace: std_path("/repo/src/components/ui"),
            subtree_metadata: vec![RepoNodeMetadata::File(FileNodeMetadata {
                path: std_path("/repo/src/components/ui/button.rs"),
                extension: Some("rs".to_string()),
                ignored: false,
            })],
        }],
        standing_results_delta: Default::default(),
    };

    tree.apply_repo_metadata_update(&update);

    assert!(
        tree.get(&std_path("/repo/src/components")).is_some(),
        "intermediate components/ should be created"
    );
    assert!(
        tree.get(&std_path("/repo/src/components/ui")).is_some(),
        "intermediate ui/ should be created"
    );
    assert!(
        tree.get(&std_path("/repo/src/components/ui/button.rs"))
            .is_some(),
        "button.rs should be inserted under the created parents"
    );

    assert!(
        matches!(
            tree.get(&std_path("/repo/src/components")),
            Some(FileTreeEntryState::Directory(d)) if !d.loaded
        ),
        "auto-created components/ should be unloaded"
    );
}

// ── Round-trip test: apply mutations on server, then apply update on client ───

#[test]
fn round_trip_apply_mutations_then_apply_update_produces_equivalent_tree() {
    use crate::local_model::FileTreeMutation;

    let server_tree_entry = dir(
        "/repo",
        vec![dir("/repo/src", vec![file("/repo/src/lib.rs")])],
    );
    let mut server_tree = build_tree_from_entry(server_tree_entry);

    let subtree = dir(
        "/repo/src/util",
        vec![
            file("/repo/src/util/helpers.rs"),
            file("/repo/src/util/constants.rs"),
        ],
    );
    let mutations = vec![
        FileTreeMutation::Remove(mutation_path("/repo/src/lib.rs")),
        FileTreeMutation::AddDirectorySubtree {
            dir_path: mutation_path("/repo/src/util"),
            subtree,
        },
        FileTreeMutation::AddFile {
            path: mutation_path("/repo/src/main.rs"),
            is_ignored: false,
            extension: Some("rs".to_string()),
        },
    ];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut server_tree, mutations, false, true)
            .expect("update should be produced");

    let client_tree_entry = dir(
        "/repo",
        vec![dir("/repo/src", vec![file("/repo/src/lib.rs")])],
    );
    let mut client_tree = build_tree_from_entry(client_tree_entry);

    client_tree.apply_repo_metadata_update(&update);

    assert!(
        client_tree.get(&std_path("/repo/src/lib.rs")).is_none(),
        "lib.rs should be removed"
    );
    assert!(
        client_tree.get(&std_path("/repo/src/util")).is_some(),
        "util/ should exist"
    );
    assert!(
        client_tree
            .get(&std_path("/repo/src/util/helpers.rs"))
            .is_some(),
        "helpers.rs should exist"
    );
    assert!(
        client_tree
            .get(&std_path("/repo/src/util/constants.rs"))
            .is_some(),
        "constants.rs should exist"
    );
    assert!(
        client_tree.get(&std_path("/repo/src/main.rs")).is_some(),
        "main.rs should exist"
    );
}

// ── Lazy-load filtering test ─────────────────────────────────────────

#[test]
fn lazy_load_filters_mutations_for_unloaded_parents() {
    use crate::local_model::FileTreeMutation;

    let initial = Entry::Directory(DirectoryEntry {
        path: std_path("/repo"),
        children: vec![
            Entry::Directory(DirectoryEntry {
                path: std_path("/repo/src"),
                children: vec![],
                ignored: false,
                loaded: true,
            }),
            Entry::Directory(DirectoryEntry {
                path: std_path("/repo/vendor"),
                children: vec![],
                ignored: false,
                loaded: false,
            }),
        ],
        ignored: false,
        loaded: true,
    });
    let mut tree = build_tree_from_entry(initial);

    let mutations = vec![
        FileTreeMutation::AddFile {
            path: mutation_path("/repo/src/main.rs"),
            is_ignored: false,
            extension: Some("rs".to_string()),
        },
        FileTreeMutation::AddFile {
            path: mutation_path("/repo/vendor/lib.rs"),
            is_ignored: false,
            extension: Some("rs".to_string()),
        },
    ];

    let update =
        LocalRepoMetadataModel::apply_file_tree_mutations(&mut tree, mutations, true, true)
            .expect("update should be produced when emit_updates is true");

    assert!(
        tree.get(&std_path("/repo/src/main.rs")).is_some(),
        "main.rs under loaded src/ should exist"
    );
    assert!(
        tree.get(&std_path("/repo/vendor/lib.rs")).is_none(),
        "lib.rs under unloaded vendor/ should NOT exist"
    );

    assert!(update.remove_entries.is_empty());
    assert_eq!(
        update.update_entries.len(),
        1,
        "update should only contain the applied mutation"
    );
    assert!(matches!(
        &update.update_entries[0].subtree_metadata[0],
        RepoNodeMetadata::File(f) if f.path == std_path("/repo/src/main.rs")
    ));
}
