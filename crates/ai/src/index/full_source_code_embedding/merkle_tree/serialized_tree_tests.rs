use futures::executor::block_on;
use serde_json;
use string_offset::ByteOffset;
use virtual_fs::VirtualFS;

use super::{
    SerializedCodebaseIndex, SerializedFilesystemInfo, SerializedFragmentLocation,
    SerializedMerkleNode, SerializedMerkleTree,
};
use crate::index::full_source_code_embedding::merkle_tree::{
    MerkleHash, MerkleTree, construct_test_merkle_tree,
};

#[test]
fn round_trip_index_serialize_deserialize_json() {
    VirtualFS::test("test_nodes_from_path_json", |dirs, mut sandbox| {
        let (original_tree, original_metadata) =
            block_on(construct_test_merkle_tree(&dirs, &mut sandbox));
        let serializable_index = SerializedCodebaseIndex::new(&original_tree, &original_metadata);
        let serializable_index =
            serializable_index.expect("Should successfully construct serializable index");

        let serialized_str =
            serde_json::to_string(&serializable_index).expect("Should serialize to JSON string");
        assert!(!serialized_str.is_empty());

        let deserialized_index: SerializedCodebaseIndex =
            serde_json::from_str(&serialized_str).expect("Should deserialize from JSON");
        assert_eq!(
            deserialized_index, serializable_index,
            "Serialized struct should be identical"
        );

        let (reconstructed_tree, reconstructed_metadata) =
            MerkleTree::from_serialized_tree(deserialized_index.into_tree())
                .expect("Should rebuild Merkle Tree");
        assert_eq!(
            reconstructed_tree.root_node().hash(),
            original_tree.root_node().hash(),
            "Reconstructed Merkle tree should be identical",
        );
        assert_eq!(
            original_metadata, reconstructed_metadata,
            "Reconstructed metadata should be identical"
        );
    })
}

#[test]
fn reconstructed_file_and_fragments_share_their_path() {
    let file_path = std::path::PathBuf::from("repo").join("lib.rs");
    let fragment = |content: &[u8], byte_range| SerializedMerkleNode {
        hash: MerkleHash::from_bytes(content),
        children: vec![],
        fs_info: SerializedFilesystemInfo::Fragment {
            location: SerializedFragmentLocation {
                start_line: 1,
                end_line: 1,
                byte_range,
            },
        },
    };
    let file = SerializedMerkleNode {
        hash: MerkleHash::from_bytes(b"file"),
        children: vec![
            fragment(b"first", ByteOffset::from(0)..ByteOffset::from(5)),
            fragment(b"second", ByteOffset::from(5)..ByteOffset::from(11)),
        ],
        fs_info: SerializedFilesystemInfo::File {
            absolute_path: file_path,
            file_size: 11,
            fs_modified_time: chrono::DateTime::UNIX_EPOCH,
            file_contents_hash: "contents".to_string(),
        },
    };
    let serialized_tree = SerializedMerkleTree {
        root: SerializedMerkleNode {
            hash: MerkleHash::from_bytes(b"root"),
            children: vec![file],
            fs_info: SerializedFilesystemInfo::Directory {
                absolute_path: std::path::PathBuf::from("repo"),
            },
        },
    };

    let (tree, metadata) =
        MerkleTree::from_serialized_tree(serialized_tree).expect("Should rebuild Merkle tree");
    let file = tree.root_node().children().next().unwrap();
    let fragments = file.children().collect::<Vec<_>>();
    let metadata_paths = metadata
        .mapping()
        .values()
        .flatten()
        .map(|metadata| metadata.absolute_path.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(fragments.len(), 2);
    assert!(std::ptr::eq(file.path(), fragments[0].path()));
    assert!(std::ptr::eq(file.path(), fragments[1].path()));
    assert_eq!(metadata_paths.len(), 2);
    assert!(
        metadata_paths
            .iter()
            .all(|path| std::ptr::eq(file.path(), *path))
    );
}

#[test]
fn round_trip_index_serialize_deserialize_bincode() {
    VirtualFS::test("test_nodes_from_path_bincode", |dirs, mut sandbox| {
        let (original_tree, original_metadata) =
            block_on(construct_test_merkle_tree(&dirs, &mut sandbox));
        let serializable_index = SerializedCodebaseIndex::new(&original_tree, &original_metadata);
        let serializable_index =
            serializable_index.expect("Should successfully construct serializable index");

        let serialized_bytes =
            bincode::serialize(&serializable_index).expect("Should serialize to bincode");
        assert!(!serialized_bytes.is_empty());

        // Bincode output should be smaller than JSON
        let json_bytes = serde_json::to_vec(&serializable_index).unwrap();
        assert!(
            serialized_bytes.len() < json_bytes.len(),
            "Bincode ({} bytes) should be smaller than JSON ({} bytes)",
            serialized_bytes.len(),
            json_bytes.len(),
        );

        let deserialized_index: SerializedCodebaseIndex =
            bincode::deserialize(&serialized_bytes).expect("Should deserialize from bincode");
        assert_eq!(
            deserialized_index, serializable_index,
            "Serialized struct should be identical"
        );

        let (reconstructed_tree, reconstructed_metadata) =
            MerkleTree::from_serialized_tree(deserialized_index.into_tree())
                .expect("Should rebuild Merkle Tree");
        assert_eq!(
            reconstructed_tree.root_node().hash(),
            original_tree.root_node().hash(),
            "Reconstructed Merkle tree should be identical",
        );
        assert_eq!(
            original_metadata, reconstructed_metadata,
            "Reconstructed metadata should be identical"
        );
    })
}
