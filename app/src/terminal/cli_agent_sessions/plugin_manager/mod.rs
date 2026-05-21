pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod gemini;
pub(crate) mod opencode;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::PathBuf;

use async_trait::async_trait;

use crate::features::FeatureFlag;
use crate::terminal::model::session::LocalCommandExecutor;
use crate::terminal::shell::ShellType;
use crate::terminal::CLIAgent;
use claude::ClaudeCodePluginManager;
use codex::CodexPluginManager;
use gemini::GeminiPluginManager;
use opencode::OpenCodePluginManager;

/// Distinguishes whether the plugin instructions modal should show install or update steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginModalKind {
    Install,
    Update,
}

/// A single step in the plugin install/update instructions pane.
pub(crate) struct PluginInstructionStep {
    pub description: &'static str,
    pub command: &'static str,
    /// When true, the code block shows a "Run" button that inserts the command into the terminal.
    /// Defaults-by-convention to `true`; set to `false` for steps that are not runnable
    /// (e.g. config file snippets).
    pub executable: bool,
    /// Optional URL rendered as a clickable "Learn more" link after the description.
    /// When set with an empty `command`, the code block is omitted entirely.
    pub link: Option<&'static str>,
}

/// All content needed to render the plugin instructions pane for a given agent.
pub(crate) struct PluginInstructions {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub steps: &'static [PluginInstructionStep],
    /// Displayed after the steps in the same style as the subtitle, one per paragraph.
    pub post_install_notes: &'static [&'static str],
}

/// Error returned when plugin installation fails.
/// Carries both a short user-facing message (for the toast) and a detailed
/// command log (for the log file the user can inspect).
pub(crate) struct PluginInstallError {
    /// Short description shown in the toast notification.
    pub message: String,
    /// Detailed log of every command/step that was attempted.
    pub log: String,
}

impl fmt::Display for PluginInstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<io::Error> for PluginInstallError {
    fn from(err: io::Error) -> Self {
        let msg = err.to_string();
        Self {
            message: msg.clone(),
            log: msg,
        }
    }
}

/// Compares two `X.Y.Z` version strings.
/// Returns `Ordering::Less` if `a < b`, etc.
/// Unparseable components are treated as 0.
pub(crate) fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> [u64; 3] {
        let mut parts = s.splitn(3, '.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        [major, minor, patch]
    };
    parse(a).cmp(&parse(b))
}

/// Runs a CLI subcommand through [`LocalCommandExecutor`], appending the
/// command and its output to `log`.
pub(crate) async fn run_cli_command_logged(
    cli_name: &str,
    args: &[&str],
    executor: &LocalCommandExecutor,
    env_vars: Option<HashMap<String, String>>,
    log: &mut String,
) -> Result<(), PluginInstallError> {
    let display_cmd = format!("{cli_name} {}", args.join(" "));
    log.push_str(&format!("$ {display_cmd}\n"));
    let result = executor
        .execute_local_command_in_login_shell(&display_cmd, None, env_vars)
        .await;
    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            for stream in [&stdout, &stderr] {
                if stream.is_empty() {
                    continue;
                }
                log.push_str(stream);
                if !stream.ends_with('\n') {
                    log.push('\n');
                }
            }
            if output.success() {
                log.push('\n');
                return Ok(());
            }
            Err(PluginInstallError {
                message: format!("'{display_cmd}' failed"),
                log: log.to_owned(),
            })
        }
        Err(err) => {
            log.push_str(&format!("error: {err}\n"));
            Err(PluginInstallError {
                message: format!("failed to run '{display_cmd}'"),
                log: log.clone(),
            })
        }
    }
}

/// Manages the Warp notification plugin for a specific CLI agent.
///
/// Each supported CLI agent has its own implementation that knows how to
/// check installation state and perform install/update operations.
#[async_trait]
pub(crate) trait CliAgentPluginManager: Send + Sync {
    /// The minimum plugin version required by this Warp build.
    fn minimum_plugin_version(&self) -> &'static str;

    /// Whether this agent supports one-click auto-install/update.
    /// When `false`, the footer always opens the manual instructions modal.
    fn can_auto_install(&self) -> bool;

    /// Whether the Warp notification plugin is installed.
    /// Default returns `false` (no filesystem check).
    fn is_installed(&self) -> bool {
        false
    }

    /// Whether the on-disk plugin version is below the minimum required.
    /// Default returns `false` (no filesystem check).
    fn needs_update(&self) -> bool {
        false
    }

    /// Install the Warp notification plugin.
    /// Default returns an error — only agents with `can_auto_install() == true` should override.
    async fn install(&self) -> Result<(), PluginInstallError> {
        Err(PluginInstallError {
            message: "Auto-install not supported for this agent".to_owned(),
            log: String::new(),
        })
    }

    /// Update the Warp notification plugin to the latest version.
    /// Default returns an error — only agents with `can_auto_install() == true` should override.
    async fn update(&self) -> Result<(), PluginInstallError> {
        Err(PluginInstallError {
            message: "Auto-update not supported for this agent".to_owned(),
            log: String::new(),
        })
    }

    /// Toast message shown after a successful auto-install.
    fn install_success_message(&self) -> &'static str {
        "Warp plugin installed. Please restart the session to activate."
    }

    /// Toast message shown after a successful auto-update.
    fn update_success_message(&self) -> &'static str {
        "Warp plugin updated. Please restart the session to activate."
    }

    /// Manual installation instructions for the modal UI.
    fn install_instructions(&self) -> &'static PluginInstructions;

    /// Whether this agent supports version-based update checking.
    /// When `false`, the update chip is never shown; only the install chip appears.
    fn supports_update(&self) -> bool {
        true
    }

    /// Manual update instructions for the modal UI.
    fn update_instructions(&self) -> &'static PluginInstructions;

    /// Install the Oz platform plugin for this CLI agent, if one exists,
    /// which provides skills that third-party harnesses can use to interact with
    /// the Oz platform.
    /// Default is a no-op — only agents with a platform plugin should override.
    async fn install_platform_plugin(&self) -> Result<(), PluginInstallError> {
        Ok(())
    }
}

/// Returns a plugin manager for the given CLI agent, or `None` if the agent
/// doesn't have Warp notification plugin support.
pub(crate) fn plugin_manager_for(agent: CLIAgent) -> Option<Box<dyn CliAgentPluginManager>> {
    plugin_manager_for_with_shell(agent, None, None, None)
}
/// Returns a plugin manager for the given CLI agent, or `None` if the agent
/// doesn't have Warp notification plugin support.
///
/// When a shell path and type are provided, plugin commands run through that shell.
/// When `path_env_var` is provided, it is set as the PATH for plugin commands
/// (needed for nvm-installed tools that are only on PATH in interactive shells).
pub(crate) fn plugin_manager_for_with_shell(
    agent: CLIAgent,
    shell_path: Option<PathBuf>,
    shell_type: Option<ShellType>,
    path_env_var: Option<String>,
) -> Option<Box<dyn CliAgentPluginManager>> {
    match agent {
        CLIAgent::Claude => Some(Box::new(ClaudeCodePluginManager::new(
            shell_path,
            shell_type,
            path_env_var,
        ))),
        CLIAgent::OpenCode
            if FeatureFlag::OpenCodeNotifications.is_enabled()
                && FeatureFlag::HOANotifications.is_enabled() =>
        {
            Some(Box::new(OpenCodePluginManager))
        }
        CLIAgent::Codex
            if FeatureFlag::CodexNotifications.is_enabled()
                && FeatureFlag::HOANotifications.is_enabled() =>
        {
            Some(Box::new(CodexPluginManager))
        }
        CLIAgent::Gemini
            if FeatureFlag::GeminiNotifications.is_enabled()
                && FeatureFlag::HOANotifications.is_enabled() =>
        {
            Some(Box::new(GeminiPluginManager::new(
                shell_path,
                shell_type,
                path_env_var,
            )))
        }
        CLIAgent::OpenCode
        | CLIAgent::Codex
        | CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Hermes
        | CLIAgent::Goose
        | CLIAgent::Vibe
        | CLIAgent::Unknown => None,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
