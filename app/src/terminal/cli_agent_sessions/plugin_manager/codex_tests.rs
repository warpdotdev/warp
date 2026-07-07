use std::fs;
use std::path::Path;

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
fn install_instructions_has_marketplace_and_plugin_add_steps() {
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
    assert_eq!(instructions.steps.len(), 2);
    assert!(!instructions.title.is_empty());
}

#[test]
fn update_instructions_has_marketplace_and_plugin_add_steps() {
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
    assert_eq!(instructions.steps.len(), 2);
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
fn installed_when_plugin_enabled_in_config() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);

    assert!(super::check_installed(dir.path()));
}

#[test]
fn not_installed_when_plugin_disabled_in_config() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, false);

    assert!(!super::check_installed(dir.path()));
}

#[test]
fn not_installed_when_only_marketplace_present() {
    // Marketplace cloned but the plugin was never enabled.
    let dir = tempfile::tempdir().unwrap();
    write_marketplace_config(dir.path(), "git");

    assert!(!super::check_installed(dir.path()));
}

#[test]
fn platform_plugin_installed_when_enabled_in_config() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLATFORM_PLUGIN_KEY, true);

    assert!(super::check_platform_plugin_installed(dir.path()));
}

#[test]
fn platform_plugin_not_installed_when_disabled_in_config() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLATFORM_PLUGIN_KEY, false);

    assert!(!super::check_platform_plugin_installed(dir.path()));
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
fn installed_version_reads_cache_manifest_version() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.4.0");

    assert_eq!(
        super::installed_version(dir.path()).as_deref(),
        Some("0.4.0")
    );
}

#[test]
fn installed_platform_plugin_version_reads_cache_manifest_version() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest(dir.path(), super::PLATFORM_PLUGIN_NAME, "0.4.0");

    assert_eq!(
        super::installed_platform_plugin_version(dir.path()).as_deref(),
        Some("0.4.0")
    );
}

#[test]
fn installed_version_picks_latest_cached() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.3.0");
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.5.0");

    assert_eq!(
        super::installed_version(dir.path()).as_deref(),
        Some("0.5.0")
    );
}

#[test]
fn installed_version_returns_none_when_cache_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(super::installed_version(dir.path()), None);
}

#[test]
fn installed_version_returns_none_when_cache_manifest_has_no_version() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest_without_version(dir.path(), super::PLUGIN_NAME, "0.4.0");

    assert_eq!(super::installed_version(dir.path()), None);
}

#[test]
fn platform_plugin_version_is_current_when_cache_current() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest(dir.path(), super::PLATFORM_PLUGIN_NAME, "0.4.0");

    assert!(super::platform_plugin_version_is_current(dir.path()));
}

#[test]
fn platform_plugin_version_is_not_current_when_cache_outdated() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest(dir.path(), super::PLATFORM_PLUGIN_NAME, "0.2.0");

    assert!(!super::platform_plugin_version_is_current(dir.path()));
}

#[test]
fn needs_update_true_when_enabled_and_version_outdated() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.2.0");

    assert!(super::plugin_needs_update(
        dir.path(),
        super::PLUGIN_NAME,
        super::PLUGIN_KEY,
        "0.4.0"
    ));
}

#[test]
fn needs_update_false_when_enabled_and_version_current() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.4.0");

    assert!(!super::plugin_needs_update(
        dir.path(),
        super::PLUGIN_NAME,
        super::PLUGIN_KEY,
        "0.4.0"
    ));
}

#[test]
fn needs_update_false_when_not_enabled() {
    let dir = tempfile::tempdir().unwrap();
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.2.0");

    assert!(!super::plugin_needs_update(
        dir.path(),
        super::PLUGIN_NAME,
        super::PLUGIN_KEY,
        "0.4.0"
    ));
}

#[test]
fn needs_update_true_when_enabled_without_cached_version() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);

    assert!(super::plugin_needs_update(
        dir.path(),
        super::PLUGIN_NAME,
        super::PLUGIN_KEY,
        "0.4.0"
    ));
}

#[test]
fn platform_plugin_needs_update_true_when_enabled_and_outdated() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLATFORM_PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLATFORM_PLUGIN_NAME, "0.2.0");

    assert!(super::plugin_needs_update(
        dir.path(),
        super::PLATFORM_PLUGIN_NAME,
        super::PLATFORM_PLUGIN_KEY,
        super::MINIMUM_PLATFORM_PLUGIN_VERSION
    ));
}

#[test]
fn platform_plugin_needs_update_false_when_current() {
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLATFORM_PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLATFORM_PLUGIN_NAME, "0.4.0");

    assert!(!super::plugin_needs_update(
        dir.path(),
        super::PLATFORM_PLUGIN_NAME,
        super::PLATFORM_PLUGIN_KEY,
        super::MINIMUM_PLATFORM_PLUGIN_VERSION
    ));
}

#[test]
#[serial_test::serial]
fn is_not_installed_via_trait_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);

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
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).is_installed();
    std::env::remove_var("CODEX_HOME");

    assert!(result);
}

#[test]
#[serial_test::serial]
fn is_platform_plugin_installed_via_trait_with_codex_home_env() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLATFORM_PLUGIN_KEY, true);

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).is_platform_plugin_installed();
    std::env::remove_var("CODEX_HOME");

    assert!(result);
}

#[test]
#[serial_test::serial]
fn is_platform_plugin_not_installed_via_trait_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLATFORM_PLUGIN_KEY, true);

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).is_platform_plugin_installed();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn needs_update_via_trait_with_codex_home_env() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.2.0");

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
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.4.0");

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn does_not_need_update_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let dir = tempfile::tempdir().unwrap();
    write_plugin_config(dir.path(), super::PLUGIN_KEY, true);
    write_cache_manifest(dir.path(), super::PLUGIN_NAME, "0.2.0");

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn does_not_need_update_when_not_enabled() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn does_not_need_update_for_non_git_marketplace_override() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    write_marketplace_config(dir.path(), "directory");

    std::env::set_var("CODEX_HOME", dir.path());
    let result = CodexPluginManager::new(None, None, None).needs_update();
    let has_override = CodexPluginManager::new(None, None, None).has_local_marketplace_override();
    std::env::remove_var("CODEX_HOME");

    assert!(!result);
    assert!(has_override);
}

fn write_plugin_config(dir: &Path, plugin_key: &str, enabled: bool) {
    fs::write(
        dir.join("config.toml"),
        format!("[plugins.\"{plugin_key}\"]\nenabled = {enabled}\n"),
    )
    .unwrap();
}

fn write_marketplace_config(dir: &Path, source_type: &str) {
    fs::write(
        dir.join("config.toml"),
        format!(
            "[marketplaces.codex-warp]\nsource_type = \"{source_type}\"\nsource = \"/tmp/codex-warp\"\n"
        ),
    )
    .unwrap();
}

fn write_cache_manifest(dir: &Path, plugin_name: &str, version: &str) {
    write_cache_manifest_json(
        dir,
        plugin_name,
        version,
        serde_json::json!({ "name": plugin_name, "version": version }),
    );
}

fn write_cache_manifest_without_version(dir: &Path, plugin_name: &str, version_dir: &str) {
    write_cache_manifest_json(
        dir,
        plugin_name,
        version_dir,
        serde_json::json!({ "name": plugin_name }),
    );
}

fn write_cache_manifest_json(
    dir: &Path,
    plugin_name: &str,
    version_dir: &str,
    manifest: serde_json::Value,
) {
    let manifest_dir = dir
        .join("plugins")
        .join("cache")
        .join("codex-warp")
        .join(plugin_name)
        .join(version_dir)
        .join(".codex-plugin");
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(manifest_dir.join("plugin.json"), manifest.to_string()).unwrap();
}
