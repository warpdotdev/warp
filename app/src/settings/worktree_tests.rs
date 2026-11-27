use std::path::{Path, PathBuf};

use super::{WorktreeLocation, worktree_repo_dir};

const REPO: &str = "/Users/me/code/backend";

#[test]
fn warp_data_directory_nests_by_repo_name() {
    let dir = worktree_repo_dir(WorktreeLocation::WarpDataDirectory, "", Path::new(REPO));
    assert_eq!(
        dir,
        warp_core::paths::data_dir()
            .join("worktrees")
            .join("backend")
    );
}

#[test]
fn next_to_repository_uses_sibling_directory() {
    let dir = worktree_repo_dir(WorktreeLocation::NextToRepository, "", Path::new(REPO));
    assert_eq!(dir, PathBuf::from("/Users/me/code/backend-worktrees"));
}

#[test]
fn inside_repository_uses_dot_worktrees() {
    let dir = worktree_repo_dir(WorktreeLocation::InsideRepository, "", Path::new(REPO));
    assert_eq!(dir, PathBuf::from("/Users/me/code/backend/.worktrees"));
}

#[test]
fn custom_directory_nests_by_repo_name() {
    let dir = worktree_repo_dir(
        WorktreeLocation::CustomDirectory,
        "/tmp/trees",
        Path::new(REPO),
    );
    assert_eq!(dir, PathBuf::from("/tmp/trees/backend"));
}

#[test]
fn custom_directory_expands_home() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let dir = worktree_repo_dir(
        WorktreeLocation::CustomDirectory,
        "~/trees",
        Path::new(REPO),
    );
    assert_eq!(dir, home.join("trees").join("backend"));
}

#[test]
fn empty_custom_directory_falls_back_to_warp_data_directory() {
    let dir = worktree_repo_dir(WorktreeLocation::CustomDirectory, "  ", Path::new(REPO));
    assert_eq!(
        dir,
        worktree_repo_dir(WorktreeLocation::WarpDataDirectory, "", Path::new(REPO))
    );
}
