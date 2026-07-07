use std::collections::HashMap;
use std::path::Path;

use super::*;
use crate::launch_configs::launch_config::{PaneTemplateType, SplitDirection};

const WORKTREE_TOML: &str = r#"
name = "New Worktree"
title = "{{worktree_branch_name}}"

[[panes]]
id = "main"
type = "terminal"
directory = "{{repo}}"
commands = [
  "git worktree add -b {{worktree_branch_name}} $HOME/.warp/worktrees/$(basename {{repo}})/{{worktree_branch_name}} {{branch}}",
  "cd $HOME/.warp/worktrees/$(basename {{repo}})/{{worktree_branch_name}}",
]

[params.repo]
type = "repo"
description = "Absolute path to repository"

[params.branch]
type = "branch"
description = "Base branch to branch from"

[params.worktree_branch_name]
type = "text"
description = "New worktree branch name"
default = "my-feature-branch"
"#;

fn generated_worktree_path_string(repo: &str, worktree_name: &str) -> String {
    generated_worktree_path(Path::new(repo), worktree_name)
        .display()
        .to_string()
}

fn build_test_tab_config_toml(name: &str, commands: Vec<String>) -> String {
    let config = TabConfig {
        name: name.to_string(),
        title: None,
        color: None,
        panes: vec![TabConfigPaneNode {
            id: "main".to_string(),
            pane_type: Some(TabConfigPaneType::Terminal),
            split: None,
            children: None,
            is_focused: None,
            directory: Some("/Users/me/repo".to_string()),
            commands: Some(commands),
            shell: None,
        }],
        params: HashMap::new(),
        source_path: None,
    };

    toml::to_string_pretty(&config).expect("Test config should serialize")
}

#[test]
fn test_generated_worktree_path_uses_repo_name_directory() {
    let repo_dir = generated_worktree_repo_dir(Path::new("/Users/me/backend"));

    assert_eq!(
        repo_dir,
        warp_core::paths::data_dir()
            .join("worktrees")
            .join("backend")
    );
    assert_eq!(
        generated_worktree_path(Path::new("/Users/me/backend"), "mesa-coyote"),
        repo_dir.join("mesa-coyote")
    );
}

#[test]
fn test_parse_worktree_toml() {
    let config: TabConfig = toml::from_str(WORKTREE_TOML).expect("Should parse worktree TOML");

    assert_eq!(config.name, "New Worktree");
    assert_eq!(config.title.as_deref(), Some("{{worktree_branch_name}}"));
    assert_eq!(config.panes.len(), 1);
    assert_eq!(config.panes[0].id, "main");
    assert_eq!(config.panes[0].directory.as_deref(), Some("{{repo}}"));
    assert_eq!(
        config.panes[0].commands.as_deref().unwrap(),
        &[
            "git worktree add -b {{worktree_branch_name}} $HOME/.warp/worktrees/$(basename {{repo}})/{{worktree_branch_name}} {{branch}}",
            "cd $HOME/.warp/worktrees/$(basename {{repo}})/{{worktree_branch_name}}"
        ]
    );
    assert_eq!(config.params.len(), 3);
    assert_eq!(config.params["repo"].param_type, TabConfigParamType::Repo);
    assert_eq!(
        config.params["branch"].param_type,
        TabConfigParamType::Branch
    );
    assert_eq!(
        config.params["worktree_branch_name"].param_type,
        TabConfigParamType::Text
    );
    assert!(config.params["repo"].default.is_none());
    assert!(config.params["branch"].default.is_none());
    assert_eq!(
        config.params["worktree_branch_name"].default.as_deref(),
        Some("my-feature-branch")
    );
}

#[test]
fn test_parse_minimal_toml() {
    let toml = r#"name = "Plain Tab""#;
    let config: TabConfig = toml::from_str(toml).expect("Should parse minimal TOML");

    assert_eq!(config.name, "Plain Tab");
    assert!(config.title.is_none());
    assert!(config.panes.is_empty());
    assert!(config.params.is_empty());
}

#[test]
fn test_parse_toml_with_on_close_fails() {
    let toml = r#"
name = "Legacy Tab"

[[panes]]
id = "main"
type = "terminal"

[on_close]
commands = ["echo cleanup"]
"#;

    let error = toml::from_str::<TabConfig>(toml).expect_err("on_close should be rejected");
    assert!(error.to_string().contains("unknown field `on_close`"));
}

#[test]
fn test_default_param_values() {
    let config: TabConfig = toml::from_str(WORKTREE_TOML).unwrap();
    let defaults = config.default_param_values();
    assert_eq!(defaults["branch"], "");
    assert_eq!(defaults["repo"], "");
    assert_eq!(defaults["worktree_branch_name"], "my-feature-branch");
}

#[test]
fn test_is_worktree() {
    let worktree_config: TabConfig = toml::from_str(WORKTREE_TOML).unwrap();
    assert!(worktree_config.is_worktree());

    let plain_config: TabConfig = toml::from_str(
        r#"
name = "Plain Tab"

[[panes]]
id = "main"
type = "terminal"
commands = ["pwd", "ls"]
"#,
    )
    .unwrap();
    assert!(!plain_config.is_worktree());
}

#[test]
fn test_render_tab_config_substitutes_values() {
    let config: TabConfig = toml::from_str(WORKTREE_TOML).unwrap();

    let mut params = HashMap::new();
    params.insert("repo".to_string(), "/Users/me/repo".to_string());
    params.insert("branch".to_string(), "main".to_string());
    params.insert("worktree_branch_name".to_string(), "my-feature".to_string());

    let (title, pane_template) = render_tab_config(&config, &params, None);

    assert_eq!(title.as_deref(), Some("my-feature"));

    if let crate::launch_configs::launch_config::PaneTemplateType::PaneTemplate {
        cwd,
        commands,
        ..
    } = pane_template
    {
        assert_eq!(cwd, std::path::PathBuf::from("/Users/me/repo"));
        // Commands should have shell-quoted values.
        assert_eq!(
            commands[0].exec,
            "git worktree add -b my-feature $HOME/.warp/worktrees/$(basename /Users/me/repo)/my-feature main"
        );
        assert_eq!(
            commands[1].exec,
            "cd $HOME/.warp/worktrees/$(basename /Users/me/repo)/my-feature"
        );
    } else {
        panic!("Expected PaneTemplate variant");
    }
}

#[test]
fn test_render_tab_config_shell_quotes_commands_with_spaces() {
    let config: TabConfig = toml::from_str(WORKTREE_TOML).unwrap();

    let mut params = HashMap::new();
    params.insert("repo".to_string(), "/Users/me/my project".to_string());
    params.insert("branch".to_string(), "release-candidate".to_string());
    params.insert("worktree_branch_name".to_string(), "my feature".to_string());

    let (_, pane_template) = render_tab_config(&config, &params, None);

    if let crate::launch_configs::launch_config::PaneTemplateType::PaneTemplate {
        commands, ..
    } = pane_template
    {
        // Values with spaces should be quoted in commands.
        assert!(
            commands[0].exec.contains("'my feature'"),
            "Expected shell-quoted worktree branch name in command: {}",
            commands[0].exec
        );
    } else {
        panic!("Expected PaneTemplate variant");
    }
}

#[test]
fn test_render_tab_config_multi_pane() {
    let toml = r#"
name = "Split Tab"

[[panes]]
id = "root"
split = "horizontal"
children = ["left", "right"]

[[panes]]
id = "left"
type = "terminal"
directory = "{{repo}}"
commands = [
  "git worktree add -b {{worktree_branch_name}} $HOME/.warp/worktrees/$(basename {{repo}})/{{worktree_branch_name}} {{branch}}",
  "cd $HOME/.warp/worktrees/$(basename {{repo}})/{{worktree_branch_name}}",
]

[[panes]]
id = "right"
type = "terminal"
directory = "{{repo}}"
commands = ["nvim"]

[params.repo]
type = "repo"
description = "Repo path"

[params.branch]
type = "branch"
description = "Base branch to branch from"

[params.worktree_branch_name]
type = "text"
description = "New worktree branch name"
"#;

    let config: TabConfig = toml::from_str(toml).expect("Should parse multi-pane TOML");
    let mut params = HashMap::new();
    params.insert("repo".to_string(), "/Users/me/repo".to_string());
    params.insert("branch".to_string(), "main".to_string());
    params.insert("worktree_branch_name".to_string(), "my-feature".to_string());

    let (title, pane_template) = render_tab_config(&config, &params, None);
    assert!(title.is_none());

    if let PaneTemplateType::PaneBranchTemplate {
        split_direction,
        panes,
    } = pane_template
    {
        assert_eq!(split_direction, SplitDirection::Horizontal);
        assert_eq!(panes.len(), 2);

        // First pane should be focused and have two commands.
        if let PaneTemplateType::PaneTemplate {
            cwd,
            commands,
            is_focused,
            ..
        } = &panes[0]
        {
            assert_eq!(*cwd, std::path::PathBuf::from("/Users/me/repo"));
            assert_eq!(commands.len(), 2);
            assert_eq!(
                commands[0].exec,
                "git worktree add -b my-feature $HOME/.warp/worktrees/$(basename /Users/me/repo)/my-feature main"
            );
            assert_eq!(
                commands[1].exec,
                "cd $HOME/.warp/worktrees/$(basename /Users/me/repo)/my-feature"
            );
            assert_eq!(*is_focused, Some(true));
        } else {
            panic!("Expected PaneTemplate for first child");
        }

        // Second pane should not be focused.
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = &panes[1] {
            assert_eq!(*is_focused, Some(false));
        } else {
            panic!("Expected PaneTemplate for second child");
        }
    } else {
        panic!("Expected PaneBranchTemplate");
    }
}

#[test]
fn test_render_tab_config_cwd_is_not_quoted() {
    let config: TabConfig = toml::from_str(WORKTREE_TOML).unwrap();

    let mut params = HashMap::new();
    params.insert("repo".to_string(), "/Users/me/my project".to_string());
    params.insert("branch".to_string(), "main".to_string());
    params.insert("worktree_branch_name".to_string(), "my-feature".to_string());

    let (_, pane_template) = render_tab_config(&config, &params, None);

    if let crate::launch_configs::launch_config::PaneTemplateType::PaneTemplate { cwd, .. } =
        pane_template
    {
        // cwd should be unquoted (raw path).
        assert_eq!(cwd, std::path::PathBuf::from("/Users/me/my project"));
    } else {
        panic!("Expected PaneTemplate variant");
    }
}

// ── Flat pane format tests ──────────────────────────────────────────

#[test]
fn test_flat_single_pane() {
    let toml_str = r#"
name = "Single"

[[panes]]
id = "main"
type = "terminal"
directory = "~/code/project"
commands = ["npm run dev"]
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse flat single pane");
    assert_eq!(config.panes.len(), 1);
    assert_eq!(config.panes[0].id, "main");

    let (_, template) = render_tab_config(&config, &HashMap::new(), None);
    if let PaneTemplateType::PaneTemplate {
        commands,
        is_focused,
        ..
    } = template
    {
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].exec, "npm run dev");
        // Single pane should be auto-focused.
        assert_eq!(is_focused, Some(true));
    } else {
        panic!("Expected PaneTemplate for single flat pane");
    }
}

#[test]
fn test_flat_two_pane_split() {
    let toml_str = r#"
name = "Split"

[[panes]]
id = "root"
split = "horizontal"
children = ["left", "right"]

[[panes]]
id = "left"
type = "terminal"
directory = "~/code/frontend"
commands = ["npm start"]

[[panes]]
id = "right"
type = "terminal"
directory = "~/code/backend"
commands = ["cargo run"]
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse flat split");
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);

    if let PaneTemplateType::PaneBranchTemplate {
        split_direction,
        panes,
    } = template
    {
        assert_eq!(split_direction, SplitDirection::Horizontal);
        assert_eq!(panes.len(), 2);

        // First leaf should be auto-focused.
        if let PaneTemplateType::PaneTemplate {
            commands,
            is_focused,
            ..
        } = &panes[0]
        {
            assert_eq!(commands[0].exec, "npm start");
            assert_eq!(*is_focused, Some(true));
        } else {
            panic!("Expected PaneTemplate for left child");
        }

        // Second leaf should not be focused.
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = &panes[1] {
            assert_eq!(*is_focused, Some(false));
        } else {
            panic!("Expected PaneTemplate for right child");
        }
    } else {
        panic!("Expected PaneBranchTemplate");
    }
}

#[test]
fn test_flat_2x2_grid() {
    let toml_str = r#"
name = "Grid"

[[panes]]
id = "root"
split = "horizontal"
children = ["left_col", "right_col"]

[[panes]]
id = "left_col"
split = "vertical"
children = ["tl", "bl"]

[[panes]]
id = "tl"
type = "terminal"
directory = "~/a"

[[panes]]
id = "bl"
type = "terminal"
directory = "~/b"

[[panes]]
id = "right_col"
split = "vertical"
children = ["tr", "br"]

[[panes]]
id = "tr"
type = "terminal"
directory = "~/c"

[[panes]]
id = "br"
type = "terminal"
directory = "~/d"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse flat 2x2");
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);

    // Root should be a horizontal split.
    let PaneTemplateType::PaneBranchTemplate {
        split_direction: root_dir,
        panes: root_children,
    } = template
    else {
        panic!("Expected root PaneBranchTemplate");
    };
    assert_eq!(root_dir, SplitDirection::Horizontal);
    assert_eq!(root_children.len(), 2);

    // Left column should be a vertical split with 2 children.
    let PaneTemplateType::PaneBranchTemplate {
        split_direction: left_dir,
        panes: left_children,
    } = &root_children[0]
    else {
        panic!("Expected left_col PaneBranchTemplate");
    };
    assert_eq!(*left_dir, SplitDirection::Vertical);
    assert_eq!(left_children.len(), 2);

    // Top-left should be focused (first leaf in tree, auto-focus).
    if let PaneTemplateType::PaneTemplate { is_focused, .. } = &left_children[0] {
        assert_eq!(*is_focused, Some(true));
    } else {
        panic!("Expected PaneTemplate for tl");
    }

    // Bottom-left should not be focused.
    if let PaneTemplateType::PaneTemplate { is_focused, .. } = &left_children[1] {
        assert_eq!(*is_focused, Some(false), "Expected bl to not be focused");
    } else {
        panic!("Expected PaneTemplate for bl");
    }

    // Right column should be a vertical split with 2 children.
    let PaneTemplateType::PaneBranchTemplate {
        panes: right_children,
        ..
    } = &root_children[1]
    else {
        panic!("Expected right_col PaneBranchTemplate");
    };
    for (label, pane) in [("tr", &right_children[0]), ("br", &right_children[1])] {
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = pane {
            assert_eq!(
                *is_focused,
                Some(false),
                "Expected {label} to not be focused"
            );
        } else {
            panic!("Expected PaneTemplate for {label}");
        }
    }
}

#[test]
fn test_flat_explicit_focus() {
    let toml_str = r#"
name = "Focus Test"

[[panes]]
id = "root"
split = "horizontal"
children = ["left", "right"]

[[panes]]
id = "left"
type = "terminal"
directory = "~/a"

[[panes]]
id = "right"
type = "terminal"
directory = "~/b"
is_focused = true
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse");
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);

    if let PaneTemplateType::PaneBranchTemplate { panes, .. } = template {
        // Left should NOT be focused (explicit focus is on right).
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = &panes[0] {
            assert_eq!(*is_focused, Some(false));
        }
        // Right should be focused.
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = &panes[1] {
            assert_eq!(*is_focused, Some(true));
        }
    } else {
        panic!("Expected PaneBranchTemplate");
    }
}

#[test]
fn test_flat_auto_focus_first_leaf() {
    let toml_str = r#"
name = "Auto Focus"

[[panes]]
id = "root"
split = "horizontal"
children = ["left", "right"]

[[panes]]
id = "left"
type = "terminal"
directory = "~/a"

[[panes]]
id = "right"
type = "terminal"
directory = "~/b"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse");
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);

    if let PaneTemplateType::PaneBranchTemplate { panes, .. } = template {
        // First leaf (left) should be auto-focused.
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = &panes[0] {
            assert_eq!(*is_focused, Some(true));
        }
        // Second leaf should not be focused.
        if let PaneTemplateType::PaneTemplate { is_focused, .. } = &panes[1] {
            assert_eq!(*is_focused, Some(false));
        }
    } else {
        panic!("Expected PaneBranchTemplate");
    }
}

#[test]
fn test_flat_color_deserialized() {
    let toml_str = r#"
name = "Colored Tab"
color = "blue"

[[panes]]
id = "main"
type = "terminal"
directory = "~/code"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse with color");
    assert_eq!(
        config.color,
        Some(crate::themes::theme::AnsiColorIdentifier::Blue)
    );
}

#[test]
fn test_flat_missing_child_ref_falls_back() {
    let toml_str = r#"
name = "Bad Ref"

[[panes]]
id = "root"
split = "horizontal"
children = ["left", "nonexistent"]

[[panes]]
id = "left"
type = "terminal"
directory = "~/a"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse");
    // render_tab_config should fall back to a single empty terminal.
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);
    if let PaneTemplateType::PaneTemplate {
        commands,
        is_focused,
        ..
    } = template
    {
        assert!(commands.is_empty());
        assert_eq!(is_focused, Some(true));
    } else {
        panic!("Expected fallback PaneTemplate on error");
    }
}

#[test]
fn test_flat_duplicate_ids_falls_back() {
    let toml_str = r#"
name = "Dup IDs"

[[panes]]
id = "main"
type = "terminal"
directory = "~/a"

[[panes]]
id = "main"
type = "terminal"
directory = "~/b"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse");
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);
    // Should fall back because of duplicate IDs.
    if let PaneTemplateType::PaneTemplate {
        commands,
        is_focused,
        ..
    } = template
    {
        assert!(commands.is_empty());
        assert_eq!(is_focused, Some(true));
    } else {
        panic!("Expected fallback PaneTemplate on duplicate ID error");
    }
}

#[test]
fn test_flat_with_params() {
    let toml_str = r#"
name = "Param Test"
title = "{{branch}}"

[[panes]]
id = "main"
type = "terminal"
directory = "{{repo}}"
commands = ["git checkout {{branch}}"]

[params.repo]
type = "repo"
description = "Repo path"

[params.branch]
type = "branch"
description = "Branch"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse with params");
    let mut params = HashMap::new();
    params.insert("repo".to_string(), "/Users/me/code".to_string());
    params.insert("branch".to_string(), "main".to_string());

    let (title, template) = render_tab_config(&config, &params, None);
    assert_eq!(title.as_deref(), Some("main"));

    if let PaneTemplateType::PaneTemplate { cwd, commands, .. } = template {
        assert_eq!(cwd, std::path::PathBuf::from("/Users/me/code"));
        assert_eq!(commands[0].exec, "git checkout main");
    } else {
        panic!("Expected PaneTemplate");
    }
}

#[test]
fn test_worktree_template_substitution() {
    let worktree_path =
        generated_worktree_path_string("/Users/me/repo", "{{autogenerated_branch_name}}");
    let toml_str = build_test_tab_config_toml(
        "Template Worktree",
        vec![
            format!("git worktree add -b {{{{autogenerated_branch_name}}}} {worktree_path} main"),
            format!("cd {worktree_path}"),
        ],
    );
    let config: TabConfig = toml::from_str(&toml_str).expect("Should parse");
    let (_, template) = render_tab_config(&config, &HashMap::new(), Some("mesa-coyote"));

    if let PaneTemplateType::PaneTemplate { commands, .. } = template {
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0].exec,
            format!(
                "git worktree add -b mesa-coyote {} main",
                generated_worktree_path_string("/Users/me/repo", "mesa-coyote")
            )
        );
        assert_eq!(
            commands[1].exec,
            format!(
                "cd {}",
                generated_worktree_path_string("/Users/me/repo", "mesa-coyote")
            )
        );
    } else {
        panic!("Expected PaneTemplate");
    }
}

#[test]
fn test_worktree_custom_commands_with_template() {
    let worktree_path =
        generated_worktree_path_string("/Users/me/repo", "{{autogenerated_branch_name}}");
    let toml_str = build_test_tab_config_toml(
        "Custom Worktree",
        vec![
            format!("git worktree add -b {{{{autogenerated_branch_name}}}} {worktree_path} main"),
            format!("cd {worktree_path}"),
            "gt branch create".to_string(),
            "npm install".to_string(),
        ],
    );
    let config: TabConfig = toml::from_str(&toml_str).expect("Should parse");
    let (_, template) = render_tab_config(&config, &HashMap::new(), Some("mesa-coyote"));

    if let PaneTemplateType::PaneTemplate { commands, .. } = template {
        assert_eq!(commands.len(), 4);
        assert_eq!(
            commands[0].exec,
            format!(
                "git worktree add -b mesa-coyote {} main",
                generated_worktree_path_string("/Users/me/repo", "mesa-coyote")
            )
        );
        assert_eq!(
            commands[1].exec,
            format!(
                "cd {}",
                generated_worktree_path_string("/Users/me/repo", "mesa-coyote")
            )
        );
        assert_eq!(commands[2].exec, "gt branch create");
        assert_eq!(commands[3].exec, "npm install");
    } else {
        panic!("Expected PaneTemplate");
    }
}

#[test]
fn test_build_worktree_toml_autogenerate_round_trips() {
    let toml_str =
        build_worktree_config_toml("Worktree: my-project", "/Users/me/repo", "main", None);
    let config: TabConfig = toml::from_str(&toml_str).expect("Generated TOML should parse");

    assert_eq!(config.name, "Worktree: my-project");
    assert!(config.title.is_none());
    assert!(config.params.is_empty());
    assert_eq!(config.panes.len(), 1);

    let pane = &config.panes[0];
    assert!(config.uses_autogenerated_branch_name());
    assert!(pane.commands.is_some());
    assert_eq!(pane.directory.as_deref(), Some("/Users/me/repo"));

    // Verify rendering produces the correct commands.
    let (_, template) = render_tab_config(&config, &HashMap::new(), Some("obsidian-hawk"));
    if let PaneTemplateType::PaneTemplate { commands, .. } = template {
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0].exec,
            format!(
                "git worktree add -b obsidian-hawk {} main",
                generated_worktree_path_string("/Users/me/repo", "obsidian-hawk")
            )
        );
        assert_eq!(
            commands[1].exec,
            format!(
                "cd {}",
                generated_worktree_path_string("/Users/me/repo", "obsidian-hawk")
            )
        );
    } else {
        panic!("Expected PaneTemplate");
    }
}

#[test]
fn test_build_worktree_toml_manual_round_trips() {
    let toml_str = build_worktree_config_toml(
        "Worktree: my-project",
        "/Users/me/repo",
        "main",
        Some("my-feature"),
    );
    let config: TabConfig = toml::from_str(&toml_str).expect("Generated TOML should parse");

    assert_eq!(config.name, "Worktree: my-project");
    assert_eq!(config.title.as_deref(), Some("{{worktree_branch_name}}"));
    assert!(config.params.contains_key("worktree_branch_name"));
    assert_eq!(
        config.params["worktree_branch_name"].param_type,
        TabConfigParamType::Text
    );
    assert_eq!(config.panes.len(), 1);

    let pane = &config.panes[0];
    assert!(!config.uses_autogenerated_branch_name());
    assert!(pane.commands.is_some());

    // Verify rendering substitutes the branch name into commands and title.
    let mut params = HashMap::new();
    params.insert("worktree_branch_name".to_string(), "my-feature".to_string());
    let (title, template) = render_tab_config(&config, &params, None);
    assert_eq!(title.as_deref(), Some("my-feature"));

    if let PaneTemplateType::PaneTemplate { commands, .. } = template {
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0].exec,
            format!(
                "git worktree add -b my-feature {} main",
                generated_worktree_path_string("/Users/me/repo", "my-feature")
            )
        );
        assert_eq!(
            commands[1].exec,
            format!(
                "cd {}",
                generated_worktree_path_string("/Users/me/repo", "my-feature")
            )
        );
    } else {
        panic!("Expected PaneTemplate");
    }
}

#[test]
fn test_flat_split_with_fewer_than_two_children_falls_back() {
    let toml_str = r#"
name = "Bad Split"

[[panes]]
id = "root"
split = "horizontal"
children = ["only"]

[[panes]]
id = "only"
type = "terminal"
directory = "~/a"
"#;
    let config: TabConfig = toml::from_str(toml_str).expect("Should parse");
    let (_, template) = render_tab_config(&config, &HashMap::new(), None);
    // Should fall back because split needs >= 2 children.
    if let PaneTemplateType::PaneTemplate { is_focused, .. } = template {
        assert_eq!(is_focused, Some(true));
    } else {
        panic!("Expected fallback PaneTemplate");
    }
}
