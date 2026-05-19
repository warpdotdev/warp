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

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
    title_key: "terminal.plugin_instructions.codex.install.title",
    title: "Enable Warp Notifications for Codex",
    subtitle_key: "terminal.plugin_instructions.codex.install.subtitle",
    subtitle: "Update Codex to the latest version, then enable in-focus notifications so Warp can display them while you work.",
    steps: &[
        PluginInstructionStep {
            description_key: "terminal.plugin_instructions.codex.install.step.update",
            description: "Update Codex to the latest version.",
            command: "",
            executable: false,
            link: Some("https://developers.openai.com/codex/cli#upgrade"),
        },
        PluginInstructionStep {
            description_key: "terminal.plugin_instructions.codex.install.step.config",
            description: "Set the notification condition to \"always\" in your Codex config. Open or create ~/.codex/config.toml and add:",
            command: "[tui]\nnotification_condition = \"always\"",
            executable: false,
            link: None,
        },
    ],
    post_install_note_keys: &["terminal.plugin_instructions.codex.install.note.restart"],
    post_install_notes: &["Restart Codex to apply the changes."],
}
});

static EMPTY_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title_key: "",
    title: "",
    subtitle_key: "",
    subtitle: "",
    steps: &[],
    post_install_note_keys: &[],
    post_install_notes: &[],
});

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
