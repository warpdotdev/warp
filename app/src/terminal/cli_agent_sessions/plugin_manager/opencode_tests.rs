use super::OpenCodePluginManager;
use crate::terminal::cli_agent_sessions::plugin_manager::CliAgentPluginManager;

#[test]
fn can_auto_install_is_false() {
    assert!(!OpenCodePluginManager.can_auto_install());
}

#[test]
fn install_instructions_has_steps() {
    let instructions = OpenCodePluginManager.install_instructions();
    assert!(!instructions.steps.is_empty());
    assert!(!instructions.title.is_empty());
}

#[test]
fn update_instructions_has_steps() {
    let instructions = OpenCodePluginManager.update_instructions();
    assert!(!instructions.steps.is_empty());
    assert!(!instructions.title.is_empty());
}
