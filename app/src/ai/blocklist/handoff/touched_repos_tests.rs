//! Tests for `touched_repos.rs`.
//!
//! Only covers `find_git_root`, which actually walks the filesystem against a
//! temporary directory layout. The pure helpers (`parse_github_repo`,
//! `pick_handoff_overlap_env`) are exercised end-to-end by the handoff submit
//! path and don't get standalone tests — their correctness is enforced by
//! their call sites.

use super::*;
use std::fs;
use tempfile::tempdir;
use tokio::runtime::Runtime;

#[test]
fn find_git_root_walks_up_to_dot_git() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let nested = repo.join("src").join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();

    let file_in_repo = nested.join("foo.rs");
    fs::write(&file_in_repo, "").unwrap();

    let outside = tmp.path().join("not_a_repo").join("file.txt");
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&outside, "").unwrap();

    let rt = Runtime::new().unwrap();
    let (root_for_file, root_for_dir, root_for_outside) = rt.block_on(async {
        (
            find_git_root(&file_in_repo).await,
            find_git_root(&nested).await,
            find_git_root(&outside).await,
        )
    });

    assert_eq!(root_for_file.expect("root for file inside repo"), repo);
    assert_eq!(root_for_dir.expect("root for directory inside repo"), repo);
    assert!(root_for_outside.is_none());
}
