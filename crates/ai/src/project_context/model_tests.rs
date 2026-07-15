use std::path::PathBuf;

use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

fn local_path(path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Local(PathBuf::from(path))
}

fn insert_remote_project_rule(
    model: &mut ProjectContextModel,
    host_id: &str,
    project_root: &str,
    rule_path: &str,
    content: &str,
) {
    let rules = model
        .path_to_rules
        .entry(remote_path(host_id, project_root))
        .or_default();
    rules.upsert_rule(&remote_path(host_id, rule_path), content.to_string());
}

fn remote_path(host_id: &str, path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        HostId::new(host_id.to_string()),
        StandardizedPath::try_new(path).unwrap(),
    ))
}

use super::*;

#[test]
fn test_find_applicable_rules_empty_rules() {
    let rules = ProjectRules { rules: vec![] };
    let path = local_path("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert!(result.is_empty());
}

#[test]
fn test_find_applicable_rules_no_matching_rules() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/x/y/WARP.md"), "content1".to_string());
    rules.upsert_rule(&local_path("/z/AGENTS.md"), "content2".to_string());

    let path = local_path("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert!(result.is_empty());
}

#[test]
fn test_find_applicable_rules_single_matching_rule() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/WARP.md"), "content1".to_string());
    rules.upsert_rule(&local_path("/x/AGENTS.md"), "content2".to_string());

    let path = local_path("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, local_path("/a/WARP.md"));
}

#[test]
fn test_find_applicable_rules_includes_all_ancestor_rules() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/WARP.md"), "root_warp".to_string());
    rules.upsert_rule(&local_path("/a/b/WARP.md"), "nested_warp".to_string());
    rules.upsert_rule(&local_path("/a/b/c/WARP.md"), "deep_warp".to_string());

    let path = local_path("/a/b/c/d/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 3);

    // All should be WARP.md files (same priority), order is not specified by depth
    // Just verify all expected rules are present
    let paths: Vec<LocalOrRemotePath> = result.iter().map(|r| r.path.clone()).collect();
    assert!(paths.contains(&local_path("/a/WARP.md")));
    assert!(paths.contains(&local_path("/a/b/WARP.md")));
    assert!(paths.contains(&local_path("/a/b/c/WARP.md")));
}

#[test]
fn test_find_applicable_rules_multiple_patterns() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/b/AGENTS.md"), "agents_content".to_string());
    rules.upsert_rule(&local_path("/a/WARP.md"), "warp_content".to_string());

    let path = local_path("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 2);

    assert_eq!(result[0].path, local_path("/a/b/AGENTS.md"));
    assert_eq!(result[0].content, "agents_content");
    assert_eq!(result[1].path, local_path("/a/WARP.md"));
    assert_eq!(result[1].content, "warp_content");
}

#[test]
fn test_find_applicable_rules_exact_path_match() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/b/WARP.md"), "exact_match".to_string());

    let path = local_path("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, local_path("/a/b/WARP.md"));
    assert_eq!(result[0].content, "exact_match");
}

#[test]
fn test_find_applicable_rules_ignores_deeper_paths() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/WARP.md"), "applicable".to_string());
    rules.upsert_rule(&local_path("/a/b/c/d/e/WARP.md"), "too_deep".to_string()); // Path doesn't contain /a/b

    let path = local_path("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, local_path("/a/WARP.md"));
    assert_eq!(result[0].content, "applicable");
}

#[test]
fn test_find_applicable_rules_handles_root_path() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/WARP.md"), "root_rule".to_string());

    let path = local_path("/a/b/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path, local_path("/WARP.md"));
    assert_eq!(result[0].content, "root_rule");
}

#[test]
fn test_find_applicable_rules_complex_scenario() {
    // This test covers the example from the original request:
    // For path /a/b/c/file.rs with rules:
    // - /a/WARP.md
    // - /a/AGENTS.md
    // - /a/b/WARP.md
    // - /a/b/AGENTS.md
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/WARP.md"), "a_warp".to_string());
    rules.upsert_rule(&local_path("/a/AGENTS.md"), "a_agents".to_string());
    rules.upsert_rule(&local_path("/a/b/WARP.md"), "ab_warp".to_string());
    rules.upsert_rule(&local_path("/a/b/AGENTS.md"), "ab_agents".to_string());
    rules.upsert_rule(&local_path("/x/WARP.md"), "irrelevant".to_string()); // Should be ignored

    let path = local_path("/a/b/c/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 2);

    // Expect only WARP.md files to be included as they have higher priority.
    assert_eq!(result[0].path, local_path("/a/WARP.md"));
    assert_eq!(result[0].content, "a_warp");
    assert_eq!(result[1].path, local_path("/a/b/WARP.md"));
    assert_eq!(result[1].content, "ab_warp");
}

#[test]
fn test_find_applicable_rules_handles_unknown_file_patterns() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("/a/WARP.md"), "known_pattern".to_string());
    rules.upsert_rule(&local_path("/a/UNKNOWN.md"), "unknown_pattern".to_string());
    let path = local_path("/a/file.rs");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 1);

    assert_eq!(result[0].path, local_path("/a/WARP.md"));
    assert_eq!(result[0].content, "known_pattern");
}

#[test]
fn test_find_applicable_rules_with_relative_paths() {
    let mut rules = ProjectRules::default();

    rules.upsert_rule(&local_path("src/WARP.md"), "src_warp".to_string());
    rules.upsert_rule(
        &local_path("src/components/WARP.md"),
        "components_warp".to_string(),
    );

    let path = local_path("src/components/Button.tsx");

    let result = rules.find_active_or_applicable_rules(&path).active_rules;
    assert_eq!(result.len(), 2);

    // Both are WARP.md files (same priority), order within same priority is not guaranteed
    // Just verify both rules are present
    let paths: Vec<LocalOrRemotePath> = result.iter().map(|r| r.path.clone()).collect();
    assert!(paths.contains(&local_path("src/WARP.md")));
    assert!(paths.contains(&local_path("src/components/WARP.md")));
}

fn make_rule_path(path: &str) -> ProjectRulePath {
    ProjectRulePath {
        path: PathBuf::from(path),
        project_root: PathBuf::from("/project"),
    }
}

#[test]
fn test_merge_independent_deltas() {
    let mut delta = RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    };
    delta.merge(RulesDelta {
        discovered_rules: vec![],
        deleted_rules: vec![PathBuf::from("/b/WARP.md")],
    });

    assert_eq!(delta.discovered_rules.len(), 1);
    assert_eq!(delta.discovered_rules[0].path, PathBuf::from("/a/WARP.md"));
    assert_eq!(delta.deleted_rules, vec![PathBuf::from("/b/WARP.md")]);
}

#[test]
fn test_merge_add_then_delete_yields_delete() {
    let mut delta = RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    };
    delta.merge(RulesDelta {
        discovered_rules: vec![],
        deleted_rules: vec![PathBuf::from("/a/WARP.md")],
    });

    assert!(delta.discovered_rules.is_empty());
    assert_eq!(delta.deleted_rules, vec![PathBuf::from("/a/WARP.md")]);
}

#[test]
fn test_merge_delete_then_add_yields_add() {
    let mut delta = RulesDelta {
        discovered_rules: vec![],
        deleted_rules: vec![PathBuf::from("/a/WARP.md")],
    };
    delta.merge(RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    });

    assert_eq!(delta.discovered_rules.len(), 1);
    assert_eq!(delta.discovered_rules[0].path, PathBuf::from("/a/WARP.md"));
    assert!(delta.deleted_rules.is_empty());
}

#[test]
fn test_merge_add_delete_add_yields_add() {
    let mut delta = RulesDelta::default();
    delta.merge(RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    });
    delta.merge(RulesDelta {
        discovered_rules: vec![],
        deleted_rules: vec![PathBuf::from("/a/WARP.md")],
    });
    delta.merge(RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    });

    assert_eq!(delta.discovered_rules.len(), 1);
    assert_eq!(delta.discovered_rules[0].path, PathBuf::from("/a/WARP.md"));
    assert!(delta.deleted_rules.is_empty());
}

#[test]
fn test_merge_delete_add_delete_yields_delete() {
    let mut delta = RulesDelta::default();
    delta.merge(RulesDelta {
        discovered_rules: vec![],
        deleted_rules: vec![PathBuf::from("/a/WARP.md")],
    });
    delta.merge(RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    });
    delta.merge(RulesDelta {
        discovered_rules: vec![],
        deleted_rules: vec![PathBuf::from("/a/WARP.md")],
    });

    assert!(delta.discovered_rules.is_empty());
    assert_eq!(delta.deleted_rules, vec![PathBuf::from("/a/WARP.md")]);
}

#[test]
fn test_merge_rediscovery_keeps_latest() {
    let mut delta = RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    };
    // A second discovery of the same path (content update) should deduplicate.
    delta.merge(RulesDelta {
        discovered_rules: vec![make_rule_path("/a/WARP.md")],
        deleted_rules: vec![],
    });

    assert_eq!(delta.discovered_rules.len(), 1);
    assert!(delta.deleted_rules.is_empty());
}

#[test]
fn test_missing_rule_content_preserves_cached_content_while_path_is_standing() {
    let rule_path = local_path("/unavailable/project/WARP.md");
    let mut existing_rules = ProjectRules::default();
    existing_rules.upsert_rule(&rule_path, "cached content".to_string());

    let rules = ProjectContextModel::reconcile_project_rules(
        vec![rule_path.clone()],
        Vec::new(),
        existing_rules,
    );
    let result = rules.find_active_or_applicable_rules(&local_path("/unavailable/project/main.rs"));

    assert_eq!(result.active_rules.len(), 1);
    assert_eq!(result.active_rules[0].path, rule_path);
    assert_eq!(result.active_rules[0].content, "cached content");
}

#[test]
fn test_rule_missing_from_standing_results_is_removed_from_cached_content() {
    let rule_path = local_path("/unavailable/project/WARP.md");
    let mut existing_rules = ProjectRules::default();
    existing_rules.upsert_rule(&rule_path, "cached content".to_string());

    let rules =
        ProjectContextModel::reconcile_project_rules(Vec::new(), Vec::new(), existing_rules);
    assert!(rules.rule_paths().next().is_none());
}

#[test]
fn test_reconcile_project_rules_hydrates_local_and_remote_paths() {
    let local_rule_path = local_path("/local/WARP.md");
    let remote_rule_path = remote_path("host-a", "/remote/AGENTS.md");

    let rules = ProjectContextModel::reconcile_project_rules(
        vec![local_rule_path.clone(), remote_rule_path.clone()],
        vec![
            (local_rule_path.clone(), "local content".to_string()),
            (remote_rule_path.clone(), "remote content".to_string()),
        ],
        ProjectRules::default(),
    );

    let local_result = rules.find_active_or_applicable_rules(&local_path("/local/main.rs"));
    assert_eq!(local_result.active_rules.len(), 1);
    assert_eq!(local_result.active_rules[0].path, local_rule_path);
    assert_eq!(local_result.active_rules[0].content, "local content");

    let remote_result =
        rules.find_active_or_applicable_rules(&remote_path("host-a", "/remote/main.rs"));
    assert_eq!(remote_result.active_rules.len(), 1);
    assert_eq!(remote_result.active_rules[0].path, remote_rule_path);
    assert_eq!(remote_result.active_rules[0].content, "remote content");
}

#[cfg(feature = "local_fs")]
#[test]
fn test_remote_standing_results_preserve_host_qualified_rule_paths() {
    let host = HostId::new("test-host".to_string());
    let repo_id = RepositoryIdentifier::Remote(RemotePath::new(
        host.clone(),
        StandardizedPath::try_new("/repo").unwrap(),
    ));
    let rule_path = StandardizedPath::try_new("/repo/nested/WARP.md").unwrap();
    let contents = [
        StandingQueryContent::file(rule_path.clone()),
        StandingQueryContent::directory(StandardizedPath::try_new("/repo/nested").unwrap()),
    ];

    assert_eq!(
        standing_project_rule_paths(&repo_id, &contents),
        vec![LocalOrRemotePath::Remote(RemotePath::new(host, rule_path))]
    );
}

// Helper for global-rules tests: inserts a synthetic global rule directly into
// the model. Bypasses the watcher infrastructure (which requires the warpui
// runtime) so we can exercise `find_applicable_rules`'s layering logic.
fn insert_global_rule(model: &mut ProjectContextModel, path: &Path, content: &str) {
    model.global_rules.rules.insert(
        path.to_path_buf(),
        ProjectRule {
            path: LocalOrRemotePath::Local(path.to_path_buf()),
            content: content.to_string(),
        },
    );
}

fn insert_project_rule(
    model: &mut ProjectContextModel,
    project_root: &Path,
    rule_path: &Path,
    content: &str,
) {
    let rules = model
        .path_to_rules
        .entry(LocalOrRemotePath::Local(project_root.to_path_buf()))
        .or_default();
    rules.upsert_rule(
        &LocalOrRemotePath::Local(rule_path.to_path_buf()),
        content.to_string(),
    );
}

#[test]
fn test_remote_project_rules_require_matching_host() {
    let mut model = ProjectContextModel::default();
    insert_remote_project_rule(
        &mut model,
        "host-a",
        "/repo",
        "/repo/WARP.md",
        "remote_project_rule",
    );

    let same_host = model
        .find_applicable_project_rules(&remote_path("host-a", "/repo/src/main.rs"))
        .expect("same-host remote rule should apply");
    assert_eq!(same_host.root_path, remote_path("host-a", "/repo"));
    assert_eq!(same_host.active_rules.len(), 1);
    assert_eq!(same_host.active_rules[0].content, "remote_project_rule");

    let other_host =
        model.find_applicable_project_rules(&remote_path("host-b", "/repo/src/main.rs"));
    assert!(other_host.is_none());
}

#[test]
fn test_global_rule_alone_no_project_rules() {
    let mut model = ProjectContextModel::default();
    insert_global_rule(
        &mut model,
        Path::new("/home/u/.agents/AGENTS.md"),
        "global_content",
    );

    let result = model
        .find_applicable_rules(&local_path("/some/project/file.rs"))
        .expect("global rule should produce a result");

    assert_eq!(result.active_rules.len(), 1);
    assert_eq!(
        result.active_rules[0].path,
        local_path("/home/u/.agents/AGENTS.md")
    );
    assert_eq!(result.active_rules[0].content, "global_content");
    assert!(result.additional_rule_paths.is_empty());
}

#[test]
fn test_global_rule_layered_with_project_warp() {
    let mut model = ProjectContextModel::default();
    insert_global_rule(&mut model, Path::new("/home/u/.agents/AGENTS.md"), "global");
    insert_project_rule(
        &mut model,
        Path::new("/repo"),
        Path::new("/repo/WARP.md"),
        "project_warp",
    );

    let result = model
        .find_applicable_rules(&local_path("/repo/src/main.rs"))
        .expect("layered rules should produce a result");

    // Layered precedence: global first, then project rules.
    assert_eq!(result.active_rules.len(), 2);
    assert_eq!(result.active_rules[0].content, "global");
    assert_eq!(result.active_rules[1].content, "project_warp");
    assert_eq!(result.root_path, local_path("/repo"));
}

#[test]
fn test_in_dir_warp_shadows_agents_with_global() {
    let mut model = ProjectContextModel::default();
    insert_global_rule(&mut model, Path::new("/home/u/.agents/AGENTS.md"), "global");
    // Both WARP.md and AGENTS.md in the same project directory: WARP.md should
    // shadow AGENTS.md (existing in-directory behavior preserved).
    insert_project_rule(
        &mut model,
        Path::new("/repo"),
        Path::new("/repo/WARP.md"),
        "project_warp",
    );
    insert_project_rule(
        &mut model,
        Path::new("/repo"),
        Path::new("/repo/AGENTS.md"),
        "project_agents",
    );

    let result = model
        .find_applicable_rules(&local_path("/repo/src/main.rs"))
        .expect("layered rules should produce a result");

    // Expect: [global, project WARP.md]. project AGENTS.md is shadowed.
    assert_eq!(result.active_rules.len(), 2);
    assert_eq!(result.active_rules[0].content, "global");
    assert_eq!(result.active_rules[1].content, "project_warp");
}

#[test]
fn test_no_rules_returns_none() {
    let model = ProjectContextModel::default();
    let result = model.find_applicable_rules(&local_path("/some/path/file.rs"));
    assert!(result.is_none());
}

#[test]
fn test_global_rule_root_path_falls_back_to_parent() {
    let mut model = ProjectContextModel::default();
    insert_global_rule(&mut model, Path::new("/home/u/.agents/AGENTS.md"), "global");

    let result = model
        .find_applicable_rules(&local_path("/some/file.rs"))
        .expect("global rule should produce a result");

    // No project root indexed; root_path falls back to parent of the global rule.
    assert_eq!(result.root_path, local_path("/home/u/.agents"));
}

#[test]
fn test_multiple_global_rules_all_contribute() {
    let mut model = ProjectContextModel::default();
    insert_global_rule(
        &mut model,
        Path::new("/home/u/.agents/AGENTS.md"),
        "agents_global",
    );
    insert_global_rule(
        &mut model,
        Path::new("/home/u/.warp/WARP.md"),
        "warp_global",
    );

    let result = model
        .find_applicable_rules(&local_path("/repo/src/main.rs"))
        .expect("globals should produce a result");

    assert_eq!(result.active_rules.len(), 2);
    let contents: Vec<&str> = result
        .active_rules
        .iter()
        .map(|r| r.content.as_str())
        .collect();
    assert!(contents.contains(&"agents_global"));
    assert!(contents.contains(&"warp_global"));
}

#[test]
fn test_remote_global_rules_only_layer_for_matching_remote_host() {
    let mut model = ProjectContextModel::default();
    insert_global_rule(
        &mut model,
        Path::new("/home/local/.agents/AGENTS.md"),
        "local_global",
    );
    insert_remote_project_rule(
        &mut model,
        "host-a",
        "/repo",
        "/repo/WARP.md",
        "remote_project",
    );
    let host_a = HostId::new("host-a".to_string());
    model.set_remote_global_rules(
        host_a.clone(),
        vec![ProjectRule {
            path: remote_path("host-a", "/home/remote/.agents/AGENTS.md"),
            content: "remote_global".to_string(),
        }],
    );
    model.set_remote_global_rules(
        HostId::new("host-b".to_string()),
        vec![ProjectRule {
            path: remote_path("host-b", "/home/remote/.agents/AGENTS.md"),
            content: "other_remote_global".to_string(),
        }],
    );

    let matching = model
        .find_applicable_rules(&remote_path("host-a", "/repo/src/main.rs"))
        .unwrap();
    assert_eq!(
        matching
            .active_rules
            .iter()
            .map(|rule| rule.content.as_str())
            .collect::<Vec<_>>(),
        ["local_global", "remote_global", "remote_project"]
    );

    let other_host = model
        .find_applicable_rules(&remote_path("host-b", "/repo/src/main.rs"))
        .unwrap();
    assert_eq!(
        other_host
            .active_rules
            .iter()
            .map(|rule| rule.content.as_str())
            .collect::<Vec<_>>(),
        ["local_global", "other_remote_global"]
    );

    let local = model
        .find_applicable_rules(&local_path("/repo/src/main.rs"))
        .unwrap();
    assert_eq!(local.active_rules.len(), 1);
    assert_eq!(local.active_rules[0].content, "local_global");

    assert_eq!(
        model.global_rule_paths().collect::<Vec<_>>(),
        [local_path("/home/local/.agents/AGENTS.md")]
    );

    model.set_remote_global_rules(host_a, Vec::new());
    let replaced = model
        .find_applicable_rules(&remote_path("host-a", "/repo/src/main.rs"))
        .unwrap();
    assert_eq!(
        replaced
            .active_rules
            .iter()
            .map(|rule| rule.content.as_str())
            .collect::<Vec<_>>(),
        ["local_global", "remote_project"]
    );
}

// ============================================================================
// Tests for the flag-on walk-based local rule discovery path
// ============================================================================

#[cfg(feature = "local_fs")]
mod standing_query_walk_tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::time::Duration;

    use futures::FutureExt as _;
    use repo_metadata::repositories::DetectedRepositories;
    use repo_metadata::{
        DirectoryWatcher, RepoMetadataModel, RepositoryIdentifier, RepositoryUpdate, TargetFile,
    };
    use warp_core::features::FeatureFlag;
    use warpui_core::r#async::Timer;
    use warpui_core::App;

    use super::*;

    /// Test rule content reader that reads local files directly.
    fn read_local_rule_contents(
        rule_paths: Vec<LocalOrRemotePath>,
        _ctx: &AppContext,
    ) -> BoxFuture<'static, anyhow::Result<ProjectRuleContents>> {
        async move {
            Ok(rule_paths
                .into_iter()
                .filter_map(|path| {
                    let local_path = path.to_local_path()?.to_path_buf();
                    fs::read_to_string(local_path).ok().map(|c| (path, c))
                })
                .collect())
        }
        .boxed()
    }

    /// Polls until the model reports active project rules for `path`, returning
    /// the active rule paths sorted for stable assertions.
    async fn wait_for_active_rules(
        app: &App,
        model_handle: &warpui_core::ModelHandle<ProjectContextModel>,
        path: &LocalOrRemotePath,
        expected_count: usize,
    ) -> Vec<LocalOrRemotePath> {
        for _ in 0..500 {
            let active = model_handle.read(app, |model, _| {
                model
                    .find_applicable_project_rules(path)
                    .map(|result| {
                        result
                            .active_rules
                            .iter()
                            .map(|rule| rule.path.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            });
            if active.len() >= expected_count {
                let mut active = active;
                active.sort_by_key(LocalOrRemotePath::display_path);
                return active;
            }
            Timer::after(Duration::from_millis(10)).await;
        }
        panic!("Timed out waiting for {expected_count} active rules at {path:?}");
    }

    fn rule_update(paths: Vec<std::path::PathBuf>) -> RepositoryUpdate {
        RepositoryUpdate {
            added: HashSet::new(),
            modified: paths
                .into_iter()
                .map(|path| TargetFile::new(path, false))
                .collect(),
            deleted: HashSet::new(),
            moved: HashMap::new(),
            commit_updated: false,
            index_lock_detected: false,
            remote_ref_updated: false,
        }
    }

    #[test]
    fn walk_discovers_rules_for_git_repo_at_any_depth() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::OnTheFlyStandingQueries.override_enabled(true);
            app.add_singleton_model(DirectoryWatcher::new_for_testing);
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(RepoMetadataModel::new);
            let model_handle = app.add_singleton_model(|ctx| {
                ProjectContextModel::new_from_persisted(Vec::new(), read_local_rule_contents, ctx)
            });

            let temp_dir = tempfile::tempdir().unwrap();
            let repo = dunce::canonicalize(temp_dir.path()).unwrap();
            fs::create_dir_all(repo.join(".git")).unwrap();
            fs::create_dir_all(repo.join("packages/api")).unwrap();
            fs::write(repo.join("WARP.md"), "root rules").unwrap();
            fs::write(repo.join("packages/api/AGENTS.md"), "nested rules").unwrap();

            let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
            model_handle.update(&mut app, |model, ctx| {
                model.ensure_local_rule_discovery(&repo_id, ctx);
            });

            let active = wait_for_active_rules(
                &app,
                &model_handle,
                &LocalOrRemotePath::Local(repo.join("packages/api/src")),
                2,
            )
            .await;
            let mut expected = vec![
                LocalOrRemotePath::Local(repo.join("WARP.md")),
                LocalOrRemotePath::Local(repo.join("packages/api/AGENTS.md")),
            ];
            expected.sort_by_key(LocalOrRemotePath::display_path);
            assert_eq!(active, expected);
        });
    }

    #[test]
    fn walk_limits_non_git_directories_to_first_level_rules() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::OnTheFlyStandingQueries.override_enabled(true);
            app.add_singleton_model(DirectoryWatcher::new_for_testing);
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(RepoMetadataModel::new);
            let model_handle = app.add_singleton_model(|ctx| {
                ProjectContextModel::new_from_persisted(Vec::new(), read_local_rule_contents, ctx)
            });

            let temp_dir = tempfile::tempdir().unwrap();
            let dir = dunce::canonicalize(temp_dir.path()).unwrap();
            fs::create_dir_all(dir.join("nested/deep")).unwrap();
            fs::write(dir.join("WARP.md"), "root rules").unwrap();
            fs::write(dir.join("nested/deep/WARP.md"), "deep rules").unwrap();

            model_handle.update(&mut app, |model, ctx| {
                model
                    .index_and_store_rules(dir.clone(), read_local_rule_contents, ctx)
                    .unwrap();
            });

            let active = wait_for_active_rules(
                &app,
                &model_handle,
                &LocalOrRemotePath::Local(dir.join("nested/deep")),
                1,
            )
            .await;
            // Only the first-level rule is discovered, matching the coverage of
            // the lazily-loaded path behavior with the flag off.
            assert_eq!(active, vec![LocalOrRemotePath::Local(dir.join("WARP.md"))]);
        });
    }

    #[test]
    fn walk_rediscovers_rules_after_rule_file_edit() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::OnTheFlyStandingQueries.override_enabled(true);
            app.add_singleton_model(DirectoryWatcher::new_for_testing);
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(RepoMetadataModel::new);
            let model_handle = app.add_singleton_model(|ctx| {
                ProjectContextModel::new_from_persisted(Vec::new(), read_local_rule_contents, ctx)
            });

            let temp_dir = tempfile::tempdir().unwrap();
            let repo = dunce::canonicalize(temp_dir.path()).unwrap();
            fs::create_dir_all(repo.join(".git")).unwrap();
            fs::write(repo.join("WARP.md"), "root rules").unwrap();

            let repo_id = RepositoryIdentifier::try_local(&repo).unwrap();
            let root_dir = StandardizedPath::try_from_local(&repo).unwrap();
            model_handle.update(&mut app, |model, ctx| {
                model.ensure_local_rule_discovery(&repo_id, ctx);
            });
            wait_for_active_rules(
                &app,
                &model_handle,
                &LocalOrRemotePath::Local(repo.join("src")),
                1,
            )
            .await;

            // A new rule file appears; the watcher update triggers a re-walk.
            fs::create_dir_all(repo.join("packages")).unwrap();
            fs::write(repo.join("packages/AGENTS.md"), "new rules").unwrap();
            model_handle.update(&mut app, |model, ctx| {
                model.handle_local_rule_watch_update(
                    root_dir,
                    &rule_update(vec![repo.join("packages/AGENTS.md")]),
                    ctx,
                );
            });

            let active = wait_for_active_rules(
                &app,
                &model_handle,
                &LocalOrRemotePath::Local(repo.join("packages/src")),
                2,
            )
            .await;
            assert!(active.contains(&LocalOrRemotePath::Local(repo.join("packages/AGENTS.md"))));
        });
    }

    #[test]
    fn irrelevant_updates_do_not_schedule_rule_refreshes() {
        App::test((), |mut app| async move {
            let _flag = FeatureFlag::OnTheFlyStandingQueries.override_enabled(true);
            app.add_singleton_model(DirectoryWatcher::new_for_testing);
            app.add_singleton_model(|_| DetectedRepositories::default());
            app.add_singleton_model(RepoMetadataModel::new);
            let model_handle = app.add_singleton_model(|ctx| {
                ProjectContextModel::new_from_persisted(Vec::new(), read_local_rule_contents, ctx)
            });

            let temp_dir = tempfile::tempdir().unwrap();
            let repo = dunce::canonicalize(temp_dir.path()).unwrap();
            let root_dir = StandardizedPath::try_from_local(&repo).unwrap();

            model_handle.update(&mut app, |model, ctx| {
                model.handle_local_rule_watch_update(
                    root_dir,
                    &rule_update(vec![repo.join("src/main.rs")]),
                    ctx,
                );
                // No refresh was scheduled for an irrelevant change.
                assert!(model.rule_refresh_generations.is_empty());
            });
        });
    }
}
