use std::collections::HashSet;

use super::{
    build_tree_from_counts, collect_dir_paths, sort_nodes, sync_expanded_dirs, CodeReviewTreeNode,
};

fn file(name: &str) -> CodeReviewTreeNode {
    CodeReviewTreeNode::File {
        name: name.to_string(),
        file_path: name.to_string(),
        additions: 0,
        deletions: 0,
        file_index: 0,
    }
}

fn dir(name: &str) -> CodeReviewTreeNode {
    CodeReviewTreeNode::Dir {
        name: name.to_string(),
        path: name.to_string(),
        children: Vec::new(),
    }
}

fn node_names(nodes: &[CodeReviewTreeNode]) -> Vec<&str> {
    nodes.iter().map(CodeReviewTreeNode::name).collect()
}

#[test]
fn sort_nodes_matches_project_explorer_ordering() {
    let mut nodes = vec![
        file("file10.rs"),
        dir("src2"),
        file(".env"),
        dir(".config"),
        file("file1.rs"),
        dir("src10"),
        file("file2.rs"),
        dir("src1"),
    ];

    sort_nodes(&mut nodes);

    assert_eq!(
        node_names(&nodes),
        [
            ".config",
            "src1",
            "src2",
            "src10",
            ".env",
            "file1.rs",
            "file2.rs",
            "file10.rs",
        ]
    );
}

fn find_file<'a>(nodes: &'a [CodeReviewTreeNode], name: &str) -> &'a CodeReviewTreeNode {
    fn walk<'a>(nodes: &'a [CodeReviewTreeNode], name: &str) -> Option<&'a CodeReviewTreeNode> {
        for node in nodes {
            match node {
                CodeReviewTreeNode::File {
                    name: node_name, ..
                } if node_name == name => {
                    return Some(node);
                }
                CodeReviewTreeNode::Dir { children, .. } => {
                    if let Some(found) = walk(children, name) {
                        return Some(found);
                    }
                }
                CodeReviewTreeNode::File { .. } => {}
            }
        }
        None
    }

    walk(nodes, name).unwrap_or_else(|| panic!("missing file {name}"))
}

#[test]
fn rebuilt_tree_uses_the_current_index_and_counts() {
    let first = build_tree_from_counts([("src/a.rs", 0, 1, 0), ("src/b.rs", 1, 2, 3)]);
    match find_file(&first, "b.rs") {
        CodeReviewTreeNode::File {
            file_index,
            additions,
            deletions,
            ..
        } => assert_eq!((*file_index, *additions, *deletions), (1, 2, 3)),
        CodeReviewTreeNode::Dir { .. } => panic!("expected a file"),
    }

    let second = build_tree_from_counts([("src/b.rs", 0, 4, 1)]);
    match find_file(&second, "b.rs") {
        CodeReviewTreeNode::File {
            file_index,
            additions,
            deletions,
            ..
        } => assert_eq!((*file_index, *additions, *deletions), (0, 4, 1)),
        CodeReviewTreeNode::Dir { .. } => panic!("expected a file"),
    }
}

#[test]
fn sync_expanded_dirs_keeps_collapse_and_expands_new_dirs() {
    let previous = build_tree_from_counts([
        ("src/a.rs", 0, 1, 0),
        ("keep/b.rs", 1, 0, 1),
        ("open/d.rs", 2, 0, 0),
    ]);
    let mut previous_dirs = HashSet::new();
    collect_dir_paths(&previous, &mut previous_dirs);

    let mut expanded = HashSet::from(["src".to_string(), "open".to_string()]);
    let next = build_tree_from_counts([
        ("keep/b.rs", 0, 2, 0),
        ("new/c.rs", 1, 1, 0),
        ("open/d.rs", 2, 0, 0),
    ]);
    sync_expanded_dirs(&previous_dirs, &next, &mut expanded);

    assert!(expanded.contains("open"));
    assert!(expanded.contains("new"));
    assert!(!expanded.contains("keep"));
    assert!(!expanded.contains("src"));
}
