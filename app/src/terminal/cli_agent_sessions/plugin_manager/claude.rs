use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use async_trait::async_trait;
use serde_json::Value;

use super::{
    compare_versions, run_cli_command_logged, CliAgentPluginManager, PluginInstallError,
    PluginInstructionStep, PluginInstructions,
};
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;

const PLUGIN_KEY: &str = "warp@claude-code-warp";
const MARKETPLACE_REPO: &str = "warpdotdev/claude-code-warp";
const MARKETPLACE_NAME: &str = "claude-code-warp";

const PLATFORM_PLUGIN_KEY: &str = "oz-harness-support@claude-code-warp";
// Note: we will eventually publish this to the same marketplace repo, but are using the internal one as we build out multi-harness.
const PLATFORM_MARKETPLACE_REPO: &str = "warpdotdev/claude-code-warp-internal";

// Keep in sync with the plugin version in warpdotdev/claude-code-warp.
// (See the Versioning section of that repo's README.)
const MINIMUM_PLUGIN_VERSION: &str = "2.1.0";

pub(super) struct ClaudeCodePluginManager {
    executor: LocalCommandExecutor,
    path_env_var: Option<String>,
}

impl ClaudeCodePluginManager {
    pub(super) fn new(
        shell_path: Option<PathBuf>,
        shell_type: Option<ShellType>,
        path_env_var: Option<String>,
    ) -> Self {
        let shell_type = shell_type.unwrap_or(ShellType::Bash);
        Self {
            executor: LocalCommandExecutor::new(shell_path, shell_type),
            path_env_var,
        }
    }

    async fn run_logged(&self, args: &[&str], log: &mut String) -> Result<(), PluginInstallError> {
        let env_vars = self
            .path_env_var
            .as_deref()
            .map(|path| HashMap::from([("PATH".to_owned(), path.to_owned())]));
        run_cli_command_logged("claude", args, &self.executor, env_vars, log).await
    }
}

#[async_trait]
impl CliAgentPluginManager for ClaudeCodePluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        MINIMUM_PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        true
    }

    fn is_installed(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        check_installed(&claude_dir)
    }

    /// Runs `claude plugin` CLI commands via the session shell.
    async fn install(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "add", MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "install", PLUGIN_KEY], &mut log)
            .await?;
        Ok(())
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        // Remove/re-add the marketplace to ensure the local clone is fresh, then
        // reinstall the plugin.
        // We use `plugin install` (not `plugin update`) because `marketplace
        // remove` unlinks the plugin, so `plugin update` would fail with
        // "Plugin is not installed".
        let _ = self
            .run_logged(
                &["plugin", "marketplace", "remove", MARKETPLACE_NAME],
                &mut log,
            )
            .await;
        self.run_logged(
            &["plugin", "marketplace", "add", MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "install", PLUGIN_KEY], &mut log)
            .await?;

        // Sanity check: verify the on-disk version actually changed.
        let still_outdated = claude_home_dir()
            .ok()
            .and_then(|dir| installed_version(&dir))
            .map(|v| compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt())
            .unwrap_or(true);
        if still_outdated {
            log.push_str("Post-update version check: plugin is still outdated\n");
            return Err(PluginInstallError {
                message: "Plugin update did not take effect".to_owned(),
                log,
            });
        }
        Ok(())
    }

    fn install_success_message(&self) -> &'static str {
        "Warp plugin installed. Please run /reload-plugins to activate."
    }

    fn update_success_message(&self) -> &'static str {
        "Warp plugin updated. Please run /reload-plugins to activate."
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &UPDATE_INSTRUCTIONS
    }

    fn needs_update(&self) -> bool {
        let Ok(claude_dir) = claude_home_dir() else {
            return false;
        };
        match installed_version(&claude_dir) {
            Some(v) => compare_versions(&v, MINIMUM_PLUGIN_VERSION).is_lt(),
            // No version field means very old plugin.
            None => check_installed(&claude_dir),
        }
    }

    async fn install_platform_plugin(&self) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        self.run_logged(
            &["plugin", "marketplace", "add", PLATFORM_MARKETPLACE_REPO],
            &mut log,
        )
        .await?;
        self.run_logged(&["plugin", "install", PLATFORM_PLUGIN_KEY], &mut log)
            .await?;
        Ok(())
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
        title: "Install Warp Plugin for Claude Code",
        subtitle: "Ensure that jq is installed on your machine. Then, run these commands.",
        steps: &[
            PluginInstructionStep {
                description: "Add the Warp plugin marketplace repository",
                command: "claude plugin marketplace add warpdotdev/claude-code-warp",
                executable: true,
                link: None,
            },
            PluginInstructionStep {
                description: "Install the Warp plugin",
                command: "claude plugin install warp@claude-code-warp",
                executable: true,
                link: None,
            },
        ],
        post_install_notes: &[
            "Restart Claude Code to activate the plugin.",
            "There are some known issues with Claude Code's plugin system. \
             If the plugin is not found after step 1, you can try manually adding an \"extraKnownMarketplaces\" entry to ~/.claude/settings.json.",
        ],
    }
});

static UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "Update Warp Plugin for Claude Code",
    subtitle: "Run the following commands.",
    steps: &[
        PluginInstructionStep {
            description: "Remove the existing marketplace (if present)",
            command: "claude plugin marketplace remove claude-code-warp",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: "Re-add the marketplace",
            command: "claude plugin marketplace add warpdotdev/claude-code-warp",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: "Install the latest plugin version",
            command: "claude plugin install warp@claude-code-warp",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: &["Restart Claude Code to activate the update."],
});

fn check_installed(claude_dir: &Path) -> bool {
    let plugins_path = claude_dir.join("plugins").join("installed_plugins.json");
    let Ok(contents) = fs::read_to_string(plugins_path) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&contents) else {
        return false;
    };
    parsed
        .get("plugins")
        .and_then(|p| p.get(PLUGIN_KEY))
        .and_then(|v| v.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false)
}

/// Reads the installed version string for the Warp plugin, if present.
fn installed_version(claude_dir: &Path) -> Option<String> {
    let plugins_path = claude_dir.join("plugins").join("installed_plugins.json");
    let contents = fs::read_to_string(plugins_path).ok()?;
    let parsed: Value = serde_json::from_str(&contents).ok()?;
    parsed
        .get("plugins")?
        .get(PLUGIN_KEY)?
        .as_array()?
        .first()?
        .get("version")?
        .as_str()
        .map(|s| s.to_owned())
}

/// Checks `CLAUDE_HOME` env var first, falls back to `~/.claude`.
fn claude_home_dir() -> io::Result<PathBuf> {
    if let Ok(claude_home) = env::var("CLAUDE_HOME") {
        return Ok(PathBuf::from(claude_home));
    }
    dirs::home_dir()
        .map(|home| home.join(".claude"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine home directory",
            )
        })
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
