use std::sync::LazyLock;

use async_trait::async_trait;

use super::{CliAgentPluginManager, PluginInstructionStep, PluginInstructions};

// Keep in sync with the opencode-warp npm package version.
// This version is also hardcoded into UPDATE_INSTRUCTIONS below (so the update
// instructions tell users to pin to this specific version to force OpenCode's
// plugin cache to re-fetch). Update both together.
const MINIMUM_PLUGIN_VERSION: &str = "0.1.5";

pub(super) struct OpenCodePluginManager;

#[async_trait]
impl CliAgentPluginManager for OpenCodePluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        false
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &UPDATE_INSTRUCTIONS
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
    title_key: "terminal.plugin_instructions.opencode.install.title",
    title: "Install Warp Plugin for OpenCode",
    subtitle_key: "terminal.plugin_instructions.opencode.install.subtitle",
    subtitle: "Add the Warp plugin to your OpenCode configuration, then restart OpenCode.",
    steps: &[
        PluginInstructionStep {
            description_key: "terminal.plugin_instructions.opencode.install.step.open_config",
            description: "Open or create your opencode.json. This can be in your project root, or the global config path:",
            command: "~/.config/opencode/opencode.json",
            executable: false,
            link: None,
        },
        PluginInstructionStep {
            description_key: "terminal.plugin_instructions.opencode.install.step.add_plugin",
            description: "Add \"@warp-dot-dev/opencode-warp\" to the \"plugin\" array in the top-level JSON object:",
            command: "\"plugin\": [\"@warp-dot-dev/opencode-warp\"]",
            executable: false,
            link: None,
        },
    ],
    post_install_note_keys: &["terminal.plugin_instructions.opencode.install.note.restart"],
    post_install_notes: &["Restart OpenCode to activate the plugin."],
}
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
    title_key: "terminal.plugin_instructions.opencode.update.title",
    title: "Update Warp Plugin for OpenCode",
    subtitle_key: "terminal.plugin_instructions.opencode.update.subtitle",
    subtitle: "Pin the plugin to the latest version in your opencode.json. OpenCode caches plugins per version spec, so changing the pin forces it to re-fetch on restart.",
    steps: &[
        PluginInstructionStep {
            description_key: "terminal.plugin_instructions.opencode.update.step.open_config",
            description: "Open or create your opencode.json. This can be in your project root, or the global config path:",
            command: "~/.config/opencode/opencode.json",
            executable: false,
            link: None,
        },
        PluginInstructionStep {
            description_key: "terminal.plugin_instructions.opencode.update.step.replace_plugin",
            description: "Replace the existing \"@warp-dot-dev/opencode-warp\" entry in the \"plugin\" array with the explicit version:",
            command: "\"plugin\": [\"@warp-dot-dev/opencode-warp@0.1.5\"]",
            executable: false,
            link: None,
        },
    ],
    post_install_note_keys: &["terminal.plugin_instructions.opencode.update.note.restart"],
    post_install_notes: &["Restart OpenCode to load the updated plugin."],
}
});

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod tests;
