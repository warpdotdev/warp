use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use warp_cli::agent::Harness;
use warp_managed_secrets::ManagedSecretValue;
use warpui::{ModelHandle, ModelSpawner};

use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::json_utils::{read_json_file_or_default, write_json_file};
use super::{
    write_temp_file, HarnessCleanupDisposition, HarnessRunner, JSONMCPServer, ResumePayload,
    SavePoint, ThirdPartyHarness,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::setup_observability::{
    OzRunTimelineEvent, SetupClientEventReporter, SetupStep,
};
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::harness_support::HarnessSupportClient;
use crate::server::server_api::ServerApi;
use crate::terminal::model::block::BlockId;
use crate::terminal::CLIAgent;

pub(crate) struct GeminiHarness;

/// Format slug sent to the server when creating a Gemini conversation.
const GEMINI_CLI_FORMAT: &str = "gemini_cli";
/// Slash command Gemini's TUI recognises as a graceful shutdown.
const GEMINI_EXIT_COMMAND: &str = "/quit";

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for GeminiHarness {
    fn harness(&self) -> Harness {
        Harness::Gemini
    }

    fn cli_agent(&self) -> CLIAgent {
        CLIAgent::Gemini
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        Some("https://geminicli.com/")
    }

    fn build_runner(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        _resumption_prompt: Option<&str>,
        context: Option<&str>,
        working_dir: &Path,
        _task_id: Option<AmbientAgentTaskId>,
        server_api: Arc<ServerApi>,
        terminal_driver: ModelHandle<TerminalDriver>,
        _resume: Option<ResumePayload>,
        _resolved_env_vars: &HashMap<OsString, OsString>,
        _resolved_secrets: &HashMap<String, ManagedSecretValue>,
        _resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
        _third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        // Prepare the environment config files.
        prepare_gemini_environment_config(working_dir, system_prompt).map_err(|error| {
            AgentDriverError::HarnessConfigSetupFailed {
                harness: self.cli_agent().command_prefix().to_owned(),
                error,
            }
        })?;

        // Gemini does not support conversation resume yet. When it does, it will add its
        // own `ResumePayload::Gemini(..)` variant and override `fetch_resume_payload`,
        // and decide how to surface the user-turn resumption preamble.
        // Prepend server context to the prompt if available.
        let effective_prompt = match context {
            Some(ctx) if !ctx.is_empty() => format!("{ctx}\n\n{prompt}"),
            _ => prompt.to_string(),
        };
        let client: Arc<dyn HarnessSupportClient> = server_api;
        Ok(Box::new(GeminiHarnessRunner::new(
            self.cli_agent().command_prefix(),
            &effective_prompt,
            system_prompt,
            working_dir,
            client,
            terminal_driver,
        )?))
    }
}

/// Build the shell command that launches the Gemini TUI.
///
/// `--yolo` auto-approves tool call. `-i` seeds the initial prompt and
/// continues in interactive TUI mode.
fn gemini_command(cli_name: &str, prompt_path: &str) -> String {
    format!("{cli_name} --yolo -i \"$(cat '{prompt_path}')\"")
}

enum GeminiRunnerState {
    Preexec,
    Running {
        conversation_id: AIConversationId,
        block_id: BlockId,
    },
}

struct GeminiHarnessRunner {
    command: String,
    /// The CLI name used to invoke Gemini.
    cli_name: String,
    /// Held so the temp file is cleaned up when the runner is dropped.
    _temp_prompt_file: NamedTempFile,
    client: Arc<dyn HarnessSupportClient>,
    terminal_driver: ModelHandle<TerminalDriver>,
    state: Mutex<GeminiRunnerState>,
}

impl GeminiHarnessRunner {
    fn new(
        cli_command: &str,
        prompt: &str,
        _system_prompt: Option<&str>,
        _working_dir: &Path,
        client: Arc<dyn HarnessSupportClient>,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Self, AgentDriverError> {
        let temp_file = write_temp_file("oz_prompt_", prompt, ".txt")?;
        let prompt_path = temp_file.path().display().to_string();

        Ok(Self {
            command: gemini_command(cli_command, &prompt_path),
            cli_name: cli_command.to_string(),
            _temp_prompt_file: temp_file,
            client,
            terminal_driver,
            state: Mutex::new(GeminiRunnerState::Preexec),
        })
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for GeminiHarnessRunner {
    fn harness_name(&self) -> &str {
        &self.cli_name
    }

    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<CommandHandle, AgentDriverError> {
        // Create the external conversation record on the server.
        let conversation_id = setup_events
            .record_result(SetupStep::ThirdPartyHarnessExternalConversation, async {
                self.client
                    .create_external_conversation(GEMINI_CLI_FORMAT)
                    .await
                    .map_err(|e| {
                        log::error!("Failed to create external conversation: {e}");
                        AgentDriverError::ConfigBuildFailed(e)
                    })
            })
            .await?;
        log::info!("Created external conversation {conversation_id}");

        let command = self.command.clone();
        let terminal_driver = self.terminal_driver.clone();
        let command_handle = foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.execute_command(&command, ctx))
            })
            .await??
            .await?;

        // Only store conversation info once the CLI command has started.
        *self.state.lock() = GeminiRunnerState::Running {
            conversation_id,
            block_id: command_handle.block_id().clone(),
        };

        setup_events
            .post_timeline_event(OzRunTimelineEvent::AgentStarted)
            .await;

        Ok(command_handle)
    }

    async fn exit(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        log::info!("Sending /quit to Gemini CLI");
        let terminal_driver = self.terminal_driver.clone();
        foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| {
                    driver.send_text_to_cli(GEMINI_EXIT_COMMAND.to_string(), ctx);
                });
            })
            .await
            .map_err(|_| anyhow::anyhow!("Agent driver dropped while sending /quit"))
    }

    async fn save_conversation(
        &self,
        save_point: SavePoint,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        if matches!(save_point, SavePoint::Periodic)
            && !super::has_running_cli_agent(&self.terminal_driver, foreground).await
        {
            log::debug!("Will not save conversation, Gemini not in progress");
            return Ok(());
        }

        let (conversation_id, block_id) = match &*self.state.lock() {
            GeminiRunnerState::Preexec => {
                log::warn!("save_conversation called before start");
                return Ok(());
            }
            GeminiRunnerState::Running {
                conversation_id,
                block_id,
            } => (*conversation_id, block_id.clone()),
        };

        // TODO(REMOTE-1408) Also save the conversation transcript.
        super::upload_current_block_snapshot(
            foreground,
            &self.terminal_driver,
            self.client.as_ref(),
            conversation_id,
            block_id,
        )
        .await
    }

    async fn cleanup(
        &self,
        _cleanup_disposition: HarnessCleanupDisposition,
        _foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        Ok(())
    }
}

fn prepare_gemini_environment_config(
    working_dir: &Path,
    system_prompt: Option<&str>,
) -> Result<()> {
    let home_dir =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    let gemini_dir = home_dir.join(GEMINI_CONFIG_DIR);
    prepare_gemini_settings(
        &gemini_dir.join(GEMINI_SETTINGS_FILE_NAME),
        system_prompt.is_some(),
    )?;
    prepare_gemini_trusted_folders(
        &gemini_dir.join(GEMINI_TRUSTED_FOLDERS_FILE_NAME),
        working_dir,
    )?;
    if let Some(prompt) = system_prompt {
        let prompt_path = gemini_dir.join(GEMINI_SYSTEM_PROMPT_FILE_NAME);
        std::fs::write(&prompt_path, prompt).with_context(|| {
            format!(
                "Failed to write Gemini system prompt to {}",
                prompt_path.display()
            )
        })?;
    }
    Ok(())
}

fn prepare_gemini_settings(settings_path: &Path, has_system_prompt: bool) -> Result<()> {
    let mut settings: GeminiSettings = read_json_file_or_default(settings_path)?;
    settings
        .security
        .get_or_insert_with(GeminiSecurity::default)
        .auth
        .get_or_insert_with(GeminiAuth::default)
        .selected_type = Some(GEMINI_API_KEY_AUTH_TYPE.to_owned());

    if has_system_prompt {
        let context = settings.context.get_or_insert_with(GeminiContext::default);
        let file_name = GEMINI_SYSTEM_PROMPT_FILE_NAME.to_owned();
        if !context.file_name.contains(&file_name) {
            context.file_name.push(file_name);
        }
    }

    write_json_file(
        settings_path,
        &settings,
        "Failed to serialize Gemini settings",
    )
}

fn prepare_gemini_trusted_folders(trusted_path: &Path, working_dir: &Path) -> Result<()> {
    let mut trusted: HashMap<String, String> = read_json_file_or_default(trusted_path)?;
    trusted.insert(
        working_dir.to_string_lossy().into_owned(),
        GEMINI_TRUST_LEVEL_FOLDER.to_owned(),
    );
    write_json_file(
        trusted_path,
        &trusted,
        "Failed to serialize Gemini trusted folders",
    )
}

const GEMINI_CONFIG_DIR: &str = ".gemini";
const GEMINI_SETTINGS_FILE_NAME: &str = "settings.json";
const GEMINI_TRUSTED_FOLDERS_FILE_NAME: &str = "trustedFolders.json";
const GEMINI_SYSTEM_PROMPT_FILE_NAME: &str = "OZ_SYSTEM_PROMPT.md";
/// Auth-type discriminant for API-key auth — matches `AuthType.USE_GEMINI` in
/// Gemini's `packages/core/src/core/contentGenerator.ts`.
const GEMINI_API_KEY_AUTH_TYPE: &str = "gemini-api-key";
/// Trust level discriminant that grants full trust to a single folder — matches
/// Gemini's `TrustLevel.TRUST_FOLDER` in `packages/cli/src/config/trustedFolders.ts`.
const GEMINI_TRUST_LEVEL_FOLDER: &str = "TRUST_FOLDER";

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    security: Option<GeminiSecurity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<GeminiContext>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiSecurity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth: Option<GeminiAuth>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_type: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContext {
    #[serde(default)]
    file_name: Vec<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[cfg(test)]
#[path = "gemini_tests.rs"]
mod tests;
