use std::sync::LazyLock;

use async_trait::async_trait;

use super::{CliAgentPluginManager, PluginInstructionStep, PluginInstructions};

pub(super) struct CodexPluginManager;

#[async_trait]
impl CliAgentPluginManager for CodexPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        "0.0.0"
    }

    fn can_auto_install(&self) -> bool {
        false
    }

    fn supports_update(&self) -> bool {
        false
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &EMPTY_INSTRUCTIONS
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "terminal.plugin_instructions.codex.install.title",
    subtitle: "terminal.plugin_instructions.codex.install.subtitle",
    steps: &[
        PluginInstructionStep {
            description: "terminal.plugin_instructions.codex.install.step.update",
            command: "",
            executable: false,
            link: Some("https://developers.openai.com/codex/cli#upgrade"),
        },
        PluginInstructionStep {
            description: "terminal.plugin_instructions.codex.install.step.config",
            command: "[tui]\nnotification_condition = \"always\"",
            executable: false,
            link: None,
        },
    ],
    post_install_notes: &["terminal.plugin_instructions.codex.install.note.restart"],
});

static EMPTY_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "",
    subtitle: "",
    steps: &[],
    post_install_notes: &[],
});

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
