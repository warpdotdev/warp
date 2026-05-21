use super::MerkleHash;
use crate::index::full_source_code_embedding::chunker::Fragment;
use std::path::{Path, PathBuf};

#[test]
fn test_fragment_hash_from_content() {
    let content = "fn main() { println!(\"Hello, world!\"); }";
    let fragment = Fragment {
        content,
        file_path: Path::new("/foo/bar/bazz"),
        start_line: 0,
        end_line: 0,
        start_byte_index: 0.into(),
        end_byte_index: content.len().into(),
    };
    let hash = MerkleHash::from_fragment(&fragment);

    assert_eq!(
        "bb343b0950832ccd077f1515e842196f2ae4bb9e9261b0935ac57916c3cf305d",
        hash.to_string()
    );
}

#[test]
fn test_node_hash_from_children() {
    let path = PathBuf::from("/foo/bar/bazz");
    let content1 = "fn func1() -> int { 1 }";
    let content2 = "fn func2() -> int { 2 }";
    let content3 = "fn func3() -> int { 3 }";

    let leaf1 = MerkleHash::from_fragment(&Fragment {
        content: content1,
        file_path: path.as_path(),
        start_line: 0,
        end_line: 0,
        start_byte_index: 0.into(),
        end_byte_index: content1.len().into(),
    });
    let leaf2 = MerkleHash::from_fragment(&Fragment {
        content: content2,
        file_path: path.as_path(),
        start_line: 1,
        end_line: 1,
        start_byte_index: content1.len().into(),
        end_byte_index: (content1.len() + content2.len()).into(),
    });
    let leaf3 = MerkleHash::from_fragment(&Fragment {
        content: content3,
        file_path: path.as_path(),
        start_line: 2,
        end_line: 2,
        start_byte_index: (content1.len() + content2.len()).into(),
        end_byte_index: (content1.len() + content2.len() + content3.len()).into(),
    });

    // Create an iterator with the leaf hashes
    let leaves = vec![&leaf1, &leaf2, &leaf3];
    let hash = MerkleHash::from_hashes(leaves.into_iter());

    assert_eq!(
        "99c2f5b808870e4b1fcf163efbb588b6d5e074658d4ac43bab2eb5ffbe72c5cd",
        hash.to_string()
    );
}

#[test]
fn test_merkle_hash_serialization_deserialization() {
    // Create a MerkleHash from a fragment
    let content = "fn main() { println!(\"Hello, world!\"); }";
    let fragment = Fragment {
        content,
        file_path: Path::new("/foo/bar/bazz"),
        start_line: 0,
        end_line: 0,
        start_byte_index: 0.into(),
        end_byte_index: content.len().into(),
    };
    let original_hash = MerkleHash::from_fragment(&fragment);

    // Serialize the hash to a JSON string
    let serialized = serde_json::to_string(&original_hash).expect("Failed to serialize MerkleHash");

    // Ensure serialized output is a hex string
    assert!(serialized.starts_with("\""));
    assert!(serialized.ends_with("\""));

    // Deserialize back to a MerkleHash
    let deserialized_hash: MerkleHash =
        serde_json::from_str(&serialized).expect("Failed to deserialize MerkleHash");

    // Verify deserialized hash matches the original
    assert_eq!(original_hash, deserialized_hash);

    // Also verify string representation matches
    assert_eq!(original_hash.to_string(), deserialized_hash.to_string());
}
