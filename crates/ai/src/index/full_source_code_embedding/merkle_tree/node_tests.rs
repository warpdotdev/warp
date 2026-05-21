use crate::index::full_source_code_embedding::{
    fragment_metadata::LeafToFragmentMetadataUpdates, merkle_tree::DirEntryOrFragment,
};
use repo_metadata::{DirectoryEntry, Entry};
use virtual_fs::{Stub, VirtualFS};

use std::collections::HashSet;

use super::{MerkleNode, NodeMask};

/// Tests that node hashes for directories are sorted (meaning they are resilient to files within
/// the directory being in a different order).
#[test]
fn test_node_hash_for_directory_is_sorted() {
    VirtualFS::test(
        "test_node_hash_for_directory_is_sorted",
        |dirs, mut sandbox| {
            sandbox.with_files(vec![Stub::FileWithContent("foo", "foo")]);
            sandbox.with_files(vec![Stub::FileWithContent("bar", "bar")]);

            let mut directory_entry = DirectoryEntry {
                path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                    .unwrap(),
                children: vec![],
                ignored: false,
                loaded: false,
            };

            for file in ["foo", "bar"] {
                directory_entry
                    .find_or_insert_child(&dirs.tests().join(file))
                    .expect("Should be able to insert into directory entry");
            }

            let (node, _leaf_to_fragment_updates) = MerkleNode::new(DirEntryOrFragment::Entry(
                Entry::Directory(directory_entry.clone()),
            ))
            .expect("Should be able to construct node");

            directory_entry.children.clear();

            for file in ["bar", "foo"] {
                directory_entry
                    .find_or_insert_child(&dirs.tests().join(file))
                    .expect("Should be able to insert into directory entry");
            }

            let (node_reverse, _leaf_to_fragment_updates) =
                MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(directory_entry)))
                    .expect("Should be able to construct node");

            // The node hashes should be the same even though the files were returned in different orders.
            assert_eq!(node.hash, node_reverse.hash);
        },
    );
}

/// Tests that upserting a file updates the Merkle tree correctly.
#[test]
fn test_merkle_node_upsert_file() {
    VirtualFS::test("test_merkle_node_upsert_file", |dirs, mut sandbox| {
        // Create a directory with an initial file
        sandbox.with_files(vec![Stub::FileWithContent(
            "initial.txt",
            "initial content",
        )]);

        let mut directory_entry = DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        };

        // Add the initial file to the directory entry
        directory_entry
            .find_or_insert_child(&dirs.tests().join("initial.txt"))
            .expect("Should be able to insert into directory entry");

        // Create the initial MerkleNode
        let (mut node, initial_metadata_update) = MerkleNode::new(DirEntryOrFragment::Entry(
            Entry::Directory(directory_entry.clone()),
        ))
        .expect("Should be able to construct node");

        assert_eq!(
            initial_metadata_update.to_insert.len(),
            1,
            "Should insert one file's metadata",
        );
        assert!(
            initial_metadata_update.to_remove.is_empty(),
            "Should not remove any file metadata",
        );

        let initial_root_hash = node.hash.clone();

        // Create a new file to upsert
        sandbox.with_files(vec![Stub::FileWithContent("new.txt", "new content")]);
        directory_entry
            .find_or_insert_child(&dirs.tests().join("new.txt"))
            .expect("Should be able to insert into directory entry");

        // Upsert the new file
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        node.upsert_files(
            &mut HashSet::from([dirs.tests().join("new.txt")]),
            &mut node_path,
            &mut leaf_to_fragment_metadata_updates,
        );

        // Verify the hash changed after adding a new file
        assert_ne!(
            initial_root_hash, node.hash,
            "Hash should change after adding a new file",
        );
        assert_eq!(
            leaf_to_fragment_metadata_updates.to_insert.len(),
            1,
            "Upserting a new file should insert its content into metadata mapping",
        );
        assert!(
            leaf_to_fragment_metadata_updates.to_remove.is_empty(),
            "Inserting a new file should not remove any content from metadata mapping",
        );

        // Updated hash should be the same as reconstructing hash from scratch.
        let hash_from_scratch = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
            directory_entry.clone(),
        )))
        .expect("Should be able to construct node")
        .0
        .hash;
        assert_eq!(node.hash, hash_from_scratch);

        // Remember the hash after adding the new file
        let hash_after_add = node.hash.clone();

        // Modify an existing file
        sandbox.with_files(vec![Stub::FileWithContent(
            "initial.txt",
            "modified content",
        )]);

        // Upsert the modified file
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        node.upsert_files(
            &mut HashSet::from([dirs.tests().join("initial.txt")]),
            &mut node_path,
            &mut leaf_to_fragment_metadata_updates,
        );

        let updated_leaf_hash = leaf_to_fragment_metadata_updates
            .to_insert
            .keys()
            .next()
            .unwrap();
        let updated_metadata_entry =
            leaf_to_fragment_metadata_updates.to_insert[updated_leaf_hash].clone();

        // Verify the hash changed after modifying a file
        assert_ne!(
            hash_after_add, node.hash,
            "Hash should change after modifying a file"
        );

        assert_eq!(
            leaf_to_fragment_metadata_updates.to_insert.len(),
            1,
            "Upserting a modified file should insert its content into metadata mapping",
        );
        assert_eq!(
            leaf_to_fragment_metadata_updates.to_remove.len(),
            1,
            "Upserting a modified file should remove its old content from metadata mapping",
        );

        // Updated hash should be the same as reconstructing hash from scratch.
        let hash_from_scratch = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
            directory_entry.clone(),
        )))
        .expect("Should be able to construct node")
        .0
        .hash;
        assert_eq!(node.hash, hash_from_scratch);

        // Remember the hash after modification
        let hash_after_modify = node.hash.clone();

        // Upsert with the same content (should not change the hash)
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        node.upsert_files(
            &mut HashSet::from([dirs.tests().join("initial.txt")]),
            &mut node_path,
            &mut leaf_to_fragment_metadata_updates,
        );

        // Verify the hash did not change when content is the same
        assert_eq!(
            hash_after_modify, node.hash,
            "Hash should not change when upserting the same content"
        );

        // Verify that we remove and re-insert the file metadata in the leaf mapping.
        assert_eq!(
            leaf_to_fragment_metadata_updates.to_remove.len(),
            1,
            "Should remove an entry from the metadata mapping",
        );
        assert!(
            leaf_to_fragment_metadata_updates
                .to_remove
                .contains_key(&dirs.tests().join("initial.txt")),
            "Upserting the same content should remove the old entry from the metadata mapping",
        );

        assert_eq!(
            leaf_to_fragment_metadata_updates.to_insert.len(),
            1,
            "Should re-insert an entry into the metadata mapping",
        );
        assert!(
            leaf_to_fragment_metadata_updates
                .to_insert
                .contains_key(updated_leaf_hash),
            "Upserting the same content re-inserts the same hash into the metadata mapping",
        );
        assert_eq!(
            leaf_to_fragment_metadata_updates
                .to_insert
                .get(updated_leaf_hash)
                .unwrap(),
            &updated_metadata_entry,
            "Upserting the same content re-inserts the same metadata into the metadata mapping"
        );

        // Test upserting a file in a new subdirectory that doesn't exist yet
        let hash_before_subdirectory = node.hash.clone();

        // Create a new subdirectory structure with a file
        sandbox.mkdir("a");
        sandbox.with_files(vec![Stub::FileWithContent(
            "a/b.txt",
            "subdirectory file content",
        )]);

        // Upsert the file in the new subdirectory
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        node.upsert_files(
            &mut HashSet::from([dirs.tests().join("a/b.txt")]),
            &mut node_path,
            &mut leaf_to_fragment_metadata_updates,
        );

        // Verify the hash changed after adding a file in a new subdirectory
        assert_ne!(
            hash_before_subdirectory, node.hash,
            "Hash should change after adding a file in a new subdirectory"
        );

        assert_eq!(
            leaf_to_fragment_metadata_updates.to_insert.len(),
            1,
            "Upserting a file in a new subdirectory should insert its content into metadata mapping"
        );
        assert!(
            leaf_to_fragment_metadata_updates.to_remove.is_empty(),
            "Upserting a file in a new subdirectory should not remove any content from metadata mapping"
        );

        // Create a new MerkleTree with the expected structure manually
        let mut expected_directory_entry = DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        };

        // Add both the existing files and new subdirectory structure
        for file in ["initial.txt", "new.txt"] {
            expected_directory_entry
                .find_or_insert_child(&dirs.tests().join(file))
                .expect("Should be able to insert into directory entry");
        }

        // Create a subdirectory entry for 'a'
        let mut subdirectory_entry = DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(
                &dirs.tests().join("a"),
            )
            .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        };

        // Add the 'b.txt' file to the subdirectory
        subdirectory_entry
            .find_or_insert_child(&dirs.tests().join("a/b.txt"))
            .expect("Should be able to insert into subdirectory entry");

        // Add the subdirectory to the main directory
        expected_directory_entry
            .children
            .push(Entry::Directory(subdirectory_entry));

        // Create a MerkleNode with the expected structure
        let (expected_node, _) = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
            expected_directory_entry,
        )))
        .expect("Should be able to construct node with expected structure");

        // The hash of our modified node should match the hash of the manually constructed node
        assert_eq!(
            node.hash, expected_node.hash,
            "Hash of node with upserted subdirectory should match hash of node constructed with expected structure"
        );
    });
}

/// Tests that removing a file updates the Merkle tree correctly.
#[test]
fn test_merkle_node_remove_file() {
    VirtualFS::test("test_merkle_node_remove_file", |dirs, mut sandbox| {
        // Create a directory with multiple files
        sandbox.with_files(vec![
            Stub::FileWithContent("file1.txt", "content 1"),
            Stub::FileWithContent("file2.txt", "content 2"),
            Stub::FileWithContent("file3.txt", "content 3"),
        ]);

        let mut directory_entry = DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        };

        // Add all files to the directory entry
        for file in ["file1.txt", "file2.txt", "file3.txt"] {
            directory_entry
                .find_or_insert_child(&dirs.tests().join(file))
                .expect("Should be able to insert into directory entry");
        }

        // Create the initial MerkleNode
        let (mut node, initial_metadata_updates) = MerkleNode::new(DirEntryOrFragment::Entry(
            Entry::Directory(directory_entry.clone()),
        ))
        .expect("Should be able to construct node");

        assert_eq!(
            initial_metadata_updates.to_insert.len(),
            3,
            "Should insert three files' metadata"
        );
        assert!(
            initial_metadata_updates.to_remove.is_empty(),
            "Should not remove any file metadata"
        );

        let initial_hash = node.hash.clone();

        // Remove one of the files
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        node.remove_files(
            &mut HashSet::from([dirs.tests().join("file2.txt")]),
            &mut node_path,
            &mut leaf_to_fragment_metadata_updates,
        );

        // Verify the hash changed after removing a file
        assert_ne!(
            initial_hash, node.hash,
            "Hash should change after removing a file"
        );

        // Verify that we update the fragment metadata
        assert_eq!(
            leaf_to_fragment_metadata_updates.to_remove.len(),
            1,
            "Removing a file should remove its content from metadata mapping"
        );
        assert!(
            leaf_to_fragment_metadata_updates.to_insert.is_empty(),
            "Removing a file should not insert any new metadata"
        );

        // Remember the hash after removal
        let hash_after_remove = node.hash.clone();

        // Try removing a non-existent file (should not change the hash)
        let mut node_path = NodeMask::default();
        let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
        node.remove_files(
            &mut HashSet::from([dirs.tests().join("nonexistent.txt")]),
            &mut node_path,
            &mut leaf_to_fragment_metadata_updates,
        );

        // Verify the hash did not change when removing a non-existent file
        assert_eq!(
            hash_after_remove, node.hash,
            "Hash should not change when removing a non-existent file"
        );

        // Verify that there are no metadata updates
        assert!(
            leaf_to_fragment_metadata_updates.is_empty(),
            "Metadata updates should be empty after trying to remove a non-existent file"
        );

        // Create a new MerkleNode with only the remaining files
        let mut directory_entry_after_remove = DirectoryEntry {
            path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                .unwrap(),
            children: vec![],
            ignored: false,
            loaded: true,
        };

        for file in ["file1.txt", "file3.txt"] {
            directory_entry_after_remove
                .find_or_insert_child(&dirs.tests().join(file))
                .expect("Should be able to insert into directory entry");
        }

        let (node_after_remove, _) = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
            directory_entry_after_remove,
        )))
        .expect("Should be able to construct node with remaining files");

        // Verify that manually constructing a node without the removed file
        // produces the same hash as removing the file from an existing node
        assert_eq!(
            node.hash, node_after_remove.hash,
            "Hash after remove should match hash of node constructed without the file"
        );
    });
}

/// Tests that upserting and removing multiple files updates the Merkle tree correctly.
#[test]
fn test_merkle_node_multiple_operations() {
    VirtualFS::test(
        "test_merkle_node_multiple_operations",
        |dirs, mut sandbox| {
            // Create a directory with multiple initial files
            sandbox.with_files(vec![
                Stub::FileWithContent("file1.txt", "content 1"),
                Stub::FileWithContent("file2.txt", "content 2"),
                Stub::FileWithContent("file3.txt", "content 3"),
            ]);

            let mut directory_entry = DirectoryEntry {
                path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                    .unwrap(),
                children: vec![],
                ignored: false,
                loaded: true,
            };

            // Add all initial files to the directory entry
            for file in ["file1.txt", "file2.txt", "file3.txt"] {
                directory_entry
                    .find_or_insert_child(&dirs.tests().join(file))
                    .expect("Should be able to insert into directory entry");
            }

            // Create the initial MerkleNode
            let (mut node, initial_metadata_updates) = MerkleNode::new(DirEntryOrFragment::Entry(
                Entry::Directory(directory_entry.clone()),
            ))
            .expect("Should be able to construct node");

            // Ensure the initial leaf-node-to-fragment-metadata updates are correct.
            assert_eq!(
                initial_metadata_updates.to_insert.len(),
                3,
                "Should insert three files' metadata"
            );

            let initial_hash = node.hash.clone();

            // Test 1: Upsert multiple files at once
            // Create multiple new files to upsert
            sandbox.with_files(vec![
                Stub::FileWithContent("file4.txt", "content 4"),
                Stub::FileWithContent("file5.txt", "content 5"),
            ]);
            for file in ["file4.txt", "file5.txt"] {
                directory_entry
                    .find_or_insert_child(&dirs.tests().join(file))
                    .expect("Should be able to insert into directory entry");
            }

            // Upsert multiple files at once
            let mut node_path = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.upsert_files(
                &mut HashSet::from([
                    dirs.tests().join("file4.txt"),
                    dirs.tests().join("file5.txt"),
                ]),
                &mut node_path,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Verify the hash changed after adding multiple files
            assert_ne!(
                initial_hash, node.hash,
                "Hash should change after adding multiple files"
            );

            assert_eq!(
                leaf_to_fragment_metadata_updates.to_insert.len(),
                2,
                "Upserting new files should insert their content into metadata mapping"
            );
            assert!(
                leaf_to_fragment_metadata_updates.to_remove.is_empty(),
                "Upserting new files should not remove content from metadata mapping"
            );

            // Updated hash should be the same as reconstructing hash from scratch.
            let hash_from_scratch = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
                directory_entry.clone(),
            )))
            .expect("Should be able to construct node")
            .0
            .hash;
            assert_eq!(node.hash, hash_from_scratch);

            let hash_after_multiple_add = node.hash.clone();

            // Test 2: Remove multiple files at once
            let mut node_path = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.remove_files(
                &mut HashSet::from([
                    dirs.tests().join("file1.txt"),
                    dirs.tests().join("file3.txt"),
                ]),
                &mut node_path,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Verify the hash changed after removing multiple files
            assert_ne!(
                hash_after_multiple_add, node.hash,
                "Hash should change after removing multiple files"
            );

            assert_eq!(
                leaf_to_fragment_metadata_updates.to_remove.len(),
                2,
                "Removing multiple files should remove their content from metadata mapping"
            );
            assert!(
                leaf_to_fragment_metadata_updates.to_insert.is_empty(),
                "Removing multiple files should not insert any new metadata"
            );

            let mut directory_entry = DirectoryEntry {
                path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                    .unwrap(),
                children: vec![],
                ignored: false,
                loaded: true,
            };
            for file in ["file2.txt", "file4.txt", "file5.txt"] {
                directory_entry
                    .find_or_insert_child(&dirs.tests().join(file))
                    .expect("Should be able to insert into directory entry");
            }

            // Updated hash should be the same as reconstructing hash from scratch.
            let hash_from_scratch = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
                directory_entry.clone(),
            )))
            .expect("Should be able to construct node")
            .0
            .hash;
            assert_eq!(node.hash, hash_from_scratch);

            let hash_after_multiple_remove = node.hash.clone();

            // Test 3: Mixed operations - modify an existing file and add a new file
            // Modify an existing file
            sandbox.with_files(vec![
                Stub::FileWithContent("file2.txt", "modified content 2"),
                Stub::FileWithContent("file6.txt", "content 6"),
            ]);
            directory_entry
                .find_or_insert_child(&dirs.tests().join("file6.txt"))
                .expect("Should be able to insert into directory entry");

            let mut node_path = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.upsert_files(
                &mut HashSet::from([
                    dirs.tests().join("file2.txt"),
                    dirs.tests().join("file6.txt"),
                ]),
                &mut node_path,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Verify the hash changed after mixed operations
            assert_ne!(
                hash_after_multiple_remove, node.hash,
                "Hash should change after mixed upsert operations"
            );

            assert_eq!(
                leaf_to_fragment_metadata_updates.to_insert.len(),
                2,
                "Upserting modified and new files should insert their content into metadata mapping"
            );
            assert_eq!(
                leaf_to_fragment_metadata_updates.to_remove.len(),
                1,
                "Upserting modified and new files should remove the old content of the modified file from metadata mapping"
            );

            // Updated hash should be the same as reconstructing hash from scratch.
            let hash_from_scratch = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
                directory_entry.clone(),
            )))
            .expect("Should be able to construct node")
            .0
            .hash;
            assert_eq!(node.hash, hash_from_scratch);

            let hash_after_mixed_upsert = node.hash.clone();

            // Test 4: Edge case - upsert a file that already exists with the same content
            let mut node_path = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.upsert_files(
                &mut HashSet::from([dirs.tests().join("file2.txt")]),
                &mut node_path,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Verify hash didn't change when upserting with the same content
            assert_eq!(
                hash_after_mixed_upsert, node.hash,
                "Hash should not change when upserting files with the same content"
            );

            assert_eq!(
                leaf_to_fragment_metadata_updates.to_remove.len(),
                1,
                "Upserting files with the same content first removes it from the metadata mapping"
            );
            assert_eq!(
                leaf_to_fragment_metadata_updates.to_insert.len(),
                1,
                "Upserting files with the same content re-adds it to the metadata mapping"
            );

            // Test 5: Edge case - remove a non-existent file while upserting a new file
            // Create nested directories and a new file
            sandbox.mkdir("subdir1");
            sandbox.mkdir("subdir1/subdir2");
            sandbox.with_files(vec![Stub::FileWithContent(
                "subdir1/subdir2/nested.txt",
                "nested content",
            )]);

            // Do mixed operations - remove non-existent file while upserting a new file in a subdirectory
            let mut node_path_remove = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.remove_files(
                &mut HashSet::from([dirs.tests().join("nonexistent.txt")]),
                &mut node_path_remove,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Hash shouldn't change after trying to remove a non-existent file
            assert_eq!(
                hash_after_mixed_upsert, node.hash,
                "Hash should not change when removing a non-existent file"
            );

            assert!(
                leaf_to_fragment_metadata_updates.is_empty(),
                "Removing a non-existent file should not modify metadata mapping"
            );

            // Add the nested file
            let mut node_path_upsert = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.upsert_files(
                &mut HashSet::from([dirs.tests().join("subdir1/subdir2/nested.txt")]),
                &mut node_path_upsert,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Hash should change after adding the nested file
            assert_ne!(
                hash_after_mixed_upsert, node.hash,
                "Hash should change after adding a file in a nested subdirectory"
            );

            assert_eq!(
                leaf_to_fragment_metadata_updates.to_insert.len(),
                1,
                "Upserting a file in a nested subdirectory should insert its content into metadata mapping",
            );
            assert!(
                leaf_to_fragment_metadata_updates.to_remove.is_empty(),
                "Upserting a file in a nested subdirectory should not remove any files from metadata mapping",
            );

            let hash_after_nested_add = node.hash.clone();

            // Test 6: Mixed operations - remove multiple files and add multiple files at once
            sandbox.with_files(vec![
                Stub::FileWithContent("new_file1.txt", "new content 1"),
                Stub::FileWithContent("new_file2.txt", "new content 2"),
            ]);

            // Remove some files
            let mut node_path_remove = NodeMask::default();
            let mut leaf_to_fragment_metadata_updates = LeafToFragmentMetadataUpdates::empty();
            node.remove_files(
                &mut HashSet::from([
                    dirs.tests().join("file4.txt"),
                    dirs.tests().join("file5.txt"),
                ]),
                &mut node_path_remove,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Add new files
            let mut node_path_upsert = NodeMask::default();
            node.upsert_files(
                &mut HashSet::from([
                    dirs.tests().join("new_file1.txt"),
                    dirs.tests().join("new_file2.txt"),
                ]),
                &mut node_path_upsert,
                &mut leaf_to_fragment_metadata_updates,
            );

            // Verify the hash changed after these mixed operations
            assert_ne!(
                hash_after_nested_add, node.hash,
                "Hash should change after removing and adding multiple files"
            );

            assert_eq!(
                leaf_to_fragment_metadata_updates.to_insert.len(),
                2,
                "Upserting multiple files should insert their content into metadata mapping"
            );
            assert_eq!(
                leaf_to_fragment_metadata_updates.to_remove.len(),
                2,
                "Removing multiple files should remove their content from metadata mapping"
            );

            // Create a new MerkleTree with the expected final structure manually
            let mut expected_directory_entry = DirectoryEntry {
                path: warp_util::standardized_path::StandardizedPath::try_from_local(dirs.tests())
                    .unwrap(),
                children: vec![],
                ignored: false,
                loaded: true,
            };

            // Add all files that should be in the final structure
            for file in ["file2.txt", "file6.txt", "new_file1.txt", "new_file2.txt"] {
                expected_directory_entry
                    .find_or_insert_child(&dirs.tests().join(file))
                    .expect("Should be able to insert into directory entry");
            }

            // Create nested subdirectory structure
            let mut subdir1_entry = DirectoryEntry {
                path: warp_util::standardized_path::StandardizedPath::try_from_local(
                    &dirs.tests().join("subdir1"),
                )
                .unwrap(),
                children: vec![],
                ignored: false,
                loaded: true,
            };

            let mut subdir2_entry = DirectoryEntry {
                path: warp_util::standardized_path::StandardizedPath::try_from_local(
                    &dirs.tests().join("subdir1/subdir2"),
                )
                .unwrap(),
                children: vec![],
                ignored: false,
                loaded: true,
            };

            // Add the nested file to its subdirectory
            subdir2_entry
                .find_or_insert_child(&dirs.tests().join("subdir1/subdir2/nested.txt"))
                .expect("Should be able to insert into nested subdirectory entry");

            // Build the directory hierarchy
            subdir1_entry.children.push(Entry::Directory(subdir2_entry));

            expected_directory_entry
                .children
                .push(Entry::Directory(subdir1_entry));

            // Create a MerkleNode with the expected final structure
            let (expected_node, _) = MerkleNode::new(DirEntryOrFragment::Entry(Entry::Directory(
                expected_directory_entry,
            )))
            .expect("Should be able to construct node with expected structure");

            // The hash of our modified node should match the hash of the manually constructed node
            assert_eq!(
            node.hash, expected_node.hash,
            "Hash after all operations should match hash of node constructed with expected structure"
        );
        },
    );
}
