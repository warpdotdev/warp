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
        title: "为 Codex 启用 Warp 通知",
        subtitle: "先将 Codex 更新到最新版本，然后启用聚焦时通知，这样 Warp 才能在你工作时显示这些通知。",
        steps: &[
            PluginInstructionStep {
                description: "将 Codex 更新到最新版本。",
                command: "",
                executable: false,
                link: Some("https://developers.openai.com/codex/cli#upgrade"),
            },
            PluginInstructionStep {
                description:
                    "在你的 Codex 配置中将通知条件设为 \"always\"。打开或创建 ~/.codex/config.toml，并添加：",
                command: "[tui]\nnotification_condition = \"always\"",
                executable: false,
                link: None,
            },
        ],
        post_install_notes: &["重启 Codex 以应用更改。"],
    }
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
