use std::fs;
use std::path::Path;

use super::{Entry, GitignoreRuleCache, IgnoredPathStrategy};

#[test]
fn gitignore_traversal_truncates_sibling_scope() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir(&left).unwrap();
    fs::create_dir(&right).unwrap();
    fs::write(left.join(".gitignore"), "shared.txt\n").unwrap();
    fs::write(right.join(".gitignore"), "other.txt\n").unwrap();

    let mut gitignore_rules = GitignoreRuleCache::empty_for_root(root);
    let mut gitignore_traversal = gitignore_rules.refreshed_traversal_for_path(root);
    let root_active_len = gitignore_traversal.enter_directory(root);

    let left_active_len = gitignore_traversal.enter_directory(&left);
    assert!(gitignore_traversal.matches(&left.join("shared.txt"), false, false));

    gitignore_traversal.truncate_active(left_active_len);
    assert!(!gitignore_traversal.matches(&left.join("shared.txt"), false, false));

    gitignore_traversal.enter_directory(&right);
    assert!(gitignore_traversal.matches(&right.join("other.txt"), false, false));
    assert!(!gitignore_traversal.matches(&left.join("shared.txt"), false, false));
    gitignore_traversal.truncate_active(root_active_len);
}

#[test]
fn gitignore_rules_repeated_lazy_loads_do_not_grow_matcher_count() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(child.join("ignored.txt"), "ignored").unwrap();

    let mut gitignore_rules = GitignoreRuleCache::empty_for_root(&root);
    for _ in 0..3 {
        let mut files = Vec::new();
        Entry::build_tree(
            child.clone(),
            &mut files,
            &mut gitignore_rules,
            None,
            usize::MAX,
            0,
            &IgnoredPathStrategy::Exclude,
        )
        .unwrap();
    }

    assert_eq!(gitignore_rules.matcher_count(), 1);
}

#[test]
fn gitignore_rules_refresh_changed_and_deleted_gitignore_files() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let ignored = root.join("ignored.txt");
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(&ignored, "ignored").unwrap();

    let mut gitignore_rules = GitignoreRuleCache::empty_for_root(&root);
    assert!(gitignore_rules.is_ignored_with_refresh(&ignored, false, false));

    fs::write(root.join(".gitignore"), "other.txt\n").unwrap();
    assert!(!gitignore_rules.is_ignored_with_refresh(&ignored, false, false));
    assert!(gitignore_rules.is_ignored_with_refresh(&root.join("other.txt"), false, false));

    fs::remove_file(root.join(".gitignore")).unwrap();
    assert!(!gitignore_rules.is_ignored_with_refresh(&root.join("other.txt"), false, false));
    assert_eq!(gitignore_rules.matcher_count(), 0);
}

#[test]
fn gitignore_traversal_for_path_only_activates_ancestor_matchers() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir(&left).unwrap();
    fs::create_dir(&right).unwrap();
    fs::write(left.join(".gitignore"), "shared.txt\n").unwrap();
    fs::write(right.join(".gitignore"), "other.txt\n").unwrap();

    let mut gitignore_rules = GitignoreRuleCache::empty_for_root(&root);
    assert!(gitignore_rules.is_ignored_with_refresh(&left.join("shared.txt"), false, false));
    let gitignore_traversal = gitignore_rules.refreshed_traversal_for_path(&right);

    assert!(gitignore_traversal.matches(&right.join("other.txt"), false, false));
    assert!(!gitignore_traversal.matches(&left.join("shared.txt"), false, false));
}

#[test]
fn build_tree_with_seeded_gitignores_does_not_apply_sibling_rules() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir(&left).unwrap();
    fs::create_dir(&right).unwrap();
    fs::write(left.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(left.join("ignored.txt"), "left ignored").unwrap();
    fs::write(right.join("ignored.txt"), "right visible").unwrap();

    let mut gitignore_rules = GitignoreRuleCache::empty_for_root(&root);
    assert!(gitignore_rules.is_ignored_with_refresh(&left.join("ignored.txt"), false, false));
    let mut files = Vec::new();
    Entry::build_tree(
        right.clone(),
        &mut files,
        &mut gitignore_rules,
        None,
        usize::MAX,
        0,
        &IgnoredPathStrategy::Exclude,
    )
    .unwrap();

    let file_paths: Vec<_> = files
        .iter()
        .map(|metadata| metadata.path.to_local_path_lossy())
        .collect();
    assert!(file_paths.contains(&right.join("ignored.txt")));
}

#[test]
fn build_tree_does_not_apply_sibling_gitignore_rules() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tempdir.path()).unwrap();
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir(&left).unwrap();
    fs::create_dir(&right).unwrap();
    fs::write(left.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(left.join("ignored.txt"), "left ignored").unwrap();
    fs::write(right.join("ignored.txt"), "right visible").unwrap();

    let mut files = Vec::new();
    let mut gitignore_rules = GitignoreRuleCache::empty_for_root(&root);
    let entry = Entry::build_tree(
        root,
        &mut files,
        &mut gitignore_rules,
        None,
        usize::MAX,
        0,
        &IgnoredPathStrategy::Exclude,
    )
    .unwrap();

    assert!(matches!(entry, Entry::Directory(_)));
    let file_paths: Vec<_> = files
        .iter()
        .map(|metadata| metadata.path.to_local_path_lossy())
        .collect();
    assert!(file_paths.contains(&right.join("ignored.txt")));
    assert!(!file_paths.contains(&left.join("ignored.txt")));
    assert_eq!(gitignore_rules.matcher_count(), 1);
}

#[test]
fn test_git_path_filtering_allowlist() {
    use super::{
        is_commit_related_git_file, is_common_git_config, is_index_lock_file,
        is_remote_tracking_ref, is_tracking_state_git_file, should_ignore_git_path,
    };

    // Non-git paths should not be ignored
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/src/main.rs"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/README.md"
    )));

    // .git directory itself should be ignored
    assert!(should_ignore_git_path(Path::new("/home/user/project/.git")));

    // Allowlisted: commit-related files are NOT ignored
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/HEAD"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/heads/main"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/heads/feature-branch"
    )));

    // Allowlisted: index.lock is NOT ignored
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/index.lock"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/config"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/remotes/origin/main"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/remotes/origin/feature/nested"
    )));

    // Everything else in .git/ IS ignored
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/index"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/COMMIT_EDITMSG"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/FETCH_HEAD"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/ORIG_HEAD"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/tags/v1.0"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/refs/remotes/origin"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/objects/abc123"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/hooks/pre-commit"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/logs/HEAD"
    )));

    // Worktree paths: allowlisted patterns under .git/worktrees/<name>/
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/HEAD"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/index.lock"
    )));
    assert!(!should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/config.worktree"
    )));
    // Non-allowlisted worktree paths are still ignored
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/index"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt/COMMIT_EDITMSG"
    )));
    // worktrees dir itself (no content after worktree name) is ignored
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees"
    )));
    assert!(should_ignore_git_path(Path::new(
        "/home/user/project/.git/worktrees/my-wt"
    )));

    // is_commit_related_git_file
    assert!(is_commit_related_git_file(Path::new("/repo/.git/HEAD")));
    assert!(is_commit_related_git_file(Path::new(
        "/repo/.git/refs/heads/main"
    )));
    assert!(is_commit_related_git_file(Path::new(
        "/repo/.git/worktrees/wt/HEAD"
    )));
    assert!(!is_commit_related_git_file(Path::new(
        "/repo/.git/index.lock"
    )));
    assert!(!is_commit_related_git_file(Path::new(
        "/repo/.git/refs/tags/v1"
    )));

    // is_index_lock_file
    assert!(is_index_lock_file(Path::new("/repo/.git/index.lock")));
    assert!(is_index_lock_file(Path::new(
        "/repo/.git/worktrees/wt/index.lock"
    )));
    assert!(!is_index_lock_file(Path::new("/repo/.git/HEAD")));
    assert!(!is_index_lock_file(Path::new("/repo/.git/index")));

    // Remote-tracking refs
    assert!(is_remote_tracking_ref(Path::new(
        "/repo/.git/refs/remotes/origin/main"
    )));
    assert!(is_remote_tracking_ref(Path::new(
        "/repo/.git/refs/remotes/origin/feature/nested"
    )));
    assert!(!is_remote_tracking_ref(Path::new(
        "/repo/.git/refs/remotes/origin"
    )));
    assert!(!is_remote_tracking_ref(Path::new(
        "/repo/.git/worktrees/wt/refs/remotes/origin/main"
    )));
    assert!(!is_remote_tracking_ref(Path::new(
        "/repo/.git/refs/heads/main"
    )));

    // Tracking-state files
    assert!(is_tracking_state_git_file(Path::new("/repo/.git/HEAD")));
    assert!(is_tracking_state_git_file(Path::new("/repo/.git/config")));
    assert!(is_tracking_state_git_file(Path::new(
        "/repo/.git/worktrees/wt/config.worktree"
    )));
    assert!(!is_tracking_state_git_file(Path::new(
        "/repo/.git/refs/remotes/origin/main"
    )));

    // Common config
    assert!(is_common_git_config(Path::new("/repo/.git/config")));
    assert!(!is_common_git_config(Path::new(
        "/repo/.git/worktrees/wt/config.worktree"
    )));

    // Test Windows-style paths (only on Windows, as path parsing is platform-specific)
    #[cfg(windows)]
    {
        assert!(!should_ignore_git_path(Path::new(
            r"C:\Users\user\project\.git\HEAD"
        )));
        assert!(!should_ignore_git_path(Path::new(
            r"C:\Users\user\project\.git\index.lock"
        )));
        assert!(should_ignore_git_path(Path::new(
            r"C:\Users\user\project\.git\index"
        )));
    }
}

#[test]
fn should_watch_directory_in_git_path_prunes_non_allowlisted_subtrees() {
    use std::path::Path;

    use super::should_watch_directory_in_git_path;
    for path in [
        "/repo/.git",
        "/repo/.git/refs",
        "/repo/.git/refs/heads",
        "/repo/.git/refs/remotes",
        "/repo/.git/refs/remotes/origin",
        "/repo/.git/worktrees",
        "/repo/.git/worktrees/my-wt",
        "/repo/.git/worktrees/my-wt/refs",
        "/repo/.git/worktrees/my-wt/refs/heads",
    ] {
        assert!(
            should_watch_directory_in_git_path(Path::new(path)),
            "{path} should remain traversable so allowlisted git children stay reachable"
        );
    }

    for path in [
        "/repo/.git/objects",
        "/repo/.git/hooks",
        "/repo/.git/logs",
        "/repo/.git/info",
        "/repo/.git/lfs",
        "/repo/.git/refs/tags",
        "/repo/.git/worktrees/my-wt/objects",
        "/repo/.git/worktrees/my-wt/logs",
    ] {
        assert!(
            !should_watch_directory_in_git_path(Path::new(path)),
            "{path} should be pruned from recursive watcher registration"
        );
    }
    assert!(!should_watch_directory_in_git_path(Path::new(
        "/repo/.git/objects/ab/blob"
    )));
    // The predicate is only consulted on directories during recursive registration;
    // file paths like `.git/HEAD` would never actually reach it, but the default
    // false return here documents that they're not treated as descend roots.
    assert!(!should_watch_directory_in_git_path(Path::new(
        "/repo/.git/HEAD"
    )));
    assert!(!should_watch_directory_in_git_path(Path::new(
        "/repo/.git/config"
    )));
}
#[test]
fn test_is_shared_git_ref() {
    use std::path::Path;

    use super::is_shared_git_ref;

    // Shared refs — broadcast to all repos
    assert!(is_shared_git_ref(Path::new("/repo/.git/refs/heads/main")));
    assert!(is_shared_git_ref(Path::new(
        "/repo/.git/refs/heads/feature"
    )));

    // Repo-specific — NOT shared
    assert!(!is_shared_git_ref(Path::new("/repo/.git/HEAD")));
    assert!(!is_shared_git_ref(Path::new("/repo/.git/index.lock")));

    // Worktree paths — NOT shared
    assert!(!is_shared_git_ref(Path::new(
        "/repo/.git/worktrees/foo/HEAD"
    )));
    assert!(!is_shared_git_ref(Path::new(
        "/repo/.git/worktrees/foo/refs/heads/main"
    )));

    // Other .git internals — NOT shared
    assert!(!is_shared_git_ref(Path::new("/repo/.git/refs/tags/v1")));
    assert!(!is_shared_git_ref(Path::new(
        "/repo/.git/refs/remotes/origin/main"
    )));
    assert!(!is_shared_git_ref(Path::new("/repo/.git/config")));

    // Not a git path at all
    assert!(!is_shared_git_ref(Path::new("/repo/src/main.rs")));
}

#[test]
fn test_extract_worktree_git_dir() {
    use std::path::{Path, PathBuf};

    use super::extract_worktree_git_dir;

    // Standard worktree path extracts the per-worktree gitdir
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees/foo/HEAD")),
        Some(PathBuf::from("/repo/.git/worktrees/foo"))
    );
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees/bar/index.lock")),
        Some(PathBuf::from("/repo/.git/worktrees/bar"))
    );

    // Non-worktree paths return None
    assert_eq!(extract_worktree_git_dir(Path::new("/repo/.git/HEAD")), None);
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/refs/heads/main")),
        None
    );
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/src/main.rs")),
        None
    );

    // Edge case: not enough depth after worktrees/
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees")),
        None
    );
    assert_eq!(
        extract_worktree_git_dir(Path::new("/repo/.git/worktrees/foo")),
        None
    );
}
