use super::DroidPluginManager;
use crate::terminal::cli_agent_sessions::plugin_manager::CliAgentPluginManager;
use serde_json::Value;

#[test]
fn can_auto_install_is_false() {
    assert!(!DroidPluginManager.can_auto_install());
}

#[test]
fn minimum_version() {
    assert_eq!(DroidPluginManager.minimum_plugin_version(), "1.0.0");
}

#[test]
fn install_instructions_has_hooks_config_steps() {
    let instructions = DroidPluginManager.install_instructions();
    assert_eq!(instructions.title, "Install Warp Hooks for Droid");
    assert_eq!(instructions.steps.len(), 2);
    assert!(instructions.steps[0].command.contains("warp-notify.sh"));
    assert!(instructions.steps[1].command.contains("\"SessionStart\""));
    assert!(instructions.steps[1]
        .command
        .contains("\"UserPromptSubmit\""));
    assert!(instructions.steps[1].command.contains("\"Notification\""));
    assert!(instructions.steps[1].command.contains("\"Stop\""));
    assert!(instructions.steps[1].command.contains("\"PostToolUse\""));
    assert!(instructions.steps[1].command.contains("\"timeout\": 5"));
    assert!(instructions.steps[0].command.contains("question_asked"));
}

#[test]
fn hooks_json_instructions_use_top_level_event_names() {
    let instructions = DroidPluginManager.install_instructions();
    let hooks_config: Value = serde_json::from_str(instructions.steps[1].command).unwrap();

    assert!(hooks_config.get("hooks").is_none());
    assert!(hooks_config.get("SessionStart").is_some());
    assert!(hooks_config.get("UserPromptSubmit").is_some());
    assert!(hooks_config.get("Notification").is_some());
    assert!(hooks_config.get("Stop").is_some());
    assert!(hooks_config.get("PostToolUse").is_some());
}

#[test]
fn supports_update_is_false() {
    assert!(!DroidPluginManager.supports_update());
}
