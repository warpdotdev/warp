use crate::index::full_source_code_embedding::merkle_tree::{
    construct_test_merkle_tree, node::ChildrenPath,
};
use futures::executor::block_on;

use virtual_fs::VirtualFS;

use super::*;

#[test]
fn test_nodes_from_path() {
    VirtualFS::test("test_nodes_from_path", |dirs, mut sandbox| {
        let (tree, _metadata) = block_on(construct_test_merkle_tree(&dirs, &mut sandbox));

        // Test: path with single child
        let single_path = NodeMask {
            index: 0,
            children: ChildrenPath::SpecificChildren(vec![NodeMask::new(0)]),
        };
        let result = tree.nodes_from_mask(single_path).expect("Should not fail");
        assert_eq!(
            result.len(),
            3,
            "Should return three nodes for single-level path: root, child and its children"
        );

        // Test: path with multiple levels
        // top_dir/file1.txt
        let multi_path = NodeMask {
            index: 0,
            children: ChildrenPath::SpecificChildren(vec![NodeMask {
                index: 1,
                children: ChildrenPath::SpecificChildren(vec![NodeMask {
                    index: 1,
                    children: ChildrenPath::SpecificChildren(vec![NodeMask::new(0)]),
                }]),
            }]),
        };
        let result = tree.nodes_from_mask(multi_path).expect("Should not fail");
        assert_eq!(
            result.len(),
            4,
            "Should return 4 nodes for multi-level path: root, top_dir, file1.txt, and file1.txt's contents"
        );
        assert!(
            result.first().unwrap().is_leaf(),
            "First node should be file1.txt's contents"
        );

        // Test: invalid index should return error
        let invalid_path = NodeMask {
            index: 0,
            children: ChildrenPath::SpecificChildren(vec![NodeMask::new(999)]),
        };
        assert!(
            tree.nodes_from_mask(invalid_path).is_err(),
            "Should return error for invalid index"
        );

        // Test: verify nodes are returned in reverse BFS order (children first)
        let path = NodeMask {
            index: 0,
            children: ChildrenPath::SpecificChildren(vec![
                NodeMask {
                    // root.txt
                    index: 0,
                    children: ChildrenPath::SpecificChildren(vec![NodeMask::new(0)]),
                },
                NodeMask {
                    // top_dir
                    index: 1,
                    children: ChildrenPath::SpecificChildren(vec![
                        NodeMask {
                            // subdir_b
                            index: 0,
                            children: ChildrenPath::SpecificChildren(vec![NodeMask {
                                // file4.txt
                                index: 0,
                                children: ChildrenPath::SpecificChildren(vec![NodeMask::new(0)]),
                            }]),
                        },
                        NodeMask::new(1), // file1.txt
                        NodeMask {
                            // subdir_a
                            index: 2,
                            children: ChildrenPath::SpecificChildren(vec![
                                NodeMask {
                                    // file2.txt
                                    index: 0,
                                    children: ChildrenPath::SpecificChildren(vec![NodeMask::new(
                                        0,
                                    )]),
                                },
                                NodeMask {
                                    // file3.txt
                                    index: 1,
                                    children: ChildrenPath::SpecificChildren(vec![NodeMask::new(
                                        0,
                                    )]),
                                },
                            ]),
                        },
                    ]),
                },
            ]),
        };

        let result = tree.nodes_from_mask(path).expect("Should not fail");

        // Children should come first
        assert!(result[0].is_leaf(), "First node should be a leaf");
        assert!(result[1].is_leaf(), "Second node should be a leaf");
        assert!(result[2].is_leaf(), "Third node should be a leaf");

        // Last node should be root
        assert!(
            !result.last().unwrap().is_leaf(),
            "Last node should not be a leaf"
        );
    });
}
