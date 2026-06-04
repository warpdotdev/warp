use std::fs;

use ignore::gitignore::Gitignore;

use super::{matches_gitignores, Entry, IgnoredPathStrategy};
#[cfg(unix)]
use crate::StandingQueryContent;
use crate::{StandingQueryDefinitions, StandingQueryResults};
#[test]
fn test_git_path_filtering_allowlist() {
    use std::path::Path;

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

/// Writes a `.gitignore` with `content` at `root` and returns a [`Gitignore`]
/// rooted there. Uses only the repo-root gitignore (not the machine's global
/// gitignore) so tests are deterministic.
fn gitignore_rooted(root: &std::path::Path, content: &str) -> Gitignore {
    fs::write(root.join(".gitignore"), content).unwrap();
    let (gitignore, _) = Gitignore::new(root.join(".gitignore"));
    gitignore
}

#[test]
fn should_watch_prunes_gitignored_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir(root.join("node_modules")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let gitignores = vec![gitignore_rooted(&root, "node_modules/\n")];

    // Root and non-ignored dirs are watched; the gitignored dir is pruned.
    assert!(super::should_watch_repo_directory(&root, &gitignores, &[]));
    assert!(super::should_watch_repo_directory(
        &root.join("src"),
        &gitignores,
        &[]
    ));
    assert!(!super::should_watch_repo_directory(
        &root.join("node_modules"),
        &gitignores,
        &[]
    ));
    // Descendants of an ignored dir are also pruned (ancestor-aware), which is
    // what preserves the watcher's monotonicity invariant.
    assert!(!super::should_watch_repo_directory(
        &root.join("node_modules/foo"),
        &gitignores,
        &[]
    ));
}

#[test]
fn should_watch_descends_to_force_included_under_ignored_ancestor() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir_all(root.join(".agents/skills/test")).unwrap();
    fs::create_dir(root.join(".agents/other")).unwrap();
    let gitignores = vec![gitignore_rooted(&root, ".agents/\n")];
    let force_included = vec![std::path::PathBuf::from(".agents/skills")];

    // The whole `.agents` subtree is gitignored, but we still descend along the
    // prefix to reach the force-included path, and into its subtree.
    assert!(super::should_watch_repo_directory(
        &root.join(".agents"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join(".agents/skills"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join(".agents/skills/test"),
        &gitignores,
        &force_included
    ));
    // A sibling ignored dir that is not force-included is still pruned.
    assert!(!super::should_watch_repo_directory(
        &root.join(".agents/other"),
        &gitignores,
        &force_included
    ));
}

#[test]
fn should_watch_handles_nested_ignored_ancestor_with_deeper_force_included() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir_all(root.join("a/b/c")).unwrap();
    fs::create_dir(root.join("a/b/other")).unwrap();
    let gitignores = vec![gitignore_rooted(&root, "a/b/\n")];
    let force_included = vec![std::path::PathBuf::from("a/b/c")];

    // `a/b` is ignored but `a/b/c` is force-included: descend along the whole
    // prefix and into it, while pruning the ignored sibling.
    assert!(super::should_watch_repo_directory(
        &root.join("a"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join("a/b"),
        &gitignores,
        &force_included
    ));
    assert!(super::should_watch_repo_directory(
        &root.join("a/b/c"),
        &gitignores,
        &force_included
    ));
    assert!(!super::should_watch_repo_directory(
        &root.join("a/b/other"),
        &gitignores,
        &force_included
    ));
}

#[test]
fn should_watch_descends_dir_only_reinclude_negation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::create_dir_all(root.join("parentdir/sub")).unwrap();
    fs::write(root.join("parentdir/loose.txt"), "").unwrap();
    // Ignore the loose files in `parentdir` but re-include its subdirectories.
    let gitignores = vec![gitignore_rooted(&root, "parentdir/*\n!parentdir/*/\n")];

    // `parentdir` itself is not matched by `parentdir/*`, so we descend.
    assert!(super::should_watch_repo_directory(
        &root.join("parentdir"),
        &gitignores,
        &[]
    ));
    // The subdirectory is re-included by the directory-only negation, so it is
    // still watched even though `parentdir/*` matched it first.
    assert!(super::should_watch_repo_directory(
        &root.join("parentdir/sub"),
        &gitignores,
        &[]
    ));
    // The loose file remains gitignored (the negation is directory-only); the
    // emit predicate filters it, but `parentdir` stays watched for its subdirs.
    assert!(matches_gitignores(
        &root.join("parentdir/loose.txt"),
        false,
        &gitignores,
        true,
    ));
}

#[test]
fn should_watch_preserves_git_internal_allowlist() {
    // No gitignores / force-included paths needed: `.git` handling
    // short-circuits and is path-based, mirroring
    // `should_watch_directory_in_git_path`.
    let repo = std::path::Path::new("/home/user/project");
    assert!(super::should_watch_repo_directory(
        &repo.join(".git/refs/heads"),
        &[],
        &[]
    ));
    assert!(!super::should_watch_repo_directory(
        &repo.join(".git/objects"),
        &[],
        &[]
    ));
}

fn find_entry<'a>(entry: &'a super::Entry, path: &std::path::Path) -> Option<&'a super::Entry> {
    let std_path = warp_util::standardized_path::StandardizedPath::try_from_local(path).ok()?;
    if entry.path() == &std_path {
        return Some(entry);
    }
    let super::Entry::Directory(directory) = entry else {
        return None;
    };
    directory
        .children
        .iter()
        .find_map(|child| find_entry(child, path))
}

fn build_skill_tree_with_gitignore(root: &std::path::Path, gitignore: &str) -> super::Entry {
    std::fs::write(root.join(".gitignore"), gitignore).unwrap();
    let mut files = Vec::new();
    let mut gitignores = Vec::new();
    let mut file_limit = 1000;
    super::Entry::build_tree_with_force_included_paths(
        root,
        &mut files,
        &mut gitignores,
        Some(&mut file_limit),
        super::BuildTreeOptions {
            max_depth: 200,
            current_depth: 0,
            ignored_path_strategy: &super::IgnoredPathStrategy::IncludeLazy,
            force_included_paths: &[std::path::PathBuf::from(".agents/skills")],
            budget_exceeded_behavior: super::BudgetExceededBehavior::StopAndLazyLoad,
        },
    )
    .unwrap()
}

#[test]
fn standing_queries_report_skills_below_an_ignored_directory() {
    virtual_fs::VirtualFS::test("standing_queries_report_ignored_skills", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/test")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/.agents/skills/test/SKILL.md",
                "name: test",
            )]);
        let repo = dirs.tests().join("repo");
        std::fs::write(repo.join(".gitignore"), ".agents/\n").unwrap();

        let mut files = Vec::new();
        let mut gitignores = Vec::new();
        let mut results = StandingQueryResults::default();
        let mut definitions = StandingQueryDefinitions::default();
        definitions.set_project_skill_provider_paths([std::path::PathBuf::from(".agents/skills")]);
        let tree = Entry::build_tree_with_standing_queries(
            &repo,
            &mut files,
            &mut gitignores,
            None,
            super::BuildTreeOptions {
                max_depth: 200,
                current_depth: 0,
                ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                force_included_paths: &[std::path::PathBuf::from(".agents/skills")],
                budget_exceeded_behavior: super::BudgetExceededBehavior::StopAndLazyLoad,
            },
            &mut results,
            &definitions,
        )
        .unwrap();

        let agents = find_entry(&tree, &repo.join(".agents")).expect(".agents should be present");
        assert!(agents.loaded());
        assert!(find_entry(&tree, &repo.join(".agents/skills/test/SKILL.md")).is_some());

        let skill_path = warp_util::standardized_path::StandardizedPath::try_from_local(
            &repo.join(".agents/skills/test/SKILL.md"),
        )
        .unwrap();
        assert!(results
            .project_skills()
            .any(|content| content.path == skill_path && !content.is_directory));
    });
}

#[cfg(unix)]
#[test]
fn standing_queries_report_symlinked_skills_without_materializing_symlinked_directories() {
    virtual_fs::VirtualFS::test(
        "standing_queries_report_symlinked_skills",
        |dirs, mut vfs| {
            vfs.mkdir("repo/.agents/skills")
                .mkdir("targets/linked")
                .with_files(vec![virtual_fs::Stub::FileWithContent(
                    "targets/linked/SKILL.md",
                    "name: linked",
                )]);
            let repo = dirs.tests().join("repo");
            let linked_directory = repo.join(".agents/skills/linked");
            std::os::unix::fs::symlink(dirs.tests().join("targets/linked"), &linked_directory)
                .unwrap();

            let mut files = Vec::new();
            let mut gitignores = Vec::new();
            let mut results = StandingQueryResults::default();
            let mut definitions = StandingQueryDefinitions::default();
            definitions
                .set_project_skill_provider_paths([std::path::PathBuf::from(".agents/skills")]);
            let tree = Entry::build_tree_with_standing_queries(
                &repo,
                &mut files,
                &mut gitignores,
                None,
                super::BuildTreeOptions {
                    max_depth: 200,
                    current_depth: 0,
                    ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                    force_included_paths: &[],
                    budget_exceeded_behavior: super::BudgetExceededBehavior::StopAndLazyLoad,
                },
                &mut results,
                &definitions,
            )
            .unwrap();

            assert!(find_entry(&tree, &linked_directory).is_none());
            assert!(results.project_skills().any(|content| {
                content
                    == &StandingQueryContent::file(
                        warp_util::standardized_path::StandardizedPath::try_from_local(
                            &linked_directory.join("SKILL.md"),
                        )
                        .unwrap(),
                    )
            }));
        },
    );
}
#[test]
fn standing_queries_do_not_report_rules_below_an_unloaded_shallow_directory() {
    virtual_fs::VirtualFS::test("standing_queries_report_shallow_rules", |dirs, mut vfs| {
        vfs.mkdir("repo/src/deep")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/src/deep/WARP.md",
                "project rules",
            )]);
        let repo = dirs.tests().join("repo");

        let mut files = Vec::new();
        let mut gitignores = Vec::new();
        let mut results = StandingQueryResults::default();
        let tree = Entry::build_tree_with_standing_queries(
            &repo,
            &mut files,
            &mut gitignores,
            None,
            super::BuildTreeOptions {
                max_depth: 1,
                current_depth: 0,
                ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                force_included_paths: &[],
                budget_exceeded_behavior: super::BudgetExceededBehavior::StopAndLazyLoad,
            },
            &mut results,
            &StandingQueryDefinitions::default(),
        )
        .unwrap();

        let src = find_entry(&tree, &repo.join("src")).expect("src should be represented");
        assert!(!src.loaded());
        assert!(find_entry(&tree, &repo.join("src/deep/WARP.md")).is_none());

        let rule_path = warp_util::standardized_path::StandardizedPath::try_from_local(
            &repo.join("src/deep/WARP.md"),
        )
        .unwrap();
        assert!(!results
            .project_rules()
            .any(|content| content.path == rule_path));
    });
}

#[test]
fn shallow_tree_expands_force_included_skill_branch_only() {
    virtual_fs::VirtualFS::test("shallow_tree_force_included_skills", |dirs, mut vfs| {
        vfs.mkdir("workspace/.agents/skills/review")
            .mkdir("workspace/src/deep")
            .with_files(vec![
                virtual_fs::Stub::FileWithContent(
                    "workspace/.agents/skills/review/SKILL.md",
                    "name: review",
                ),
                virtual_fs::Stub::FileWithContent("workspace/src/deep/WARP.md", "project rules"),
            ]);
        let workspace = dirs.tests().join("workspace");
        let skill_path = workspace.join(".agents/skills/review/SKILL.md");
        let rule_path = workspace.join("src/deep/WARP.md");

        let mut files = Vec::new();
        let mut gitignores = Vec::new();
        let mut results = StandingQueryResults::default();
        let mut definitions = StandingQueryDefinitions::default();
        definitions.set_project_skill_provider_paths([std::path::PathBuf::from(".agents/skills")]);
        let tree = Entry::build_tree_with_standing_queries(
            &workspace,
            &mut files,
            &mut gitignores,
            None,
            super::BuildTreeOptions {
                max_depth: 1,
                current_depth: 0,
                ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                force_included_paths: &[std::path::PathBuf::from(".agents/skills")],
                budget_exceeded_behavior: super::BudgetExceededBehavior::StopAndLazyLoad,
            },
            &mut results,
            &definitions,
        )
        .unwrap();

        let agents = find_entry(&tree, &workspace.join(".agents"))
            .expect("force-included ancestor should be represented");
        assert!(agents.loaded());
        assert!(find_entry(&tree, &skill_path).is_some());

        let src = find_entry(&tree, &workspace.join("src"))
            .expect("unrelated shallow directory should be represented");
        assert!(!src.loaded());
        assert!(find_entry(&tree, &rule_path).is_none());

        let skill_path =
            warp_util::standardized_path::StandardizedPath::try_from_local(&skill_path).unwrap();
        let rule_path =
            warp_util::standardized_path::StandardizedPath::try_from_local(&rule_path).unwrap();
        assert!(results
            .project_skills()
            .any(|content| content.path == skill_path && !content.is_directory));
        assert!(!results
            .project_rules()
            .any(|content| content.path == rule_path));
    });
}

#[test]
fn ignored_directory_stays_lazy() {
    virtual_fs::VirtualFS::test("ignored_dir_lazy", |dirs, mut vfs| {
        vfs.mkdir("repo/target/debug")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/target/debug/app",
                "binary",
            )]);
        let repo = dirs.tests().join("repo");
        std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
        let mut files = Vec::new();
        let mut gitignores = Vec::new();
        let tree = Entry::build_tree(
            &repo,
            &mut files,
            &mut gitignores,
            None,
            200,
            0,
            &IgnoredPathStrategy::IncludeLazy,
            super::BudgetExceededBehavior::StopAndLazyLoad,
        )
        .unwrap();
        let target_dir = find_entry(&tree, &repo.join("target"))
            .expect("ignored unrelated directory should be present as lazy");
        assert!(target_dir.ignored());
        assert!(!target_dir.loaded());
        assert!(find_entry(&tree, &repo.join("target/debug/app")).is_none());
    });
}

#[test]
fn ignored_skill_file_is_loaded_for_registered_provider_path() {
    virtual_fs::VirtualFS::test("ignored_skill_file_loaded", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/test")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/.agents/skills/test/SKILL.md",
                "name: test",
            )]);
        let repo = dirs.tests().join("repo");

        let tree = build_skill_tree_with_gitignore(&repo, ".agents/skills/test/SKILL.md\n");
        let skill_file = find_entry(&tree, &repo.join(".agents/skills/test/SKILL.md"))
            .expect("ignored skill file should be present");
        assert!(skill_file.ignored());
    });
}

#[test]
fn ignored_skill_directory_is_loaded_for_registered_provider_path() {
    virtual_fs::VirtualFS::test("ignored_skill_dir_loaded", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/test")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/.agents/skills/test/SKILL.md",
                "name: test",
            )]);
        let repo = dirs.tests().join("repo");

        let tree = build_skill_tree_with_gitignore(&repo, ".agents/skills/test/\n");
        let skill_dir = find_entry(&tree, &repo.join(".agents/skills/test"))
            .expect("ignored skill directory should be present");
        assert!(skill_dir.ignored());
        assert!(skill_dir.loaded());
        assert!(find_entry(&tree, &repo.join(".agents/skills/test/SKILL.md")).is_some());
    });
}

#[test]
fn ignored_agents_directory_is_loaded_for_registered_provider_path() {
    virtual_fs::VirtualFS::test("ignored_agents_dir_loaded", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/test")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/.agents/skills/test/SKILL.md",
                "name: test",
            )]);
        let repo = dirs.tests().join("repo");

        let tree = build_skill_tree_with_gitignore(&repo, ".agents/\n");
        let agents_dir = find_entry(&tree, &repo.join(".agents"))
            .expect("ignored .agents directory should be present");
        assert!(agents_dir.ignored());
        assert!(agents_dir.loaded());
        assert!(find_entry(&tree, &repo.join(".agents/skills/test/SKILL.md")).is_some());
    });
}

#[test]
fn ignored_agents_skills_directory_is_loaded_for_registered_provider_path() {
    virtual_fs::VirtualFS::test("ignored_agents_skills_dir_loaded", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/test")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/.agents/skills/test/SKILL.md",
                "name: test",
            )]);
        let repo = dirs.tests().join("repo");

        let tree = build_skill_tree_with_gitignore(&repo, ".agents/skills/\n");
        let skills_dir = find_entry(&tree, &repo.join(".agents/skills"))
            .expect("ignored .agents/skills directory should be present");
        assert!(skills_dir.ignored());
        assert!(skills_dir.loaded());
        assert!(find_entry(&tree, &repo.join(".agents/skills/test/SKILL.md")).is_some());
    });
}

#[test]
fn unrelated_ignored_directory_stays_lazy_without_registered_force_included() {
    virtual_fs::VirtualFS::test("unrelated_ignored_dir_lazy", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/test")
            .mkdir("repo/target/debug")
            .with_files(vec![
                virtual_fs::Stub::FileWithContent(
                    "repo/.agents/skills/test/SKILL.md",
                    "name: test",
                ),
                virtual_fs::Stub::FileWithContent("repo/target/debug/app", "binary"),
            ]);
        let repo = dirs.tests().join("repo");

        let tree = build_skill_tree_with_gitignore(&repo, "target/\n");
        let target_dir = find_entry(&tree, &repo.join("target"))
            .expect("ignored unrelated directory should be present as lazy");
        assert!(target_dir.ignored());
        assert!(!target_dir.loaded());
        assert!(find_entry(&tree, &repo.join("target/debug/app")).is_none());
    });
}

#[test]
fn build_tree_marks_descendants_of_ignored_directory_as_ignored() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root_path = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::write(root_path.join(".gitignore"), "ignored-dir/\n").unwrap();
    fs::create_dir(root_path.join("ignored-dir")).unwrap();
    fs::write(root_path.join("ignored-dir").join("ignored-file.txt"), "").unwrap();

    let mut files = Vec::new();
    let mut gitignores = Vec::<Gitignore>::new();
    let tree = Entry::build_tree(
        &root_path,
        &mut files,
        &mut gitignores,
        None,
        10,
        0,
        &IgnoredPathStrategy::Include,
        super::BudgetExceededBehavior::StopAndLazyLoad,
    )
    .unwrap();

    let Entry::Directory(root) = tree else {
        panic!("root should be a directory");
    };
    let ignored_dir = root
        .children
        .iter()
        .find(|entry| entry.path().file_name() == Some("ignored-dir"))
        .unwrap();
    let Entry::Directory(ignored_dir) = ignored_dir else {
        panic!("ignored child should be a directory");
    };
    assert!(ignored_dir.ignored);

    let ignored_file = ignored_dir
        .children
        .iter()
        .find(|entry| entry.path().file_name() == Some("ignored-file.txt"))
        .unwrap();
    assert!(ignored_file.ignored());
}

#[test]
fn lazy_loaded_ignored_directory_marks_loaded_children_as_ignored() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root_path = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::write(root_path.join(".gitignore"), "ignored-dir/\n").unwrap();
    fs::create_dir(root_path.join("ignored-dir")).unwrap();
    fs::write(root_path.join("ignored-dir").join("ignored-file.txt"), "").unwrap();

    let mut files = Vec::new();
    let mut gitignores = Vec::<Gitignore>::new();
    let mut tree = Entry::build_tree(
        &root_path,
        &mut files,
        &mut gitignores,
        None,
        10,
        0,
        &IgnoredPathStrategy::IncludeLazy,
        super::BudgetExceededBehavior::StopAndLazyLoad,
    )
    .unwrap();

    let ignored_path = root_path.join("ignored-dir");
    let ignored_dir = tree.find_mut(&ignored_path).unwrap();
    let Entry::Directory(directory) = ignored_dir else {
        panic!("ignored child should be a directory");
    };
    assert!(directory.ignored);
    assert!(!directory.loaded);
    assert!(directory.children.is_empty());

    ignored_dir.load(&mut gitignores).unwrap();

    let Entry::Directory(directory) = ignored_dir else {
        panic!("ignored child should still be a directory");
    };
    assert!(directory.ignored);
    assert!(directory.loaded);

    let ignored_file = directory
        .children
        .iter()
        .find(|entry| entry.path().file_name() == Some("ignored-file.txt"))
        .unwrap();
    assert!(ignored_file.ignored());
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

/// Builds a tree with an explicit file budget and force-included paths using
/// the lazy ignored-path strategy.
fn build_with_budget(
    root: &std::path::Path,
    budget: usize,
    force_included_paths: &[std::path::PathBuf],
) -> super::Entry {
    let mut files = Vec::new();
    let mut gitignores = Vec::new();
    let mut file_limit = budget;
    super::Entry::build_tree_with_force_included_paths(
        root,
        &mut files,
        &mut gitignores,
        Some(&mut file_limit),
        super::BuildTreeOptions {
            max_depth: 200,
            current_depth: 0,
            ignored_path_strategy: &super::IgnoredPathStrategy::IncludeLazy,
            force_included_paths,
            budget_exceeded_behavior: super::BudgetExceededBehavior::StopAndLazyLoad,
        },
    )
    .unwrap()
}

#[test]
fn build_tree_budget_covers_breadth_first_and_leaves_remainder_unloaded() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();

    // 5 top-level dirs, each with 2 direct files and a `sub` dir of 3 files.
    for i in 0..5 {
        let d = root.join(format!("d{i}"));
        fs::create_dir(&d).unwrap();
        fs::write(d.join("f0.txt"), "").unwrap();
        fs::write(d.join("f1.txt"), "").unwrap();
        let sub = d.join("sub");
        fs::create_dir(&sub).unwrap();
        for j in 0..3 {
            fs::write(sub.join(format!("g{j}.txt")), "").unwrap();
        }
    }

    // Budget exactly covers the 10 level-1 files; level-2 `sub` dirs are cut.
    let tree = build_with_budget(&root, 10, &[]);

    // Root stays loaded (no whole-tree depth-1 collapse on budget exhaustion).
    let Entry::Directory(root_dir) = &tree else {
        panic!("root should be a directory");
    };
    assert!(root_dir.loaded);

    for i in 0..5 {
        let d_path = root.join(format!("d{i}"));
        let d = find_entry(&tree, &d_path).expect("level-1 dir present");
        // All level-1 dirs are expanded before the budget is spent on level 2.
        assert!(d.loaded(), "all level-1 dirs are covered breadth-first");
        assert!(find_entry(&tree, &d_path.join("f0.txt")).is_some());

        // Level-2 dirs beyond the budget remain unloaded placeholders.
        let sub = find_entry(&tree, &d_path.join("sub")).expect("sub placeholder present");
        assert!(
            !sub.loaded(),
            "level-2 dirs beyond the budget stay unloaded"
        );
        assert!(find_entry(&tree, &d_path.join("sub").join("g0.txt")).is_none());
    }
}

#[test]
fn build_tree_budget_does_not_prune_force_included_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();

    // Files at the root so the budget is exhausted almost immediately.
    for i in 0..5 {
        fs::write(root.join(format!("f{i}.txt")), "").unwrap();
    }
    // A skill provider directory nested several levels under the root.
    let skill_dir = root.join(".agents").join("skills").join("test");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "name: test").unwrap();

    // Tiny budget: the root expands and is immediately exhausted, but the
    // force-included path must still be loaded all the way down.
    let force_included = [std::path::PathBuf::from(".agents/skills")];
    let tree = build_with_budget(&root, 1, &force_included);

    assert!(
        find_entry(&tree, &skill_dir.join("SKILL.md")).is_some(),
        "force-included path files must load even when the budget is exhausted"
    );
}

#[test]
fn build_tree_full_coverage_reaches_full_depth_within_budget() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();

    // Nested chain a/b/c/d with a file at the deepest level.
    let deep = root.join("a").join("b").join("c").join("d");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("leaf.txt"), "").unwrap();
    fs::write(root.join("top.txt"), "").unwrap();

    // A generous budget must fully cover this small tree to its full depth.
    let tree = build_with_budget(&root, 1000, &[]);

    for dir in [
        root.join("a"),
        root.join("a").join("b"),
        root.join("a").join("b").join("c"),
        deep.clone(),
    ] {
        let entry = find_entry(&tree, &dir).expect("dir present");
        assert!(
            entry.loaded(),
            "dirs are fully loaded under a generous budget"
        );
    }
    assert!(find_entry(&tree, &deep.join("leaf.txt")).is_some());
    assert!(find_entry(&tree, &root.join("top.txt")).is_some());
}

#[test]
fn build_tree_directories_do_not_consume_budget() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();

    // A deep chain of empty directories (no files at all).
    let deep = root.join("l1").join("l2").join("l3").join("l4");
    fs::create_dir_all(&deep).unwrap();

    // Even a budget of 1 fully expands the tree, because only files — not
    // directories — draw down the budget.
    let tree = build_with_budget(&root, 1, &[]);
    let leaf = find_entry(&tree, &deep).expect("deepest dir present");
    assert!(
        leaf.loaded(),
        "directories must not consume the file budget"
    );
}

#[test]
fn build_tree_gitignored_files_do_not_consume_budget() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();

    // A gitignored directory with many files (e.g. node_modules/target).
    let ignored = root.join("ignored");
    fs::create_dir(&ignored).unwrap();
    for i in 0..50 {
        fs::write(ignored.join(format!("big{i}.txt")), "").unwrap();
    }
    // Plus the tracked files at the root (`.gitignore` itself counts as one).
    fs::write(root.join("tracked0.txt"), "").unwrap();
    fs::write(root.join("tracked1.txt"), "").unwrap();

    // Budget only covers the 3 tracked root files. The 50 gitignored files must
    // not draw it down, so both tracked files are still indexed and the ignored
    // directory stays a lazy (unloaded) placeholder.
    let tree = build_with_budget(&root, 3, &[]);

    assert!(find_entry(&tree, &root.join("tracked0.txt")).is_some());
    assert!(find_entry(&tree, &root.join("tracked1.txt")).is_some());
    let ignored_dir = find_entry(&tree, &ignored).expect("ignored dir placeholder present");
    assert!(ignored_dir.ignored());
    assert!(
        !ignored_dir.loaded(),
        "gitignored dirs stay lazy and never consume the budget"
    );
}

#[test]
fn build_tree_fail_fast_errors_when_budget_exceeded() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    for i in 0..10 {
        fs::write(root.join(format!("f{i}.txt")), "").unwrap();
    }

    let mut files = Vec::new();
    let mut gitignores = Vec::new();
    let mut file_limit = 5;
    let result = Entry::build_tree(
        &root,
        &mut files,
        &mut gitignores,
        Some(&mut file_limit),
        200,
        0,
        &IgnoredPathStrategy::Exclude,
        super::BudgetExceededBehavior::FailFast,
    );
    assert!(
        matches!(result, Err(super::BuildTreeError::ExceededMaxFileLimit)),
        "FailFast must abort when the file budget is exceeded"
    );
}

#[test]
fn build_tree_fail_fast_succeeds_within_budget() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(temp_dir.path()).unwrap();
    for i in 0..3 {
        fs::write(root.join(format!("f{i}.txt")), "").unwrap();
    }

    let mut files = Vec::new();
    let mut gitignores = Vec::new();
    let mut file_limit = 10;
    let result = Entry::build_tree(
        &root,
        &mut files,
        &mut gitignores,
        Some(&mut file_limit),
        200,
        0,
        &IgnoredPathStrategy::Exclude,
        super::BudgetExceededBehavior::FailFast,
    );
    assert!(result.is_ok(), "FailFast must succeed when within budget");
}
