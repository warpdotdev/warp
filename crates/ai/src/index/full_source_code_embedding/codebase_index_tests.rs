#![allow(clippy::single_range_in_vec_init)]
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use futures::executor::block_on;
use repo_metadata::DirectoryWatcher;
use string_offset::ByteOffset;
use virtual_fs::{Stub, VirtualFS};
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, SingletonEntity};

use super::{
    CodebaseIndex, CodebaseIndexTimeStampMetadata, ServerSyncResult, TreeSourceSyncState,
    DEFAULT_INCREMENAL_SYNC_FLUSH_INTERVAL,
};
use crate::index::full_source_code_embedding::changed_files::ChangedFiles;
use crate::index::full_source_code_embedding::codebase_index::MAX_DEPTH;
use crate::index::full_source_code_embedding::fragment_metadata::{
    FragmentLocation, LeafToFragmentMetadata,
};
use crate::index::full_source_code_embedding::merkle_tree::{MerkleHash, MerkleTree};
use crate::index::full_source_code_embedding::store_client::MockStoreClient;
use crate::index::full_source_code_embedding::{
    ContentHash, EmbeddingConfig, Fragment, FragmentMetadata,
};
use crate::index::locations::{CodeContextLocation, FileFragmentLocation};

impl CodebaseIndex {
    fn new_for_test(
        fragment_metadata: HashMap<MerkleHash, Vec<FragmentMetadata>>,
        app: &mut App,
    ) -> Self {
        let repo_path = PathBuf::from(".");
        let repository = DirectoryWatcher::handle(app).update(app, |repo_watcher, ctx| {
            repo_watcher
                .add_directory(
                    StandardizedPath::from_local_canonicalized(&repo_path).unwrap(),
                    ctx,
                )
                .unwrap()
        });

        Self {
            repo_path,
            repository,
            ts_metadata: CodebaseIndexTimeStampMetadata {
                last_edited: Some(Utc::now()),
                last_snapshot: None,
                earliest_unsynced_change: None,
            },
            next_incremental_flush_handle: None,
            incremental_sync_interval: DEFAULT_INCREMENAL_SYNC_FLUSH_INTERVAL,
            embedding_config: EmbeddingConfig::default(),
            leaf_node_to_fragment_metadatas: LeafToFragmentMetadata::new_for_test(
                fragment_metadata,
            ),
            gitignores: Arc::new(vec![]),
            tree_sync_state: TreeSourceSyncState::unsynced(),
            retrieval_requests: HashMap::new(),
            store_client: Arc::new(MockStoreClient {}),
            pending_file_changes: None,
            sync_progress_tx: async_channel::unbounded().0,
            embedding_generation_batch_size: 0,
        }
    }
}

// Helper function to create a test fragment
fn create_test_fragment(
    content: &str,
    content_hash: ContentHash,
    path: &str,
    byte_range: Range<ByteOffset>,
) -> Fragment {
    Fragment {
        content: content.to_string(),
        content_hash,
        location: crate::index::full_source_code_embedding::FragmentLocation {
            absolute_path: PathBuf::from(path),
            byte_range,
        },
    }
}

// Helper function to create test fragment metadata
fn create_test_metadata(
    path: &str,
    byte_range: Range<ByteOffset>,
    start_line: usize,
    end_line: usize,
) -> FragmentMetadata {
    FragmentMetadata {
        absolute_path: PathBuf::from(path),
        location: FragmentLocation {
            start_line,
            end_line,
            byte_range,
        },
    }
}

#[test]
fn synced_index_with_queued_file_changes_reports_pending_status() {
    VirtualFS::test(
        "synced_index_with_queued_file_changes_reports_pending_status",
        |dirs, mut sandbox| {
            App::test((), |mut app| async move {
                app.add_singleton_model(DirectoryWatcher::new);

                let repo_name = "warp-virtual";
                sandbox.mkdir(repo_name);
                sandbox.with_files(vec![Stub::FileWithContent(
                    format!("{repo_name}/existing_file").as_str(),
                    "existing content",
                )]);

                let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();
                let build_file_tree_result =
                    block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
                let (tree, _) =
                    block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

                let mut index = CodebaseIndex::new_for_test(Default::default(), &mut app);
                index.tree_sync_state = TreeSourceSyncState::Synced {
                    tree,
                    server_sync_result: ServerSyncResult::Success,
                };

                let mut changed_files = ChangedFiles::default();
                changed_files.upsertions.insert(repo_path.join("new_file"));
                index.pending_file_changes = Some(changed_files);

                let status = index.codebase_index_status();
                assert!(status.has_pending());
                assert!(status.has_synced_version());
                assert_eq!(status.last_sync_successful(), Some(true));
            });
        },
    );
}

#[test]
fn synced_index_without_pending_file_changes_stays_ready_after_flush() {
    VirtualFS::test(
        "synced_index_without_pending_file_changes_stays_ready_after_flush",
        |dirs, mut sandbox| {
            App::test((), |mut app| async move {
                app.add_singleton_model(DirectoryWatcher::new);

                let repo_name = "warp-virtual";
                sandbox.mkdir(repo_name);
                sandbox.with_files(vec![Stub::FileWithContent(
                    format!("{repo_name}/existing_file").as_str(),
                    "existing content",
                )]);

                let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();
                let build_file_tree_result =
                    block_on(CodebaseIndex::build_file_tree(repo_path, None)).unwrap();
                let (tree, _) =
                    block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

                let mut test_index = CodebaseIndex::new_for_test(Default::default(), &mut app);
                test_index.tree_sync_state = TreeSourceSyncState::Synced {
                    tree,
                    server_sync_result: ServerSyncResult::Success,
                };
                let index = app.add_model(|_| test_index);

                index.update(&mut app, |index, ctx| {
                    index.flush_pending_file_changes(ctx);

                    let status = index.codebase_index_status();
                    assert!(!status.has_pending());
                    assert!(status.has_synced_version());
                    assert_eq!(status.last_sync_successful(), Some(true));
                });
            });
        },
    );
}
#[test]
fn test_empty_fragments() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test with empty input
        let mock_index = CodebaseIndex::new_for_test(Default::default(), &mut app);
        let fragments = Vec::new();
        let result = mock_index.process_fragments(fragments, 0);

        assert!(
            result.is_empty(),
            "Empty fragments should result in empty output"
        );
    });
}

#[test]
fn test_single_fragment() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test with a single fragment
        let hash = MerkleHash::from_bytes("fragment_content".as_bytes());
        let file_path = "/path/to/file.rs";
        let byte_range = ByteOffset::from(0)..ByteOffset::from(100);

        let fragment = create_test_fragment(
            "fragment_content",
            ContentHash::new(hash.clone()),
            file_path,
            byte_range.clone(),
        );
        let metadata = create_test_metadata(file_path, byte_range, 10, 20);

        let mock_index =
            CodebaseIndex::new_for_test(HashMap::from([(hash, vec![metadata])]), &mut app);

        let result = mock_index.process_fragments(vec![fragment], 0);

        assert_eq!(result.len(), 1, "Should have one result for one fragment");

        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![10..21], // end is exclusive, so 20+1
        });

        assert!(
            result.contains(&expected),
            "Result should contain the expected fragment"
        );
    });
}

#[test]
fn test_multiple_fragments_different_files() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test with fragments in different files
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());

        let file1_path = "/path/to/file1.rs";
        let file2_path = "/path/to/file2.rs";

        let byte_range1 = ByteOffset::from(0)..ByteOffset::from(50);
        let byte_range2 = ByteOffset::from(0)..ByteOffset::from(60);

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file1_path,
            byte_range1.clone(),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file2_path,
            byte_range2.clone(),
        );

        let metadata1 = create_test_metadata(file1_path, byte_range1, 5, 15);
        let metadata2 = create_test_metadata(file2_path, byte_range2, 20, 30);

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([(hash1, vec![metadata1]), (hash2, vec![metadata2])]),
            &mut app,
        );

        let result = mock_index.process_fragments(vec![fragment1, fragment2], 0);

        assert_eq!(
            result.len(),
            2,
            "Should have two results for two fragments in different files"
        );

        let expected1 = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file1_path),
            line_ranges: vec![5..16], // end is exclusive, so 15+1
        });

        let expected2 = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file2_path),
            line_ranges: vec![20..31], // end is exclusive, so 30+1
        });

        assert!(
            result.contains(&expected1),
            "Result should contain file1 fragment"
        );
        assert!(
            result.contains(&expected2),
            "Result should contain file2 fragment"
        );
    });
}

#[test]
fn test_multiple_fragments_same_file() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test with multiple fragments in the same file
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());

        let file_path = "/path/to/file.rs";

        let byte_range1 = ByteOffset::from(0)..ByteOffset::from(50);
        let byte_range2 = ByteOffset::from(100)..ByteOffset::from(150);

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file_path,
            byte_range1.clone(),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file_path,
            byte_range2.clone(),
        );

        let metadata1 = create_test_metadata(file_path, byte_range1, 5, 15);
        let metadata2 = create_test_metadata(file_path, byte_range2, 30, 40);

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([(hash1, vec![metadata1]), (hash2, vec![metadata2])]),
            &mut app,
        );

        let result = mock_index.process_fragments(vec![fragment1, fragment2], 0);

        assert_eq!(
            result.len(),
            1,
            "Should have one result for fragments in the same file"
        );

        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![5..16, 30..41], // Both ranges should be present
        });

        assert!(
            result.contains(&expected),
            "Result should contain both ranges in one fragment"
        );
    });
}

#[test]
fn test_context_lines_non_overlapping() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test context lines with non-overlapping fragments
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());

        let file_path = "/path/to/file.rs";

        let byte_range1 = ByteOffset::from(0)..ByteOffset::from(50);
        let byte_range2 = ByteOffset::from(100)..ByteOffset::from(150);

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file_path,
            byte_range1.clone(),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file_path,
            byte_range2.clone(),
        );

        let metadata1 = create_test_metadata(file_path, byte_range1, 10, 15);
        let metadata2 = create_test_metadata(file_path, byte_range2, 30, 35);

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([(hash1, vec![metadata1]), (hash2, vec![metadata2])]),
            &mut app,
        );

        // Add 2 context lines
        let result = mock_index.process_fragments(vec![fragment1, fragment2], 2);

        assert_eq!(
            result.len(),
            1,
            "Should have one result for fragments in the same file"
        );

        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![8..18, 28..38], // Both ranges expanded by 2 lines
        });

        assert!(
            result.contains(&expected),
            "Result should contain expanded ranges"
        );
    });
}

#[test]
fn test_context_lines_overlapping() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test context lines with fragments that overlap after expansion
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());

        let file_path = "/path/to/file.rs";

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file_path,
            ByteOffset::from(0)..ByteOffset::from(50),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
        );

        let metadata1 =
            create_test_metadata(file_path, ByteOffset::from(0)..ByteOffset::from(50), 10, 15);
        let metadata2 = create_test_metadata(
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
            20,
            25,
        ); // Closer to first fragment

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([(hash1, vec![metadata1]), (hash2, vec![metadata2])]),
            &mut app,
        );

        // Add 5 context lines - this will make the ranges overlap
        let result = mock_index.process_fragments(vec![fragment1, fragment2], 5);

        assert_eq!(
            result.len(),
            1,
            "Should have one result for fragments in the same file"
        );

        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![5..31], // Ranges should be merged: 10-5..15+1+5 and 20-5..25+1+5 = 5..21 and 15..31 -> merged to 5..31
        });

        assert!(
            result.contains(&expected),
            "Result should contain merged overlapping ranges"
        );
    });
}

#[test]
fn test_adjacent_ranges() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test adjacent ranges that should be merged
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());

        let file_path = "/path/to/file.rs";

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file_path,
            ByteOffset::from(0)..ByteOffset::from(50),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
        );

        let metadata1 =
            create_test_metadata(file_path, ByteOffset::from(0)..ByteOffset::from(50), 10, 15);
        let metadata2 = create_test_metadata(
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
            16,
            20,
        ); // Adjacent to first range

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([(hash1, vec![metadata1]), (hash2, vec![metadata2])]),
            &mut app,
        );
        let result = mock_index.process_fragments(vec![fragment1, fragment2], 0);

        assert_eq!(
            result.len(),
            1,
            "Should have one result for fragments in the same file"
        );

        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![10..21], // Ranges should be merged: 10..16 and 16..21 -> 10..21
        });

        assert!(
            result.contains(&expected),
            "Result should contain merged adjacent ranges"
        );
    });
}

#[test]
fn test_complex_overlapping_ranges() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test complex case with multiple overlapping ranges
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());
        let hash3 = MerkleHash::from_bytes("fragment3".as_bytes());

        let file_path = "/path/to/file.rs";

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file_path,
            ByteOffset::from(0)..ByteOffset::from(50),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
        );
        let fragment3 = create_test_fragment(
            "fragment3",
            ContentHash::new(hash3.clone()),
            file_path,
            ByteOffset::from(200)..ByteOffset::from(250),
        );

        let metadata1 =
            create_test_metadata(file_path, ByteOffset::from(0)..ByteOffset::from(50), 10, 20);
        let metadata2 = create_test_metadata(
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
            30,
            40,
        );
        let metadata3 = create_test_metadata(
            file_path,
            ByteOffset::from(200)..ByteOffset::from(250),
            25,
            35,
        ); // Overlaps with fragment2

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([
                (hash1, vec![metadata1]),
                (hash2, vec![metadata2]),
                (hash3, vec![metadata3]),
            ]),
            &mut app,
        );

        // Add 2 context lines
        let result = mock_index.process_fragments(vec![fragment1, fragment2, fragment3], 2);

        assert_eq!(
            result.len(),
            1,
            "Should have one result for fragments in the same file"
        );

        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![8..43], // After merging: 8..43
        });

        assert!(
            result.contains(&expected),
            "Result should contain properly merged ranges"
        );
    });
}

#[test]
fn test_nonsequential_fragments_sorting() {
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);

        // Test complex case with multiple overlapping ranges
        let hash1 = MerkleHash::from_bytes("fragment1".as_bytes());
        let hash2 = MerkleHash::from_bytes("fragment2".as_bytes());
        let hash3 = MerkleHash::from_bytes("fragment3".as_bytes());

        let file_path = "/path/to/file.rs";

        let fragment1 = create_test_fragment(
            "fragment1",
            ContentHash::new(hash1.clone()),
            file_path,
            ByteOffset::from(200)..ByteOffset::from(250),
        );
        let fragment2 = create_test_fragment(
            "fragment2",
            ContentHash::new(hash2.clone()),
            file_path,
            ByteOffset::from(0)..ByteOffset::from(50),
        );
        let fragment3 = create_test_fragment(
            "fragment3",
            ContentHash::new(hash3.clone()),
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
        );

        // Define metadata with line ranges out of order [30-40, 10-20, 20-30]
        let metadata1 = create_test_metadata(
            file_path,
            ByteOffset::from(200)..ByteOffset::from(250),
            30,
            40,
        );
        let metadata2 =
            create_test_metadata(file_path, ByteOffset::from(0)..ByteOffset::from(50), 10, 20);
        let metadata3 = create_test_metadata(
            file_path,
            ByteOffset::from(100)..ByteOffset::from(150),
            20,
            30,
        );

        let mock_index = CodebaseIndex::new_for_test(
            HashMap::from([
                (hash1, vec![metadata1]),
                (hash2, vec![metadata2]),
                (hash3, vec![metadata3]),
            ]),
            &mut app,
        );

        // Process fragments (intentionally in a different order from their line numbers)
        let result = mock_index.process_fragments(vec![fragment1, fragment2, fragment3], 0);

        assert_eq!(
            result.len(),
            1,
            "Should have one result for fragments in the same file"
        );

        // After sorting and merging, we should get one continuous range 10..41
        let expected = CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(file_path),
            line_ranges: vec![10..41], // All three ranges combined into one
        });

        assert!(
            result.contains(&expected),
            "Result should contain properly sorted and merged ranges"
        );
    });
}

#[test]
fn test_diff_merkle_node_no_diffs() {
    VirtualFS::test("test_diff_merkle_node_no_diffs", |dirs, mut sandbox| {
        let repo_name = "warp-virtual";
        let repo_path = dirs.tests().join(repo_name);

        // Initialize repo:
        // warp-virtual/
        // ├── foo
        // └── bar
        sandbox.mkdir(repo_name);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/foo").as_str(),
            "back to the footure",
        )]);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/bar").as_str(),
            "raise the bar",
        )]);

        // Construct merkle tree
        let build_file_tree_result =
            block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
        let (tree, _) = block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

        let mut changed_files = ChangedFiles::default();
        let mut gitignores = vec![];

        // Diff with no changes
        let result = CodebaseIndex::diff_merkle_node(
            &mut changed_files,
            &tree.root_node(),
            repo_path.clone(),
            &mut gitignores,
            None,
            MAX_DEPTH,
            0,
        );
        assert!(result.is_ok(), "Should not error when no changes");
        assert!(changed_files.is_empty(), "Should not detect any changes");
    });
}

#[test]
fn test_diff_merkle_node_new_file() {
    VirtualFS::test("test_diff_merkle_node_new_file", |dirs, mut sandbox| {
        let repo_name = "warp-virtual";

        // Initialize repo:
        // warp-virtual/
        // ├── foo
        // └── bar
        sandbox.mkdir(repo_name);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/foo").as_str(),
            "back to the footure",
        )]);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/bar").as_str(),
            "raise the bar",
        )]);

        let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

        // Construct merkle tree
        let build_file_tree_result =
            block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
        let (tree, _) = block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

        // Add a new file:
        // warp-virtual/
        // ├── foo
        // └── bar
        // └── baz
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/baz").as_str(),
            "the baz is back",
        )]);

        // Diff with new file
        let mut changed_files = ChangedFiles::default();
        let mut gitignores = vec![];
        let result = CodebaseIndex::diff_merkle_node(
            &mut changed_files,
            &tree.root_node(),
            repo_path.clone(),
            &mut gitignores,
            None,
            MAX_DEPTH,
            0,
        );
        assert!(result.is_ok(), "Should not error when adding file");
        assert!(
            changed_files.upsertions.contains(&repo_path.join("baz")),
            "Should detect new file"
        );
    });
}

#[test]
fn test_diff_merkle_node_new_empty_subdirectory() {
    VirtualFS::test(
        "test_diff_merkle_node_new_empty_subdirectory",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "back to the footure",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Add a new subdirectory:
            // warp-virtual/
            // ├── foo
            // └── bar
            // └── subdir/
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());

            // Diff with new subdirectory:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(result.is_ok(), "Should not error when adding subdirectory");
            assert!(changed_files.is_empty(), "Should not detect any changes");
        },
    );
}

#[test]
fn test_diff_merkle_node_new_subdirectory_with_file() {
    VirtualFS::test(
        "test_diff_merkle_node_new_subdirectory_with_file",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "back to the footure",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Add a new subdirectory with a file:
            // warp-virtual/
            // ├── foo
            // └── bar
            // └── subdir/
            //     └── baz
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/subdir/baz").as_str(),
                "the baz is back",
            )]);

            // Diff with new subdirectory
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(result.is_ok(), "Should not error when adding subdirectory");
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join("subdir/baz")),
                "Should detect new file in subdirectory"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_deleted_file() {
    VirtualFS::test("test_diff_merkle_node_deleted_file", |dirs, mut sandbox| {
        let repo_name = "warp-virtual";

        // Initialize repo:
        // warp-virtual/
        // ├── foo
        // └── bar
        sandbox.mkdir(repo_name);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/foo").as_str(),
            "back to the footure",
        )]);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/bar").as_str(),
            "raise the bar",
        )]);

        let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

        // Construct merkle tree
        let build_file_tree_result =
            block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
        let (tree, _) = block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

        // Delete foo:
        // warp-virtual/
        // └── bar
        std::fs::remove_file(repo_path.join("foo")).expect("can not remove file");

        // Diff with deleted foo:
        let mut changed_files = ChangedFiles::default();
        let mut gitignores = vec![];
        let result = CodebaseIndex::diff_merkle_node(
            &mut changed_files,
            &tree.root_node(),
            repo_path.clone(),
            &mut gitignores,
            None,
            MAX_DEPTH,
            0,
        );
        assert!(result.is_ok(), "Should not error when deleting file");
        assert!(
            changed_files.deletions.contains(&repo_path.join("foo")),
            "Should detect deleted file, expected: {:?}, actual: {:?}",
            repo_path.join("foo"),
            changed_files.deletions
        );
    });
}

#[test]
fn test_diff_merkle_node_deleted_empty_subdirectory() {
    VirtualFS::test(
        "test_diff_merkle_node_deleted_empty_subdirectory",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo with two files:
            // warp-virtual/
            // ├── subdir/
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Delete subdir:
            // warp-virtual/
            // └── bar
            std::fs::remove_dir_all(repo_path.join("subdir")).expect("can not remove subdirectory");

            // Diff with deleted subdirectory:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when deleting subdirectory"
            );
            assert!(changed_files.is_empty(), "Should not detect any changes");
        },
    );
}

#[test]
fn test_diff_merkle_node_deleted_subdirectory_with_file() {
    VirtualFS::test(
        "test_diff_merkle_node_deleted_subdirectory_with_file",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo with two files:
            // warp-virtual/
            // ├── subdir/
            // │   └── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/subdir/foo").as_str(),
                "kung foo fighting",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Delete subdir:
            // warp-virtual/
            // └── bar
            std::fs::remove_dir_all(repo_path.join("subdir")).expect("can not remove subdirectory");

            // Diff with deleted subdirectory:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when deleting subdirectory"
            );
            assert!(
                changed_files
                    .deletions
                    .contains(&repo_path.join("subdir/foo")),
                "Should detect deleted file in subdirectory"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_moved_file() {
    VirtualFS::test("test_diff_merkle_node_moved_file", |dirs, mut sandbox| {
        let repo_name = "warp-virtual";

        // Initialize repo with two files:
        // warp-virtual/
        // ├── foo
        // └── bar
        sandbox.mkdir(repo_name);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/foo").as_str(),
            "back to the footure",
        )]);
        sandbox.with_files(vec![Stub::FileWithContent(
            format!("{repo_name}/bar").as_str(),
            "raise the bar",
        )]);

        let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

        // Construct merkle tree
        let build_file_tree_result =
            block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
        let (tree, _) = block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

        // Rename bar to baz:
        // warp-virtual/
        // ├── foo
        // └── baz
        std::fs::rename(repo_path.join("bar"), repo_path.join("baz")).expect("can not rename file");

        // Diff with renamed bar:
        let mut changed_files = ChangedFiles::default();
        let mut gitignores = vec![];
        let result = CodebaseIndex::diff_merkle_node(
            &mut changed_files,
            &tree.root_node(),
            repo_path.clone(),
            &mut gitignores,
            None,
            MAX_DEPTH,
            0,
        );
        assert!(result.is_ok(), "Should not error when renaming file");
        assert!(
            changed_files.deletions.contains(&repo_path.join("bar")),
            "Should detect deleted file"
        );
        assert!(
            changed_files.upsertions.contains(&repo_path.join("baz")),
            "Should detect renamed file"
        );
    });
}

#[test]
fn test_diff_merkle_node_moved_subdirectory_with_file() {
    VirtualFS::test(
        "test_diff_merkle_node_moved_subdirectory_with_file",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── subdir/
            // │   └── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/subdir/foo").as_str(),
                "kung foo panda",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Rename subdir to otherdir:
            // warp-virtual/
            // ├── otherdir/
            // │   └── foo
            // └── bar
            std::fs::rename(repo_path.join("subdir"), repo_path.join("otherdir"))
                .expect("can not rename subdirectory");

            // Diff with renamed subdirectory:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when renaming subdirectory"
            );
            assert!(
                changed_files
                    .deletions
                    .contains(&repo_path.join("subdir/foo")),
                "Should detect deleted file in subdirectory"
            );
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join("otherdir/foo")),
                "Should detect renamed file in subdirectory"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_changed_to_empty_subdirectory() {
    VirtualFS::test(
        "test_diff_merkle_node_file_changed_to_empty_subdirectory",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "back to the footure",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Delete foo, and create a subdirectory with the same name:
            // warp-virtual/
            // ├── foo/
            // └── bar
            std::fs::remove_file(repo_path.join("foo")).expect("can not remove file");
            sandbox.mkdir(format!("{repo_name}/foo").as_str());

            // Diff with changed foo:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing file to subdirectory"
            );
            assert!(
                changed_files.deletions.contains(&repo_path.join("foo")),
                "Should detect deleted file"
            );
            assert!(
                changed_files.upsertions.is_empty(),
                "Should not detect any upsertions"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_changed_to_non_empty_subdirectory() {
    VirtualFS::test(
        "test_diff_merkle_node_file_changed_to_non_empty_subdirectory",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "back to the footure",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Delete foo, and create a subdirectory with the same name:
            // warp-virtual/
            // ├── foo/
            // │   └── bar
            // └── bar
            std::fs::remove_file(repo_path.join("foo")).expect("can not remove file");
            sandbox.mkdir(format!("{repo_name}/foo").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo/bar").as_str(),
                "foobar",
            )]);

            // Diff with changed foo:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing file to subdirectory"
            );
            assert!(
                changed_files.deletions.contains(&repo_path.join("foo")),
                "Should detect deleted file"
            );
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join("foo/bar")),
                "Should detect new file in subdirectory"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_subdirectory_changed_to_file() {
    VirtualFS::test(
        "test_diff_merkle_node_subdirectory_changed_to_file",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── subdir/
            // │   └── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/subdir/foo").as_str(),
                "kung foo panda",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Delete subdir, and create a file with the same name:
            // warp-virtual/
            // ├── subdir
            // └── bar
            std::fs::remove_dir_all(repo_path.join("subdir")).expect("can not remove subdirectory");
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/subdir").as_str(),
                "subdir but not a directory",
            )]);

            // Diff with changed subdir:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing subdirectory to file"
            );
            assert!(
                changed_files
                    .deletions
                    .contains(&repo_path.join("subdir/foo")),
                "Should detect deleted file in subdirectory"
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("subdir")),
                "Should detect new file"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_content_changed() {
    VirtualFS::test(
        "test_diff_merkle_node_file_content_changed",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "original content",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "bar content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Change content of foo file:
            // warp-virtual/
            // ├── foo (content changed)
            // └── bar
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "modified content",
            )]);

            // Diff with changed content:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing file content"
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("foo")),
                "Should detect file content change as upsert"
            );
            assert!(
                changed_files.deletions.is_empty(),
                "Should not detect file content change as deletion"
            );
            assert!(
                changed_files.upsertions.len() == 1,
                "Should detect exactly one changed file"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_content_changed_but_file_size_unchanged() {
    VirtualFS::test(
        "test_diff_merkle_node_file_content_changed_but_file_size_unchanged",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "content0",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "bar content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Change content of foo file:
            // warp-virtual/
            // ├── foo (content changed)
            // └── bar
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "content1",
            )]);

            // Diff with changed content:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing file content, {:?}",
                result.err()
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("foo")),
                "Should detect file content change as upsert"
            );
            assert!(
                changed_files.deletions.is_empty(),
                "Should not detect file content change as deletion"
            );
            assert!(
                changed_files.upsertions.len() == 1,
                "Should detect exactly one changed file"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_multiple_files_changed() {
    VirtualFS::test(
        "test_diff_merkle_node_multiple_files_changed",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── foo
            // ├── bar
            // └── baz
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo content",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "bar content",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/baz").as_str(),
                "baz content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Change multiple files simultaneously:
            // warp-virtual/
            // ├── foo (content changed)
            // ├── bar (content changed)
            // └── baz (unchanged)
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "modified foo content",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "modified bar content",
            )]);

            // Diff with multiple changed files:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing multiple files"
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("foo")),
                "Should detect upsertion of foo"
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("bar")),
                "Should detect upsertion of bar"
            );
            assert!(
                changed_files.upsertions.len() == 2,
                "Should detect exactly two changed files"
            );
            assert!(
                changed_files.deletions.is_empty(),
                "Should not detect any deletions for content changes"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_gitignore_file_changed() {
    VirtualFS::test(
        "test_diff_merkle_node_gitignore_file_changed",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // ├── subdir/
            // │   └── baz
            // ├── foo
            // └── bar
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/subdir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/subdir/baz").as_str(),
                "baz",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "back to the footure",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/bar").as_str(),
                "raise the bar",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Add a gitignore file to root directory:
            // warp-virtual/
            // ├── .gitignore
            // ├── subdir/
            // │   └── baz
            // ├── foo
            // └── bar
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/.gitignore").as_str(),
                "foo", // This should be ignored
            )]);

            // Diff with changed gitignore file:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing gitignore file"
            );
            assert!(
                changed_files.deletions.contains(&repo_path.join("foo")),
                "Should detect deleted file"
            );
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join(".gitignore")),
                "Should detect new gitignore file"
            );

            // Overwrite gitignore file to ignore baz
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/.gitignore").as_str(),
                "baz", // This should be ignored
            )]);

            // Diff with changed gitignore file:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when changing gitignore file"
            );
            assert!(
                changed_files
                    .deletions
                    .contains(&repo_path.join("subdir/baz")),
                "Should detect ignored file in subdirectory"
            );
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join(".gitignore")),
                "Should detect new gitignore file"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_node_with_no_children() {
    VirtualFS::test(
        "test_diff_merkle_node_file_node_with_no_children",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // └── foo
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            let snapshot_str = format!(
                r#"
{{
  "tree": {{
    "root": {{
      "hash": "6fa4ca57b106db11ccee6345326f665b90bc181640e9d9c4b7327692eb043608",
      "children": [
        {{
          "hash": "c7ade88fc7a21498a6a5e5c385e1f68bed822b72aa63c4a9a48a02c2466ee29e",
          "children": [],
          "fs_info": {{
            "File": {{
              "absolute_path": "{foo_path}",
              "file_size": 3,
              "fs_modified_time": "2025-05-22T06:11:15.185189373Z",
              "file_contents_hash": "b5bb9d8014a0f9b1d61e21e796d78dccdf1352f23cd32812f4850b878ae4944c"
            }}
          }}
        }}
      ],
      "fs_info": {{
        "Directory": {{
          "absolute_path": "{repo_path}"
        }}
      }}
    }}
  }}
}}
            "#,
                foo_path = dunce::canonicalize(repo_path.join("foo"))
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "\\\\"), // Escape backslashes in the path for windows
                repo_path = repo_path.display().to_string().replace('\\', "\\\\"), // Escape backslashes in the path for windows
            );

            // Construct merkle tree from snapshot with invalid structure (file node with no fragment children)
            let (tree, _) = block_on(CodebaseIndex::deserialize_snapshot(
                snapshot_str.into_bytes(),
            ))
            .unwrap();

            // Diff with changed file:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when diffing merkle tree with invalid structure, {:?}",
                result.err()
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("foo")),
                "Should detect upsertion of foo"
            );
            assert!(
                changed_files.upsertions.len() == 1,
                "Should detect exactly one changed file"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_node_with_fragment_children_with_children() {
    VirtualFS::test(
        "test_diff_merkle_node_file_node_with_fragment_children_with_children",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo:
            // warp-virtual/
            // └── foo
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            let snapshot_str = format!(
                r#"
{{
  "tree": {{
    "root": {{
      "hash": "6fa4ca57b106db11ccee6345326f665b90bc181640e9d9c4b7327692eb043608",
      "children": [
        {{
          "hash": "c7ade88fc7a21498a6a5e5c385e1f68bed822b72aa63c4a9a48a02c2466ee29e",
          "children": [
            {{
              "hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
              "children": [
                {{
                  "hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                  "children": [],
                  "fs_info": {{
                    "Fragment": {{
                      "location": {{
                        "start_line": 0,
                        "end_line": 0,
                        "byte_range": {{
                          "start": 0,
                          "end": 3
                        }}
                      }}
                    }}
                  }}
                }}
              ],
              "fs_info": {{
                "Fragment": {{
                  "location": {{
                    "start_line": 0,
                    "end_line": 0,
                    "byte_range": {{
                      "start": 0,
                      "end": 3
                    }}
                  }}
                }}
              }}
            }}
          ],
          "fs_info": {{
            "File": {{
              "absolute_path": "{foo_path}",
              "file_size": 3,
              "fs_modified_time": "2025-05-22T06:11:15.185189373Z",
              "file_contents_hash": "b5bb9d8014a0f9b1d61e21e796d78dccdf1352f23cd32812f4850b878ae4944c"
            }}
          }}
        }}
      ],
      "fs_info": {{
        "Directory": {{
          "absolute_path": "{repo_path}"
        }}
      }}
    }}
  }}
}}
                "#,
                foo_path = dunce::canonicalize(repo_path.join("foo"))
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "\\\\"), // Escape backslashes in the path for windows
                repo_path = repo_path.display().to_string().replace('\\', "\\\\"), // Escape backslashes in the path for windows
            );

            // Construct merkle tree from snapshot with invalid structure (file node with fragment children with children)
            let (tree, _) = block_on(CodebaseIndex::deserialize_snapshot(
                snapshot_str.into_bytes(),
            ))
            .unwrap();

            // Diff with changed file:
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                MAX_DEPTH,
                0,
            );
            assert!(
                result.is_ok(),
                "Should not error when diffing merkle tree with invalid structure, {:?}",
                result.err()
            );
            assert!(
                changed_files.upsertions.contains(&repo_path.join("foo")),
                "Should detect upsertion of foo"
            );
            assert!(
                changed_files.upsertions.len() == 1,
                "Should detect exactly one changed file"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_max_depth_exceeded() {
    VirtualFS::test(
        "test_diff_merkle_node_max_depth_exceeded",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize a simple repo:
            // warp-virtual/
            // └── foo
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Try diffing with max_depth set to 0 (should fail immediately)
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                0, // max_depth = 0
                0, // current_depth = 0, when we recurse from root to its children, max depth check should fail
            );

            assert!(result.is_err(), "Should error when max depth is exceeded");
            if let Err(error) = result {
                match error {
                    crate::index::full_source_code_embedding::Error::DiffMerkleTreeError(
                        crate::index::full_source_code_embedding::DiffMerkleTreeError::MaxDepthExceeded,
                    ) => {
                        // Expected error type
                    }
                    _ => panic!("Expected MaxDepthExceeded error, got: {error:?}"),
                }
            }
        },
    );
}

#[test]
fn test_diff_merkle_node_max_depth_boundary() {
    VirtualFS::test(
        "test_diff_merkle_node_max_depth_boundary",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize a simple repo:
            // warp-virtual/
            // └── foo
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Test the actual boundary: current_depth == max_depth should succeed
            // but recursion to current_depth + 1 should fail
            // Use max_depth = 1 with current_depth = 0, so recursion to depth 1 succeeds
            // but further recursion to depth 2 would fail (if there were deeper directories)
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                1, // max_depth = 1 (allows root=0 and children=1)
                0, // current_depth = 0 (root level)
            );

            assert!(
                result.is_ok(),
                "Should succeed when recursion stays within max_depth"
            );
        },
    );
}

#[test]
fn test_diff_merkle_node_file_limit_exceeded() {
    VirtualFS::test(
        "test_diff_merkle_node_file_limit_exceeded",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo with multiple files:
            // warp-virtual/
            // └── foo
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Try diffing with file quota set to 0 (should fail when processing files)
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let mut file_quota = 0usize;
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                Some(&mut file_quota),
                MAX_DEPTH,
                0,
            );

            assert!(result.is_err(), "Should error when file limit is exceeded");
            if let Err(error) = result {
                match error {
                    crate::index::full_source_code_embedding::Error::DiffMerkleTreeError(
                        crate::index::full_source_code_embedding::DiffMerkleTreeError::ExceededMaxFileLimit,
                    ) => {
                        // Expected error type
                    }
                    _ => panic!("Expected ExceededMaxFileLimit error, got: {error:?}"),
                }
            }
        },
    );
}

#[test]
fn test_diff_merkle_node_file_limit_boundary() {
    VirtualFS::test(
        "test_diff_merkle_node_file_limit_boundary",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize repo with one file
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/existing_file").as_str(),
                "existing content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Add a new file to trigger file processing
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/new_file").as_str(),
                "new content",
            )]);

            // Try diffing with file quota set to 2 (enough for both existing and new file operations)
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let mut file_quota = 2usize;
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                Some(&mut file_quota),
                MAX_DEPTH,
                0,
            );

            assert!(
                result.is_ok(),
                "Should succeed when file quota allows processing"
            );
            assert!(
                file_quota == 0,
                "File quota should be decremented after processing files"
            );
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join("new_file")),
                "Should detect the new file"
            );
        },
    );
}

#[test]
fn test_add_merkle_node_max_depth_exceeded() {
    VirtualFS::test(
        "test_add_merkle_node_max_depth_exceeded",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize a simple repo with a single file
            // warp-virtual/
            // └── foo
            sandbox.mkdir(repo_name);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/foo").as_str(),
                "foo content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();

            // Construct merkle tree
            let build_file_tree_result =
                block_on(CodebaseIndex::build_file_tree(repo_path.clone(), None)).unwrap();
            let (tree, _) =
                block_on(MerkleTree::try_new(build_file_tree_result.file_tree)).unwrap();

            // Add a deep directory structure to the merkle tree
            // warp-virtual/
            // ├── foo
            // └── deep/
            //     └── nested/
            //         └── dir/
            //             └── file.txt
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/deep").as_str());
            sandbox.mkdir(format!("{repo_name}/deep/nested").as_str());
            sandbox.mkdir(format!("{repo_name}/deep/nested/dir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/deep/nested/dir/file.txt").as_str(),
                "deep file content",
            )]);

            // Diffing the merkle tree will cause it to try and recurse and add the deep directory
            // to the changed files but max depth check should fail.
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                1, // max_depth = 1
                0, // current_depth = 0 (root level)
            );

            assert!(result.is_err(), "Should error when max depth is exceeded");
            if let Err(error) = result {
                match error {
                    crate::index::full_source_code_embedding::Error::DiffMerkleTreeError(
                        crate::index::full_source_code_embedding::DiffMerkleTreeError::MaxDepthExceeded,
                    ) => {
                        // Expected error type
                    }
                    _ => panic!("Expected MaxDepthExceeded error, got: {error:?}"),
                }
            }

            // Try again with max depth set to 4, which should succeed.
            let result = CodebaseIndex::diff_merkle_node(
                &mut changed_files,
                &tree.root_node(),
                repo_path.clone(),
                &mut gitignores,
                None,
                4,
                0,
            );
            assert!(result.is_ok(), "Should succeed when max depth is set to 4");
            assert!(
                changed_files
                    .upsertions
                    .contains(&repo_path.join("deep/nested/dir/file.txt")),
                "Should detect the new file"
            );
        },
    );
}

#[test]
fn test_add_merkle_node_file_limit_exceeded() {
    VirtualFS::test(
        "test_add_merkle_node_file_limit_exceeded",
        |dirs, mut sandbox| {
            let repo_name = "warp-virtual";

            // Initialize a repo with multiple files:
            // warp-virtual/
            // └── dir/
            //     ├── file1.txt
            //     ├── file2.txt
            //     └── file3.txt
            sandbox.mkdir(repo_name);
            sandbox.mkdir(format!("{repo_name}/dir").as_str());
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/dir/file1.txt").as_str(),
                "file1 content",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/dir/file2.txt").as_str(),
                "file2 content",
            )]);
            sandbox.with_files(vec![Stub::FileWithContent(
                format!("{repo_name}/dir/file3.txt").as_str(),
                "file3 content",
            )]);

            let repo_path = dunce::canonicalize(dirs.tests().join(repo_name)).unwrap();
            let dir_path = repo_path.join("dir");

            // Try adding the directory with file quota set to 1 (should fail after processing 1 file)
            let mut changed_files = ChangedFiles::default();
            let mut gitignores = vec![];
            let mut file_quota = 1usize;
            let result = CodebaseIndex::add_merkle_node(
                &mut changed_files,
                &dir_path,
                &mut gitignores,
                Some(&mut file_quota),
                MAX_DEPTH,
                0,
            );

            assert!(result.is_err(), "Should error when file limit is exceeded");
            assert_eq!(file_quota, 0, "File quota should be fully consumed");
            if let Err(error) = result {
                match error {
                    crate::index::full_source_code_embedding::Error::DiffMerkleTreeError(
                        crate::index::full_source_code_embedding::DiffMerkleTreeError::ExceededMaxFileLimit,
                    ) => {
                        // Expected error type
                    }
                    _ => panic!("Expected ExceededMaxFileLimit error, got: {error:?}"),
                }
            }
        },
    );
}
