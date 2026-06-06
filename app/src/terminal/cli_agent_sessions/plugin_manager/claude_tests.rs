use std::fs;

use super::{
    check_installed, check_platform_plugin_installed, claude_code_marketplace_has_local_override,
    installed_platform_plugin_version, installed_version, ClaudeCodePluginManager,
    CliAgentPluginManager,
};

#[test]
fn installed_when_plugin_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(check_installed(dir.path()));
}

#[test]
fn local_marketplace_override_detects_directory_source() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({
        "extraKnownMarketplaces": {
            "claude-code-warp": {
                "source": {
                    "path": "/Users/example/Developer/claude-code-warp-internal",
                    "source": "directory"
                }
            }
        }
    });
    fs::write(
        dir.path().join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    assert!(claude_code_marketplace_has_local_override(dir.path()));
}

#[test]
fn local_marketplace_override_ignores_repo_source() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({
        "extraKnownMarketplaces": {
            "claude-code-warp": {
                "source": "warpdotdev/claude-code-warp"
            }
        }
    });
    fs::write(
        dir.path().join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    assert!(!claude_code_marketplace_has_local_override(dir.path()));
}

#[test]
#[serial_test::serial]
fn local_marketplace_override_via_trait_uses_claude_home() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({
        "extraKnownMarketplaces": {
            "claude-code-warp": {
                "source": {
                    "path": "../claude-code-warp-internal",
                    "source": "directory"
                }
            }
        }
    });
    fs::write(
        dir.path().join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    std::env::set_var("CLAUDE_HOME", dir.path());
    let result = ClaudeCodePluginManager::new(None, None, None).has_local_marketplace_override();
    std::env::remove_var("CLAUDE_HOME");

    assert!(result);
}

#[test]
fn installed_platform_plugin_version_returns_version_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": "1.1.3"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(
        installed_platform_plugin_version(dir.path()).as_deref(),
        Some("1.1.3")
    );
}

#[test]
fn platform_plugin_installed_when_platform_plugin_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": "1.1.3"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(check_platform_plugin_installed(dir.path()));
}

#[test]
#[serial_test::serial]
fn platform_plugin_needs_update_via_trait_when_version_below_minimum() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": "1.1.2"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    std::env::set_var("CLAUDE_HOME", dir.path());
    let result = ClaudeCodePluginManager::new(None, None, None).platform_plugin_needs_update();
    std::env::remove_var("CLAUDE_HOME");

    assert!(result);
}

#[test]
#[serial_test::serial]
fn platform_plugin_does_not_need_update_via_trait_when_current() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": "1.1.3"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    std::env::set_var("CLAUDE_HOME", dir.path());
    let result = ClaudeCodePluginManager::new(None, None, None).platform_plugin_needs_update();
    std::env::remove_var("CLAUDE_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn platform_plugin_needs_update_via_trait_when_installed_without_version() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"scope": "user"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    std::env::set_var("CLAUDE_HOME", dir.path());
    let result = ClaudeCodePluginManager::new(None, None, None).platform_plugin_needs_update();
    std::env::remove_var("CLAUDE_HOME");

    assert!(result);
}

#[test]
fn platform_plugin_not_installed_when_only_notification_plugin_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_platform_plugin_installed(dir.path()));
}

#[test]
fn not_installed_when_plugin_key_absent() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "some-other-plugin": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_plugin_array_empty() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": []
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_json_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();
    fs::write(plugins_dir.join("installed_plugins.json"), "not json").unwrap();

    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_plugins_key_missing() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({"other_key": "value"});
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_installed(dir.path()));
}

/// Tests `ClaudeCodePluginManager::is_installed` end-to-end by pointing
/// `CLAUDE_HOME` at a temp directory with a valid installed_plugins.json.
#[test]
#[serial_test::serial]
fn is_installed_via_trait_with_claude_home_env() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    std::env::set_var("CLAUDE_HOME", dir.path());
    let result = ClaudeCodePluginManager::new(None, None, None).is_installed();
    std::env::remove_var("CLAUDE_HOME");

    assert!(result);
}

#[test]
#[serial_test::serial]
fn not_installed_via_trait_when_claude_home_empty() {
    let dir = tempfile::tempdir().unwrap();

    std::env::set_var("CLAUDE_HOME", dir.path());
    let result = ClaudeCodePluginManager::new(None, None, None).is_installed();
    std::env::remove_var("CLAUDE_HOME");

    assert!(!result);
}

#[test]
fn installed_version_returns_version_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.5.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(installed_version(dir.path()).as_deref(), Some("1.5.0"));
}

#[test]
fn installed_version_returns_none_when_no_version_field() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"scope": "user"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(installed_version(dir.path()), None);
}

#[test]
fn installed_version_returns_none_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(installed_version(dir.path()), None);
}
