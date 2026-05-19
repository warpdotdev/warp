use super::*;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;
use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
use repo_metadata::{repositories::DetectedRepositories, DirectoryWatcher, RepoMetadataModel};
use std::collections::{HashMap, HashSet};
use std::fs;
use tempfile::TempDir;
use warp_core::channel::ChannelState;
use warpui::App;
use watcher::HomeDirectoryWatcher;

// ============================================================================
// Tests for get_skills_for_working_directory subdirectory scoping
// ============================================================================

#[test]
fn get_skills_for_working_directory_scopes_subdirectory_skills() {
    // This test verifies the key scoping behavior:
    // - Root skills are visible from anywhere in the repo
    // - Subdirectory skills are only visible when working_directory is within that subdirectory

    // Use real temp directories so DetectedRepositories can canonicalize paths
    // and correctly report repo_root, which controls ancestor-vs-descendant scoping.
    // Canonicalize the temp base to avoid macOS /var -> /private/var symlink mismatches.
    let temp = TempDir::new().unwrap();
    let base = dunce::canonicalize(temp.path()).unwrap();
    let repo = base.join("repo");
    let frontend_dir = repo.join("packages/frontend");
    let backend_dir = repo.join("packages/backend");
    fs::create_dir_all(&frontend_dir).unwrap();
    fs::create_dir_all(&backend_dir).unwrap();

    // Create mock skills
    let root_skill_path = repo.join(".agents/skills/root-skill/SKILL.md");
    let frontend_skill_path = frontend_dir.join(".agents/skills/frontend-skill/SKILL.md");

    let root_skill = ParsedSkill {
        name: "root-skill".to_string(),
        description: "A root skill".to_string(),
        path: root_skill_path.clone(),
        content: "# Root skill".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let frontend_skill = ParsedSkill {
        name: "frontend-skill".to_string(),
        description: "A frontend skill".to_string(),
        path: frontend_skill_path.clone(),
        content: "# Frontend skill".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    // Build the internal state manually
    let mut directory_skills: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
    directory_skills
        .entry(repo.clone())
        .or_default()
        .insert(root_skill_path.clone());
    directory_skills
        .entry(frontend_dir.clone())
        .or_default()
        .insert(frontend_skill_path.clone());

    let mut skills_by_path: HashMap<PathBuf, ParsedSkill> = HashMap::new();
    skills_by_path.insert(root_skill_path.clone(), root_skill);
    skills_by_path.insert(frontend_skill_path.clone(), frontend_skill);

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        let repo_handle = app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let skill_manager_handle = app.add_singleton_model(SkillManager::new);

        // Register the repo root so get_root_for_path returns Some.
        let canonical_repo =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                .unwrap();
        repo_handle.update(&mut app, |repos, _ctx| {
            repos.insert_test_repo_root(canonical_repo);
        });

        // Inject the test state
        skill_manager_handle.update(&mut app, |manager, _ctx| {
            manager.directory_skills = directory_skills;
            manager.skills_by_path = skills_by_path;
        });

        // Test 1: From frontend directory, should see both root and frontend skills
        let skills_from_frontend = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(Some(&frontend_dir), ctx)
        });
        let names_from_frontend: Vec<&str> = skills_from_frontend
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            names_from_frontend.contains(&"root-skill"),
            "Root skill should be visible from frontend dir"
        );
        assert!(
            names_from_frontend.contains(&"frontend-skill"),
            "Frontend skill should be visible from frontend dir"
        );

        // Test 2: From backend directory, should only see root skill (not frontend skill)
        let skills_from_backend = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(Some(&backend_dir), ctx)
        });
        let names_from_backend: Vec<&str> = skills_from_backend
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            names_from_backend.contains(&"root-skill"),
            "Root skill should be visible from backend dir"
        );
        assert!(
            !names_from_backend.contains(&"frontend-skill"),
            "Frontend skill should NOT be visible from backend dir"
        );

        // Test 3: From repo root, should only see root skill (not frontend skill)
        let skills_from_root = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(Some(&repo), ctx)
        });
        let names_from_root: Vec<&str> = skills_from_root.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names_from_root.contains(&"root-skill"),
            "Root skill should be visible from repo root"
        );
        assert!(
            !names_from_root.contains(&"frontend-skill"),
            "Frontend skill should NOT be visible from repo root"
        );
    });
}

#[test]
fn get_skills_for_working_directory_name_collision_returns_both() {
    // When the same skill name exists at root and subdirectory, both should be returned.
    // The caller (agent) is responsible for precedence based on path proximity.

    // Use real temp directories so DetectedRepositories can canonicalize paths.
    // Canonicalize the temp base to avoid macOS /var -> /private/var symlink mismatches.
    let temp = TempDir::new().unwrap();
    let base = dunce::canonicalize(temp.path()).unwrap();
    let repo = base.join("repo");
    let subdir = repo.join("packages/frontend");
    fs::create_dir_all(&subdir).unwrap();

    let root_skill_path = repo.join(".agents/skills/deploy/SKILL.md");
    let subdir_skill_path = subdir.join(".agents/skills/deploy/SKILL.md");

    let root_skill = ParsedSkill {
        name: "deploy".to_string(),
        description: "Root deploy".to_string(),
        path: root_skill_path.clone(),
        content: "# Root deploy".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let subdir_skill = ParsedSkill {
        name: "deploy".to_string(),
        description: "Subdir deploy".to_string(),
        path: subdir_skill_path.clone(),
        content: "# Subdir deploy".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let mut directory_skills: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
    directory_skills
        .entry(repo.clone())
        .or_default()
        .insert(root_skill_path.clone());
    directory_skills
        .entry(subdir.clone())
        .or_default()
        .insert(subdir_skill_path.clone());

    let mut skills_by_path: HashMap<PathBuf, ParsedSkill> = HashMap::new();
    skills_by_path.insert(root_skill_path.clone(), root_skill);
    skills_by_path.insert(subdir_skill_path.clone(), subdir_skill);

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        let repo_handle = app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let skill_manager_handle = app.add_singleton_model(SkillManager::new);

        // Register the repo root so get_root_for_path returns Some.
        let canonical_repo =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo)
                .unwrap();
        repo_handle.update(&mut app, |repos, _ctx| {
            repos.insert_test_repo_root(canonical_repo);
        });

        skill_manager_handle.update(&mut app, |manager, _ctx| {
            manager.directory_skills = directory_skills;
            manager.skills_by_path = skills_by_path;
        });

        // From subdir: should see both "deploy" skills (root + subdir)
        let skills = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(Some(&subdir), ctx)
        });
        let deploy_skills: Vec<_> = skills.iter().filter(|s| s.name == "deploy").collect();
        assert_eq!(
            deploy_skills.len(),
            2,
            "Both deploy skills should be visible from subdir"
        );

        // From repo root: should only see root "deploy"
        let skills = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(Some(&repo), ctx)
        });
        let deploy_skills: Vec<_> = skills.iter().filter(|s| s.name == "deploy").collect();
        assert_eq!(
            deploy_skills.len(),
            1,
            "Only root deploy should be visible from repo root"
        );
        assert_eq!(deploy_skills[0].description, "Root deploy");
    });
}

#[test]
fn cloud_environment_skills_always_included() {
    // In a cloud environment, all skills should be in scope regardless of
    // the working directory—even when cwd is inside a different repo or
    // when working_directory is None.

    let temp = TempDir::new().unwrap();
    let base = dunce::canonicalize(temp.path()).unwrap();
    let repo_a = base.join("repo-a");
    let repo_b = base.join("repo-b");
    fs::create_dir_all(&repo_a).unwrap();
    fs::create_dir_all(&repo_b).unwrap();

    let skill_a_path = repo_a.join(".agents/skills/build/SKILL.md");
    let skill_b_path = repo_b.join(".agents/skills/deploy/SKILL.md");

    let skill_a = ParsedSkill {
        name: "build".to_string(),
        description: "Repo A skill".to_string(),
        path: skill_a_path.clone(),
        content: "# Build".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let skill_b = ParsedSkill {
        name: "deploy".to_string(),
        description: "Repo B skill".to_string(),
        path: skill_b_path.clone(),
        content: "# Deploy".to_string(),
        line_range: None,
        provider: SkillProvider::Agents,
        scope: SkillScope::Project,
    };

    let mut directory_skills: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
    directory_skills
        .entry(repo_a.clone())
        .or_default()
        .insert(skill_a_path.clone());
    directory_skills
        .entry(repo_b.clone())
        .or_default()
        .insert(skill_b_path.clone());

    let mut skills_by_path: HashMap<PathBuf, ParsedSkill> = HashMap::new();
    skills_by_path.insert(skill_a_path.clone(), skill_a);
    skills_by_path.insert(skill_b_path.clone(), skill_b);

    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        let repo_handle = app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let skill_manager_handle = app.add_singleton_model(SkillManager::new);

        let canonical_repo_a =
            warp_util::standardized_path::StandardizedPath::from_local_canonicalized(&repo_a)
                .unwrap();
        repo_handle.update(&mut app, |repos, _ctx| {
            repos.insert_test_repo_root(canonical_repo_a);
        });

        skill_manager_handle.update(&mut app, |manager, _ctx| {
            manager.directory_skills = directory_skills;
            manager.skills_by_path = skills_by_path;
            manager.is_cloud_environment = true;
        });

        // From inside repo_a, both repo_a and repo_b skills are visible
        // because is_cloud_environment skips the ancestor filter.
        let skills = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(Some(&repo_a), ctx)
        });
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"build"),
            "Repo A skill should be visible from repo A"
        );
        assert!(
            names.contains(&"deploy"),
            "Repo B skill should be visible from repo A in cloud environment"
        );

        // With no working directory, all skills are still included.
        let skills_none = skill_manager_handle.read(&app, |manager, ctx| {
            manager.get_skills_for_working_directory(None, ctx)
        });
        let names_none: Vec<&str> = skills_none.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names_none.contains(&"build"),
            "Repo A skill should be visible even without a working directory"
        );
        assert!(
            names_none.contains(&"deploy"),
            "Repo B skill should be visible even without a working directory"
        );
    });
}

#[test]
fn test_read_bundled_skills_with_variable_substitution() {
    let temp_dir = TempDir::new().unwrap();
    let skills_dir = temp_dir.path();

    // Create a test skill with variables
    let skill_dir = skills_dir.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(
        &skill_file,
        r#"---
name: test-skill
description: Test skill with variables
---

Run `{{warp_cli_binary_name}}` to connect to {{warp_server_url}}.
"#,
    )
    .unwrap();

    let skills = futures::executor::block_on(read_bundled_skills(skills_dir));

    assert_eq!(skills.len(), 1);
    let skill = skills.get("test-skill").unwrap();

    let expected_cli = ChannelState::channel().cli_command_name();
    let expected_url = ChannelState::server_root_url();
    assert!(skill.content.contains(&format!(
        "Run `{expected_cli}` to connect to {expected_url}."
    )));
}

#[test]
fn test_read_bundled_skills_preserves_other_content() {
    let temp_dir = TempDir::new().unwrap();
    let skills_dir = temp_dir.path();

    // Create a test skill with both warp and non-warp variables
    let skill_dir = skills_dir.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(
        &skill_file,
        r#"---
name: test-skill
description: Test skill with mixed variables
---

Use {{other_var}} and {{warp_cli_binary_name}} together.
"#,
    )
    .unwrap();

    let skills = futures::executor::block_on(read_bundled_skills(skills_dir));

    assert_eq!(skills.len(), 1);
    let skill = skills.get("test-skill").unwrap();

    let expected_cli = ChannelState::channel().cli_command_name();
    assert!(skill.content.contains(&format!(
        "Use {{{{other_var}}}} and {expected_cli} together."
    )));
}

#[test]
fn test_read_bundled_skills_no_variables() {
    let temp_dir = TempDir::new().unwrap();
    let skills_dir = temp_dir.path();

    // Create a test skill with no variables
    let skill_dir = skills_dir.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(
        &skill_file,
        r#"---
name: test-skill
description: Test skill without variables
---

Plain content with no variables.
"#,
    )
    .unwrap();

    let skills = futures::executor::block_on(read_bundled_skills(skills_dir));

    assert_eq!(skills.len(), 1);
    let skill = skills.get("test-skill").unwrap();
    assert!(skill.content.contains("Plain content with no variables."));
}

#[test]
fn test_build_bundled_skill_context() {
    let context = build_bundled_skill_context();

    // At least 5 entries: server_url, cli_binary_name, url_scheme, settings_file_path, keybindings_file_path.
    // settings_schema_path is only present when bundled_resources_dir() returns Some.
    assert!(context.len() >= 5);
    assert!(context.contains_key("warp_server_url"));
    assert!(context.contains_key("warp_cli_binary_name"));
    assert!(context.contains_key("warp_url_scheme"));
    assert!(context.contains_key("settings_file_path"));
    assert!(context.contains_key("keybindings_file_path"));

    assert_eq!(
        context.get("warp_server_url").unwrap(),
        &ChannelState::server_root_url().to_string()
    );
    assert_eq!(
        context.get("warp_cli_binary_name").unwrap(),
        ChannelState::channel().cli_command_name()
    );
    assert_eq!(
        context.get("warp_url_scheme").unwrap(),
        ChannelState::url_scheme()
    );
    assert_eq!(
        context.get("settings_file_path").unwrap(),
        &crate::settings::user_preferences_toml_file_path()
            .display()
            .to_string()
    );
    assert_eq!(
        context.get("keybindings_file_path").unwrap(),
        &crate::keyboard::keybinding_file_path()
            .display()
            .to_string()
    );
}

#[test]
fn modify_spinner_verbs_bundled_skill_is_schema_independent() {
    assert!(matches!(
        activation_for_bundled_skill("modify-spinner-verbs", Path::new("/missing/resources")),
        BundledSkillActivation::Always
    ));
}

#[test]
fn modify_spinner_verbs_bundled_skill_has_routing_metadata() {
    let skill_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("resources/bundled/skills/modify-spinner-verbs/SKILL.md");
    let skill = parse_bundled_skill(&skill_path).unwrap();

    assert_eq!(skill.name, "modify-spinner-verbs");
    assert!(skill.description.contains("spinner verbs"));
    assert!(skill.description.contains("Medieval"));
    assert!(skill.content.contains("custom_warping_verbs"));
    assert!(skill.content.contains("Interrogating the lesser lords"));
}

// ============================================================================
// Tests for best_supported_provider
// ============================================================================

/// Helper: creates a ParsedSkill under a given provider directory.
fn make_skill(name: &str, provider_dir: &str) -> ParsedSkill {
    let path = PathBuf::from(format!("/repo/{provider_dir}/skills/{name}/SKILL.md"));
    ParsedSkill {
        name: name.to_string(),
        description: format!("{name} skill"),
        path,
        content: format!("# {name}"),
        line_range: None,
        provider: get_provider_for_path(&PathBuf::from(format!(
            "/repo/{provider_dir}/skills/{name}/SKILL.md"
        )))
        .unwrap_or(SkillProvider::Warp),
        scope: SkillScope::Project,
    }
}

#[test]
fn best_supported_provider_fast_path_returns_deduped_provider() {
    // When the deduped provider is already in the supported set, return it immediately.
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let handle = app.add_singleton_model(SkillManager::new);

        let claude_skill = make_skill("deploy", ".claude");
        handle.update(&mut app, |manager, _| {
            manager.add_skill_for_testing(claude_skill.clone());
        });

        let descriptor = SkillDescriptor::from(claude_skill);
        let result = handle.read(&app, |manager, _| {
            manager.best_supported_provider(&descriptor, &[SkillProvider::Claude])
        });
        assert_eq!(result, SkillProvider::Claude);
    });
}

#[test]
fn best_supported_provider_remaps_to_supported_provider() {
    // Skill exists under both .agents and .claude. Dedup picked Agents (higher priority).
    // When supported set is [Claude], should re-map to Claude.
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let handle = app.add_singleton_model(SkillManager::new);

        let agents_skill = make_skill("deploy", ".agents");
        let claude_skill = make_skill("deploy", ".claude");
        handle.update(&mut app, |manager, _| {
            manager.add_skill_for_testing(agents_skill.clone());
            manager.add_skill_for_testing(claude_skill.clone());
        });

        // Descriptor has provider = Agents (the dedup winner).
        let descriptor = SkillDescriptor::from(agents_skill);
        assert_eq!(descriptor.provider, SkillProvider::Agents);

        let result = handle.read(&app, |manager, _| {
            manager.best_supported_provider(&descriptor, &[SkillProvider::Claude])
        });
        assert_eq!(result, SkillProvider::Claude);
    });
}

#[test]
fn best_supported_provider_falls_back_when_no_match() {
    // Skill only exists under .agents, but the supported set is [Claude].
    // Should fall back to the original deduped provider (Agents).
    App::test((), |mut app| async move {
        app.add_singleton_model(DirectoryWatcher::new);
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
        app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
        let handle = app.add_singleton_model(SkillManager::new);

        let agents_skill = make_skill("deploy", ".agents");
        handle.update(&mut app, |manager, _| {
            manager.add_skill_for_testing(agents_skill.clone());
        });

        let descriptor = SkillDescriptor::from(agents_skill);
        let result = handle.read(&app, |manager, _| {
            manager.best_supported_provider(&descriptor, &[SkillProvider::Claude])
        });
        // No .claude path exists, so falls back to the deduped provider.
        assert_eq!(result, SkillProvider::Agents);
    });
}
