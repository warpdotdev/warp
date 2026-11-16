use std::collections::HashMap;

use futures::executor::block_on;
use virtual_fs::VirtualFS;

use super::batch_leaves_by_size;
use crate::index::full_source_code_embedding::merkle_tree::{NodeLens, construct_test_merkle_tree};

/// Collect all leaf nodes from a merkle tree by walking it recursively.
fn collect_leaves<'a>(node: NodeLens<'a>) -> Vec<NodeLens<'a>> {
    if node.is_leaf() {
        return vec![node];
    }
    node.children().flat_map(collect_leaves).collect()
}

#[test]
fn test_batch_leaves_single_batch_when_under_limits() {
    VirtualFS::test("batch_single", |dirs, mut sandbox| {
        let (tree, metadata) = block_on(construct_test_merkle_tree(&dirs, &mut sandbox));

        let leaves = collect_leaves(tree.root_node());
        assert!(!leaves.is_empty(), "Tree should have leaf nodes");

        let batches = batch_leaves_by_size(&leaves, metadata.mapping(), 1000, 10_000_000).unwrap();

        assert_eq!(batches.len(), 1, "All leaves should fit in a single batch");
        assert_eq!(
            batches[0].len(),
            leaves.len(),
            "The single batch should contain all leaves"
        );
    });
}

#[test]
fn test_batch_leaves_splits_on_count_limit() {
    VirtualFS::test("batch_count", |dirs, mut sandbox| {
        let (tree, metadata) = block_on(construct_test_merkle_tree(&dirs, &mut sandbox));

        let leaves = collect_leaves(tree.root_node());
        let leaf_count = leaves.len();
        assert!(leaf_count >= 2, "Need at least 2 leaves for this test");

        let batches = batch_leaves_by_size(&leaves, metadata.mapping(), 1, 10_000_000).unwrap();

        assert_eq!(
            batches.len(),
            leaf_count,
            "Each leaf should be in its own batch when max_count=1"
        );
        for batch in &batches {
            assert_eq!(batch.len(), 1);
        }
    });
}

#[test]
fn test_batch_leaves_splits_on_byte_limit() {
    VirtualFS::test("batch_bytes", |dirs, mut sandbox| {
        let (tree, metadata) = block_on(construct_test_merkle_tree(&dirs, &mut sandbox));

        let leaves = collect_leaves(tree.root_node());
        let leaf_count = leaves.len();
        assert!(leaf_count >= 2, "Need at least 2 leaves for this test");

        // max_bytes=1 means every leaf exceeds the limit, but progress guarantee
        // ensures each still gets its own batch.
        let batches = batch_leaves_by_size(&leaves, metadata.mapping(), 1000, 1).unwrap();

        assert_eq!(
            batches.len(),
            leaf_count,
            "Each leaf should be in its own batch when max_bytes=1 (progress guarantee)"
        );
        for batch in &batches {
            assert_eq!(batch.len(), 1);
        }
    });
}

#[test]
fn test_batch_leaves_empty_input() {
    let leaves: Vec<NodeLens<'_>> = vec![];
    let mapping = HashMap::new();

    let batches = batch_leaves_by_size(&leaves, &mapping, 100, 4_000_000).unwrap();
    assert!(
        batches.is_empty(),
        "Empty input should produce empty output"
    );
}

#[test]
fn test_batch_leaves_missing_metadata_returns_error() {
    VirtualFS::test("batch_missing", |dirs, mut sandbox| {
        let (tree, _metadata) = block_on(construct_test_merkle_tree(&dirs, &mut sandbox));

        let leaves = collect_leaves(tree.root_node());
        assert!(!leaves.is_empty());

        // Pass an empty mapping — every leaf lookup should fail.
        let empty_mapping = HashMap::new();
        let result = batch_leaves_by_size(&leaves, &empty_mapping, 1000, 10_000_000);

        assert!(
            result.is_err(),
            "Should return error when metadata is missing"
        );
    });
}
