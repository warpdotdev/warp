use super::{
    GrokPluginManager, HOOK_JSON_FILE, PLUGIN_SCRIPT_REL, VERSION_FILE, check_installed,
    install_plugin_files, installed_version,
};
use crate::terminal::cli_agent_sessions::plugin_manager::CliAgentPluginManager;

#[test]
fn can_auto_install_is_true() {
    assert!(GrokPluginManager.can_auto_install());
}

#[test]
fn install_instructions_has_steps() {
    let instructions = GrokPluginManager.install_instructions();
    assert!(!instructions.steps.is_empty());
    assert!(!instructions.title.is_empty());
    assert!(
        instructions
            .steps
            .iter()
            .any(|s| s.command.contains("grok") || s.description.contains("Grok"))
    );
    assert!(
        instructions
            .steps
            .iter()
            .any(|s| s.command.contains("warp-plugin"))
    );
}

#[test]
fn update_instructions_has_steps() {
    let instructions = GrokPluginManager.update_instructions();
    assert!(!instructions.steps.is_empty());
    assert!(!instructions.title.is_empty());
}

#[test]
fn minimum_plugin_version_is_semver() {
    let version = GrokPluginManager.minimum_plugin_version();
    assert!(version.split('.').count() >= 2);
}

#[test]
#[serial_test::serial]
fn install_writes_plugin_files_under_grok_home() {
    let dir = tempfile::tempdir().unwrap();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("GROK_HOME", dir.path()) };

    install_plugin_files().expect("install should succeed");

    let hooks = dir.path().join("hooks");
    assert!(check_installed(&hooks));
    assert_eq!(
        installed_version(&hooks).as_deref(),
        Some(GrokPluginManager.minimum_plugin_version())
    );
    assert!(hooks.join(PLUGIN_SCRIPT_REL).is_file());
    let json = std::fs::read_to_string(hooks.join(HOOK_JSON_FILE)).unwrap();
    assert!(json.contains("warp-plugin.sh"));
    assert!(json.contains("SessionStart"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(hooks.join(PLUGIN_SCRIPT_REL))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "plugin script should be executable");
    }

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("GROK_HOME") };
}

#[test]
#[serial_test::serial]
fn needs_update_when_version_file_missing_but_hooks_present() {
    let dir = tempfile::tempdir().unwrap();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("GROK_HOME", dir.path()) };

    install_plugin_files().unwrap();
    std::fs::remove_file(dir.path().join("hooks").join(VERSION_FILE)).unwrap();

    assert!(GrokPluginManager.is_installed());
    assert!(GrokPluginManager.needs_update());

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("GROK_HOME") };
}

#[test]
#[serial_test::serial]
fn needs_update_when_version_is_old() {
    let dir = tempfile::tempdir().unwrap();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("GROK_HOME", dir.path()) };

    install_plugin_files().unwrap();
    std::fs::write(dir.path().join("hooks").join(VERSION_FILE), "0.0.1\n").unwrap();

    assert!(GrokPluginManager.is_installed());
    assert!(GrokPluginManager.needs_update());

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("GROK_HOME") };
}

#[test]
#[serial_test::serial]
fn is_installed_false_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("GROK_HOME", dir.path()) };

    assert!(!GrokPluginManager.is_installed());
    assert!(!GrokPluginManager.needs_update());

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("GROK_HOME") };
}
