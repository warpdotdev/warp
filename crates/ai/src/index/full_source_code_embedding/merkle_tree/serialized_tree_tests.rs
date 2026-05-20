use futures::executor::block_on;
use serde_json;
use virtual_fs::VirtualFS;

use crate::index::full_source_code_embedding::merkle_tree::{
    construct_test_merkle_tree, MerkleTree,
};

use super::SerializedCodebaseIndex;

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
