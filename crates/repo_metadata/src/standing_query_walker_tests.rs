use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{evaluate_standing_queries, StandingQueryWalkOptions};
use crate::entry::{BudgetExceededBehavior, BuildTreeOptions, Entry, IgnoredPathStrategy};
use crate::standing_queries::{StandingQueryContent, StandingQueryDefinitions};
use crate::StandingQueryResults;

fn definitions() -> StandingQueryDefinitions {
    let mut definitions = StandingQueryDefinitions::default();
    definitions.set_project_skill_provider_paths([PathBuf::from(".agents/skills")]);
    definitions
}

fn walk_options() -> StandingQueryWalkOptions {
    StandingQueryWalkOptions {
        force_included_paths: vec![PathBuf::from(".agents/skills")],
        ..Default::default()
    }
}

fn standardized(path: &Path) -> warp_util::standardized_path::StandardizedPath {
    warp_util::standardized_path::StandardizedPath::try_from_local(path).unwrap()
}

fn contains_rule(results: &StandingQueryResults, path: &Path) -> bool {
    results
        .project_rules()
        .any(|content| content == &StandingQueryContent::file(standardized(path)))
}

fn contains_skill_file(results: &StandingQueryResults, path: &Path) -> bool {
    results
        .project_skills()
        .any(|content| content == &StandingQueryContent::file(standardized(path)))
}

#[test]
fn walker_discovers_rules_and_skills_at_any_depth() {
    virtual_fs::VirtualFS::test("walker_discovers_rules_and_skills", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/review")
            .mkdir("repo/packages/api/nested")
            .with_files(vec![
                virtual_fs::Stub::FileWithContent(
                    "repo/.agents/skills/review/SKILL.md",
                    "name: review",
                ),
                virtual_fs::Stub::FileWithContent("repo/WARP.md", "root rules"),
                virtual_fs::Stub::FileWithContent(
                    "repo/packages/api/nested/AGENTS.md",
                    "nested rules",
                ),
            ]);
        let repo = dirs.tests().join("repo");

        let results = evaluate_standing_queries(&repo, &definitions(), &walk_options());

        assert!(contains_rule(&results, &repo.join("WARP.md")));
        assert!(contains_rule(
            &results,
            &repo.join("packages/api/nested/AGENTS.md")
        ));
        assert!(contains_skill_file(
            &results,
            &repo.join(".agents/skills/review/SKILL.md")
        ));
        // The provider directory itself is retained as a directory match.
        assert!(results.project_skills().any(|content| {
            content == &StandingQueryContent::directory(standardized(&repo.join(".agents/skills")))
        }));
    });
}

#[test]
fn walker_prunes_gitignored_directories_but_keeps_force_included_providers() {
    virtual_fs::VirtualFS::test("walker_prunes_gitignored", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/review")
            .mkdir("repo/ignored/nested")
            .with_files(vec![
                virtual_fs::Stub::FileWithContent(
                    "repo/.agents/skills/review/SKILL.md",
                    "name: review",
                ),
                virtual_fs::Stub::FileWithContent("repo/ignored/nested/WARP.md", "hidden rules"),
            ]);
        let repo = dirs.tests().join("repo");
        std::fs::write(repo.join(".gitignore"), "ignored/\n.agents/\n").unwrap();

        let results = evaluate_standing_queries(&repo, &definitions(), &walk_options());

        // Rules below a gitignored directory are not discovered, matching the
        // eager tree walk.
        assert!(!contains_rule(
            &results,
            &repo.join("ignored/nested/WARP.md")
        ));
        // Force-included provider directories are still discovered even when
        // gitignored.
        assert!(contains_skill_file(
            &results,
            &repo.join(".agents/skills/review/SKILL.md")
        ));
    });
}

#[test]
fn walker_respects_nested_gitignores() {
    virtual_fs::VirtualFS::test("walker_respects_nested_gitignores", |dirs, mut vfs| {
        vfs.mkdir("repo/src/generated/deep")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/src/generated/deep/AGENTS.md",
                "generated rules",
            )]);
        let repo = dirs.tests().join("repo");
        std::fs::write(repo.join("src/.gitignore"), "generated/\n").unwrap();

        let results = evaluate_standing_queries(&repo, &definitions(), &walk_options());

        assert!(!contains_rule(
            &results,
            &repo.join("src/generated/deep/AGENTS.md")
        ));
    });
}

#[test]
fn walker_does_not_descend_into_git_internals() {
    virtual_fs::VirtualFS::test("walker_skips_git_internals", |dirs, mut vfs| {
        vfs.mkdir("repo/.git/info")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "repo/.git/info/WARP.md",
                "not a rule",
            )]);
        let repo = dirs.tests().join("repo");

        let results = evaluate_standing_queries(&repo, &definitions(), &walk_options());

        assert!(!contains_rule(&results, &repo.join(".git/info/WARP.md")));
    });
}

#[test]
fn walker_depth_limit_matches_lazy_path_coverage() {
    virtual_fs::VirtualFS::test("walker_depth_limit", |dirs, mut vfs| {
        vfs.mkdir("dir/src/deep")
            .mkdir("dir/.agents/skills/review")
            .with_files(vec![
                virtual_fs::Stub::FileWithContent("dir/WARP.md", "root rules"),
                virtual_fs::Stub::FileWithContent("dir/src/deep/WARP.md", "deep rules"),
                virtual_fs::Stub::FileWithContent(
                    "dir/.agents/skills/review/SKILL.md",
                    "name: review",
                ),
            ]);
        let dir = dirs.tests().join("dir");

        let results = evaluate_standing_queries(
            &dir,
            &definitions(),
            &StandingQueryWalkOptions {
                max_depth: 1,
                force_included_paths: vec![PathBuf::from(".agents/skills")],
            },
        );

        // First-level rules are found; deeper rules are not (matching the
        // lazily-loaded non-repository path behavior today).
        assert!(contains_rule(&results, &dir.join("WARP.md")));
        assert!(!contains_rule(&results, &dir.join("src/deep/WARP.md")));
        // Force-included provider subtrees are fully explored regardless of
        // the depth limit.
        assert!(contains_skill_file(
            &results,
            &dir.join(".agents/skills/review/SKILL.md")
        ));
    });
}

#[cfg(unix)]
#[test]
fn walker_reports_symlinked_skill_directories_via_lexical_paths() {
    virtual_fs::VirtualFS::test("walker_symlinked_skills", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills")
            .mkdir("targets/linked")
            .with_files(vec![virtual_fs::Stub::FileWithContent(
                "targets/linked/SKILL.md",
                "name: linked",
            )]);
        let repo = dirs.tests().join("repo");
        let linked_directory = repo.join(".agents/skills/linked");
        std::os::unix::fs::symlink(dirs.tests().join("targets/linked"), &linked_directory).unwrap();

        let results = evaluate_standing_queries(&repo, &definitions(), &walk_options());

        assert!(contains_skill_file(
            &results,
            &linked_directory.join("SKILL.md")
        ));
    });
}

/// The walker must observe the same matches as the standing-query evaluation
/// embedded in the eager tree build, so flag-gated consumers see identical
/// results regardless of the data path.
#[test]
fn walker_matches_tree_build_standing_results() {
    virtual_fs::VirtualFS::test("walker_parity_with_tree_build", |dirs, mut vfs| {
        vfs.mkdir("repo/.agents/skills/review")
            .mkdir("repo/packages/api")
            .mkdir("repo/ignored/nested")
            .mkdir("repo/.git/info")
            .with_files(vec![
                virtual_fs::Stub::FileWithContent(
                    "repo/.agents/skills/review/SKILL.md",
                    "name: review",
                ),
                virtual_fs::Stub::FileWithContent("repo/WARP.md", "root rules"),
                virtual_fs::Stub::FileWithContent("repo/AGENTS.md", "agents rules"),
                virtual_fs::Stub::FileWithContent("repo/packages/api/AGENTS.md", "nested rules"),
                virtual_fs::Stub::FileWithContent("repo/ignored/nested/WARP.md", "hidden rules"),
                virtual_fs::Stub::FileWithContent("repo/.git/info/WARP.md", "not a rule"),
            ]);
        let repo = dirs.tests().join("repo");
        std::fs::write(repo.join(".gitignore"), "ignored/\n").unwrap();

        let definitions = definitions();
        let force_included = vec![PathBuf::from(".agents/skills")];

        let mut files = Vec::new();
        let mut gitignores = crate::entry::gitignores_for_directory(&repo);
        let mut tree_results = StandingQueryResults::default();
        Entry::build_tree_with_standing_queries(
            &repo,
            &mut files,
            &mut gitignores,
            None,
            BuildTreeOptions {
                max_depth: super::STANDING_QUERY_WALK_MAX_DEPTH,
                current_depth: 0,
                ignored_path_strategy: &IgnoredPathStrategy::IncludeLazy,
                force_included_paths: &force_included,
                budget_exceeded_behavior: BudgetExceededBehavior::StopAndLazyLoad,
            },
            false,
            &mut tree_results,
            &definitions,
        )
        .unwrap();

        let walk_results = evaluate_standing_queries(
            &repo,
            &definitions,
            &StandingQueryWalkOptions {
                force_included_paths: force_included,
                ..Default::default()
            },
        );

        let tree_skills: HashSet<_> = tree_results.project_skills().cloned().collect();
        let walk_skills: HashSet<_> = walk_results.project_skills().cloned().collect();
        assert_eq!(walk_skills, tree_skills);

        let tree_rules: HashSet<_> = tree_results.project_rules().cloned().collect();
        let walk_rules: HashSet<_> = walk_results.project_rules().cloned().collect();
        assert_eq!(walk_rules, tree_rules);
    });
}
