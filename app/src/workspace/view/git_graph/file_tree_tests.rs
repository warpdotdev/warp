//! Tests for the flat-files → collapsible-tree transform. Pure logic: no
//! rendering, no async, no git.

use std::collections::HashSet;

use super::super::data::ChangedFile;
use super::{all_dir_paths, build_file_rows, FileRow};

/// Terse constructor for a changed file with explicit counts.
fn file(path: &str, additions: u32, deletions: u32) -> ChangedFile {
    ChangedFile {
        path: path.to_string(),
        additions,
        deletions,
    }
}

fn no_collapse() -> HashSet<String> {
    HashSet::new()
}

#[test]
fn directories_come_before_files_alphabetically() {
    // Root level mixes a directory and a file; within the directory two files.
    let files = vec![
        file("README.md", 1, 0),
        file("src/b.rs", 1, 0),
        file("src/a.rs", 1, 0),
    ];
    let rows = build_file_rows(&files, &no_collapse());
    assert_eq!(
        rows,
        vec![
            // `src` directory (and its children) before the root-level file.
            FileRow::Dir {
                path: "src".to_string(),
                name: "src".to_string(),
                depth: 0,
            },
            FileRow::File {
                index: 2,
                name: "a.rs".to_string(),
                depth: 1,
            },
            FileRow::File {
                index: 1,
                name: "b.rs".to_string(),
                depth: 1,
            },
            FileRow::File {
                index: 0,
                name: "README.md".to_string(),
                depth: 0,
            },
        ]
    );
}

#[test]
fn nested_directories_increase_depth() {
    let files = vec![file("a/b/c.rs", 0, 0)];
    let rows = build_file_rows(&files, &no_collapse());
    assert_eq!(
        rows,
        vec![
            FileRow::Dir {
                path: "a".to_string(),
                name: "a".to_string(),
                depth: 0,
            },
            FileRow::Dir {
                path: "a/b".to_string(),
                name: "b".to_string(),
                depth: 1,
            },
            FileRow::File {
                index: 0,
                name: "c.rs".to_string(),
                depth: 2,
            },
        ]
    );
}

#[test]
fn collapsed_directory_hides_its_subtree_but_stays_visible() {
    let files = vec![
        file("src/a.rs", 0, 0),
        file("src/sub/x.rs", 0, 0),
        file("top.rs", 0, 0),
    ];
    let mut collapsed = HashSet::new();
    collapsed.insert("src".to_string());
    let rows = build_file_rows(&files, &collapsed);
    // `src` itself shows, but neither its files nor its `sub` child are emitted.
    assert_eq!(
        rows,
        vec![
            FileRow::Dir {
                path: "src".to_string(),
                name: "src".to_string(),
                depth: 0,
            },
            FileRow::File {
                index: 2,
                name: "top.rs".to_string(),
                depth: 0,
            },
        ]
    );
}

#[test]
fn file_index_is_preserved_across_alphabetical_reordering() {
    // Original order is reverse-alphabetical; the rows reorder by name but each
    // file keeps its original index so OpenFileDiff(index) still resolves.
    let files = vec![file("z.rs", 0, 0), file("a.rs", 0, 0)];
    let rows = build_file_rows(&files, &no_collapse());
    assert_eq!(
        rows,
        vec![
            FileRow::File {
                index: 1,
                name: "a.rs".to_string(),
                depth: 0,
            },
            FileRow::File {
                index: 0,
                name: "z.rs".to_string(),
                depth: 0,
            },
        ]
    );
}

#[test]
fn all_dir_paths_lists_every_directory_prefix_once() {
    let files = vec![
        file("src/a.rs", 0, 0),
        file("src/sub/x.rs", 0, 0),
        file("top.rs", 0, 0),
    ];
    let mut paths = all_dir_paths(&files);
    paths.sort();
    assert_eq!(paths, vec!["src".to_string(), "src/sub".to_string()]);
}
