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

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Install Warp Plugin for OpenCode",
    subtitle: "Add the Warp plugin to your OpenCode configuration, then restart OpenCode.",
    steps: &[
        PluginInstructionStep {
            description: "Open or create your opencode.json. This can be in your project root, or the global config path:",
            command: "~/.config/opencode/opencode.json",
            executable: false,
            link: None,
        },
        PluginInstructionStep {
            description: "Add \"@warp-dot-dev/opencode-warp\" to the \"plugin\" array in the top-level JSON object:",
            command: "\"plugin\": [\"@warp-dot-dev/opencode-warp\"]",
            executable: false,
            link: None,
        },
    ],
    post_install_notes: &["Restart OpenCode to activate the plugin."],
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Update Warp Plugin for OpenCode",
    subtitle: "Pin the plugin to the latest version in your opencode.json. OpenCode caches plugins per version spec, so changing the pin forces it to re-fetch on restart.",
    steps: &[
        PluginInstructionStep {
            description: "Open or create your opencode.json. This can be in your project root, or the global config path:",
            command: "~/.config/opencode/opencode.json",
            executable: false,
            link: None,
        },
        PluginInstructionStep {
            description: "Replace the existing \"@warp-dot-dev/opencode-warp\" entry in the \"plugin\" array with the explicit version:",
            command: "\"plugin\": [\"@warp-dot-dev/opencode-warp@0.1.5\"]",
            executable: false,
            link: None,
        },
    ],
    post_install_notes: &["Restart OpenCode to load the updated plugin."],
});

#[cfg(test)]
#[path = "opencode_tests.rs"]
mod tests;
