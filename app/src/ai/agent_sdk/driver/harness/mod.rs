use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use tempfile::NamedTempFile;
use warp_cli::agent::Harness;
use warp_managed_secrets::ManagedSecretValue;
use warpui::{ModelHandle, ModelSpawner, SingletonEntity};

use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::CLIAgent;
use crate::util::path::resolve_executable;

use super::terminal::{CommandHandle, TerminalDriver};
use super::{AgentDriver, AgentDriverError};

mod claude_code;
mod gemini;
mod json_utils;

pub(crate) use claude_code::ClaudeHarness;
use gemini::GeminiHarness;

/// Trait for third-party agent harnesses that execute prompts via their own CLIs.
///
/// Each retained harness must be local-only: it may launch and configure the
/// third-party CLI, but must not create hosted conversations, poll tasks, upload
/// transcripts, or depend on hosted Warp server APIs.
#[async_trait]
pub(crate) trait ThirdPartyHarness: Send + Sync {
    /// Returns the [`Harness`] variant this implementation corresponds to.
    fn harness(&self) -> Harness;

    /// Returns the CLIAgent type associated with this harness.
    fn cli_agent(&self) -> CLIAgent;

    /// URL to install instructions for this harness's CLI, surfaced in the
    /// default [`validate`] impl when the CLI is not on `PATH`.
    fn install_docs_url(&self) -> Option<&'static str> {
        None
    }

    /// Validate that the harness is ready to run. Default impl checks that the
    /// CLI is installed on `PATH`; override for additional checks.
    fn validate(&self) -> Result<(), AgentDriverError> {
        validate_cli_installed(self.cli_agent().command_prefix(), self.install_docs_url())
    }

    /// Prepare CLI-specific config files before launching the harness command.
    fn prepare_environment_config(
        &self,
        _working_dir: &Path,
        _system_prompt: Option<&str>,
        _secrets: &HashMap<String, ManagedSecretValue>,
    ) -> Result<(), AgentDriverError> {
        Ok(())
    }

    /// Build a runner for executing this harness with the given prompt.
    #[allow(clippy::too_many_arguments)]
    fn build_runner(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        working_dir: &Path,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError>;
}

/// Harness type for driver dispatch.
pub(crate) enum HarnessKind {
    /// Third-party CLI-backed harness (e.g. Claude, Gemini).
    ThirdParty(Box<dyn ThirdPartyHarness>),
    /// Harnesses that exist in the shared CLI enum but are not supported by the
    /// standalone agent driver.
    Unsupported(Harness),
}

impl HarnessKind {
    /// Corresponding [`Harness`] enum value.
    pub(crate) fn harness(&self) -> Harness {
        match self {
            HarnessKind::ThirdParty(h) => h.harness(),
            HarnessKind::Unsupported(harness) => *harness,
        }
    }
}

impl fmt::Debug for HarnessKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use the `Display` method on the [`Harness`] enum.
        write!(f, "{}", self.harness())
    }
}

/// Build a [`HarnessKind`] for the given [`Harness`].
///
/// We shouldn't ever get an unknown local runner here because clap should handle
/// it.
pub(crate) fn harness_kind(harness: Harness) -> Result<HarnessKind, AgentDriverError> {
    match harness {
        Harness::Claude => Ok(HarnessKind::ThirdParty(Box::new(ClaudeHarness))),
        Harness::OpenCode => Ok(HarnessKind::Unsupported(Harness::OpenCode)),
        Harness::Gemini => Ok(HarnessKind::ThirdParty(Box::new(GeminiHarness))),
        Harness::Unknown => Err(AgentDriverError::InvalidRuntimeState),
    }
}

/// Check that `cli` is installed and on PATH, returning a `HarnessSetupFailed`
/// error with an optional install-docs link when it isn't.
pub(crate) fn validate_cli_installed(
    cli: &str,
    install_docs_url: Option<&str>,
) -> Result<(), AgentDriverError> {
    if resolve_executable(cli).is_none() {
        let mut reason = format!("'{cli}' CLI not found on your machine.");
        if let Some(url) = install_docs_url {
            reason.push_str(&format!(" Install it first: {url}"));
        }
        return Err(AgentDriverError::HarnessSetupFailed {
            harness: cli.into(),
            reason,
        });
    }
    Ok(())
}

/// Indicates when the harness conversation is being saved.
/// Implementations may use this to customize the saved data, such as
/// recording additional metadata on completion.
pub(crate) enum SavePoint {
    /// A periodic auto-save to minimize data loss.
    Periodic,
    /// The final save of conversation state, after the harness has completed.
    Final,
    /// A save after the harness reports it finished an agent turn.
    PostTurn,
}

/// Stateful per-run representation of an external harness produced
/// by [`ThirdPartyHarness::build_runner`].
///
/// All `HarnessRunner` methods take `&self` as a parameter, but may mutate internal
/// state. There are no `&mut self` methods, as this would require that the `AgentDriver`
/// store the runner in a mutex and lock it across `await` points.
///
/// The driver uses this to manage the lifecycle of a particular third-party harness.
#[async_trait]
pub(crate) trait HarnessRunner: Send + Sync {
    /// Start the harness command in the terminal.
    ///
    /// Returns a [`CommandHandle`] that resolves to the exit code. The runner
    /// may store local block/session state for process management.
    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<CommandHandle, AgentDriverError>;

    /// Save local harness state, if the retained harness has any.
    async fn save_conversation(
        &self,
        save_point: SavePoint,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()>;

    /// Gracefully ask the harness to exit.
    async fn exit(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()>;
    /// Handle a CLI session update such as a prompt submit or completed tool use.
    async fn handle_session_update(&self, _foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        Ok(())
    }

    /// Clean up any harness-owned background state after the harness exits.
    async fn cleanup(&self, _foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        Ok(())
    }
}

/// Returns `true` if the terminal tracked by `terminal_driver` has a CLI agent session
/// that is currently in progress.
pub(crate) async fn has_running_cli_agent(
    terminal_driver: &ModelHandle<TerminalDriver>,
    foreground: &ModelSpawner<AgentDriver>,
) -> bool {
    let driver = terminal_driver.clone();
    let Ok(running) = foreground
        .spawn(move |_, ctx| {
            let terminal_view_id = driver.as_ref(ctx).terminal_view().id();
            CLIAgentSessionsModel::handle(ctx)
                .as_ref(ctx)
                .session(terminal_view_id)
                .is_some_and(|s| s.status == CLIAgentSessionStatus::InProgress)
        })
        .await
    else {
        return false;
    };
    running
}

/// Create a [`NamedTempFile`] with the given prefix and write `content` into it.
///
/// Used by third-party harnesses to stage prompts / system prompts on disk
/// before launching the CLI, avoiding shell-quoting issues with complex input.
pub(super) fn write_temp_file(
    prefix: &str,
    content: &str,
) -> Result<NamedTempFile, AgentDriverError> {
    let mut file = tempfile::Builder::new()
        .prefix(prefix)
        .suffix(".txt")
        .tempfile()
        .map_err(|e| {
            AgentDriverError::ConfigBuildFailed(anyhow::anyhow!(
                "Failed to create temp file '{prefix}': {e}"
            ))
        })?;
    file.write_all(content.as_bytes()).map_err(|e| {
        AgentDriverError::ConfigBuildFailed(anyhow::anyhow!(
            "Failed to write temp file '{prefix}': {e}"
        ))
    })?;
    Ok(file)
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
