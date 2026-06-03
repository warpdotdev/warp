use std::fs;

use super::CodexPluginManager;
use crate::features::FeatureFlag;
use crate::terminal::cli_agent_sessions::plugin_manager::CliAgentPluginManager;

#[test]
fn can_auto_install_is_true() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    assert!(CodexPluginManager::new(None, None, None).can_auto_install());
}

#[test]
fn can_auto_install_is_false_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    assert!(!CodexPluginManager::new(None, None, None).can_auto_install());
}

#[test]
fn install_instructions_are_native_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let instructions = CodexPluginManager::new(None, None, None).install_instructions();
    assert_eq!(instructions.title, "Enable Warp Notifications for Codex");
    assert_eq!(
        instructions.steps[1].command,
        "[tui]\nnotification_condition = \"always\""
    );
}

#[test]
fn supports_update() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    assert!(CodexPluginManager::new(None, None, None).supports_update());
}

#[test]
fn does_not_support_update_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    assert!(!CodexPluginManager::new(None, None, None).supports_update());
}

#[test]
fn minimum_version() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    assert_eq!(
        CodexPluginManager::new(None, None, None).minimum_plugin_version(),
        "0.4.0"
    );
}

#[test]
fn minimum_version_is_zero_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    assert_eq!(
        CodexPluginManager::new(None, None, None).minimum_plugin_version(),
        "0.0.0"
    );
}

#[test]
fn install_instructions_has_steps() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let instructions = CodexPluginManager::new(None, None, None).install_instructions();
    assert_eq!(
        instructions.steps[0].command,
        "codex plugin marketplace add warpdotdev/codex-warp"
    );
    assert_eq!(
        instructions.steps[1].command,
        "codex plugin add warp@codex-warp"
    );
    assert!(!instructions.steps.is_empty());
    assert!(!instructions.title.is_empty());
}

#[test]
fn update_instructions_has_steps() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let instructions = CodexPluginManager::new(None, None, None).update_instructions();
    assert_eq!(
        instructions.steps[0].command,
        "codex plugin marketplace upgrade codex-warp"
    );
    assert_eq!(
        instructions.steps[1].command,
        "codex plugin add warp@codex-warp"
    );
    assert!(!instructions.steps.is_empty());
    assert!(!instructions.title.is_empty());
}

#[test]
fn update_instructions_are_empty_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let instructions = CodexPluginManager::new(None, None, None).update_instructions();
    assert!(instructions.steps.is_empty());
    assert!(instructions.title.is_empty());
}

#[test]
fn installed_when_config_enabled() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.toml"),
        "[plugins.\"warp@codex-warp\"]\nenabled = true\n",
    )
    .unwrap();

    assert!(super::check_installed(dir.path()));
}

#[test]
fn not_installed_when_config_disabled() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("config.toml"),
        "[plugins.\"warp@codex-warp\"]\nenabled = false\n",
    )
    .unwrap();

    assert!(!super::check_installed(dir.path()));
}

#[test]
fn not_installed_when_config_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!super::check_installed(dir.path()));
}

#[test]
fn not_installed_when_config_invalid() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("config.toml"), "not toml").unwrap();

    assert!(!super::check_installed(dir.path()));
}

#[test]
fn installed_version_returns_latest_manifest_version() {
    let dir = tempfile::tempdir().unwrap();
    write_manifest(dir.path(), "0.9.0");
    write_manifest(dir.path(), "1.5.0");
    write_manifest(dir.path(), "1.2.0");

    assert_eq!(
        super::installed_version(dir.path()).as_deref(),
        Some("1.5.0")
    );
}

#[test]
fn installed_version_returns_none_when_cache_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(super::installed_version(dir.path()), None);
}

#[test]
fn installed_version_returns_none_when_manifest_has_no_version() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_dir = dir
        .path()
        .join("plugins/cache/codex-warp/warp/1.0.0/.codex-plugin");
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(manifest_dir.join("plugin.json"), "{\"name\":\"warp\"}").unwrap();

    assert_eq!(super::installed_version(dir.path()), None);
}

#[test]
#[serial_test::serial]
fn is_not_installed_via_trait_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let dir = tempfile::tempdir().unwrap();
    write_enabled_config(dir.path());

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).is_installed();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn is_installed_via_trait_with_codex_home_env() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_enabled_config(dir.path());

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).is_installed();
    std::env::remove_var("CODEX_HOME");

    assert!(result);
}

#[test]
#[serial_test::serial]
fn does_not_need_update_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let dir = tempfile::tempdir().unwrap();
    write_enabled_config(dir.path());
    write_manifest(dir.path(), "0.2.0");

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn needs_update_via_trait_with_codex_home_env() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_enabled_config(dir.path());
    write_manifest(dir.path(), "0.2.0");

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(result);
}

#[test]
#[serial_test::serial]
fn does_not_need_update_via_trait_when_version_current() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_enabled_config(dir.path());
    write_manifest(dir.path(), "0.4.0");

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn needs_update_via_trait_when_installed_without_manifest() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_enabled_config(dir.path());

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(result);
}

fn write_enabled_config(dir: &std::path::Path) {
    fs::write(
        dir.join("config.toml"),
        "[plugins.\"warp@codex-warp\"]\nenabled = true\n",
    )
    .unwrap();
}

fn write_manifest(dir: &std::path::Path, version: &str) {
    let manifest_dir = dir
        .join("plugins")
        .join("cache")
        .join("codex-warp")
        .join("warp")
        .join(version)
        .join(".codex-plugin");
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(
        manifest_dir.join("plugin.json"),
        serde_json::json!({
            "name": "warp",
            "version": version
        })
        .to_string(),
    )
    .unwrap();
}
