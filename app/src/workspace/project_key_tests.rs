use std::path::Path;

use warp_util::standardized_path::StandardizedPath;

use super::ProjectKey;

fn std_path(path: &str) -> StandardizedPath {
    StandardizedPath::try_from_local(Path::new(path)).unwrap()
}

#[test]
fn local_git_display_name_is_repo_folder() {
    // `common_git_dir` is `<repo>/.git`; the label should be the repo folder.
    let key = ProjectKey::LocalGit(std_path("/Users/sam/dev/warp/.git"));
    assert_eq!(key.display_name(), "warp");
}

#[test]
fn worktrees_of_same_repo_share_one_key_and_name() {
    // Two worktrees of one repo both resolve (via `common_git_dir`) to the same
    // shared `.git`, so they produce the same `ProjectKey` and label. The
    // worktree-checkout → common-git-dir resolution itself is covered by
    // `repo_metadata`'s own tests (`derive_common_git_dir`); here we assert that
    // keying on the shared `.git` unifies them.
    let main = ProjectKey::LocalGit(std_path("/Users/sam/dev/warp/.git"));
    let worktree = ProjectKey::LocalGit(std_path("/Users/sam/dev/warp/.git"));
    assert_eq!(main, worktree);
    assert_eq!(main.display_name(), "warp");
}

#[test]
fn distinct_repos_have_distinct_keys() {
    let warp = ProjectKey::LocalGit(std_path("/Users/sam/dev/warp/.git"));
    let orbit = ProjectKey::LocalGit(std_path("/Users/sam/dev/orbit/.git"));
    assert_ne!(warp, orbit);
    assert_eq!(orbit.display_name(), "orbit");
}

#[test]
fn local_dir_display_name_is_folder() {
    let key = ProjectKey::LocalDir(std_path("/Users/sam/dev/marketing_videos"));
    assert_eq!(key.display_name(), "marketing_videos");
}
