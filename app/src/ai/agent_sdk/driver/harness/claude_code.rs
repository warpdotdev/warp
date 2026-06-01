use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_managed_secrets::ManagedSecretValue;
use warpui::{ModelHandle, ModelSpawner};

use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::claude_transcript::{
    claude_config_dir, home_dir_for_claude_config, read_envelope, write_envelope,
    write_session_index_entry, ClaudeResumeInfo, ClaudeTranscriptEnvelope,
};
use super::json_utils::{read_json_file_or_default, write_json_file};
use super::{
    cli_agent_session_status, write_temp_file, HarnessCleanupDisposition, HarnessRunner,
    JSONMCPServer, ResumePayload, SavePoint, ThirdPartyHarness,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::mcp::JSONTransportType;
use crate::server::server_api::harness_support::{upload_to_target, HarnessSupportClient};
use crate::server::server_api::ServerApi;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::ExecuteCommandOptions;
use crate::terminal::CLIAgent;
mod parent_bridge;
mod wake_driver;

#[cfg(test)]
use parent_bridge::{
    acknowledge_parent_bridge_hook_output, ensure_parent_bridge_state_dir,
    parent_bridge_char_count, parent_bridge_event_cursor_file, parent_bridge_hook_output_ack_file,
    parent_bridge_hook_output_file, parent_bridge_root, parent_bridge_staged_message_path,
    parent_bridge_surfaced_message_path, prepare_parent_bridge_hook_output,
    prime_parent_bridge_staged_for_self_managed_wake, read_parent_bridge_event_cursor,
    render_parent_bridge_message_block, stage_parent_bridge_message,
    write_parent_bridge_event_cursor, MessageBridgeHookOutput, MessageBridgeMessageRecord,
    MESSAGE_BRIDGE_CONTEXT_PREAMBLE,
};
use parent_bridge::{MessageBridge, MessageBridgeCleanupDisposition};
#[cfg(test)]
use shell_words::quote as shell_quote;
#[cfg(test)]
use wake_driver::{ClaudeWakeRemoteContext, CLAUDE_WAKE_PROMPT_FILE_NAME};

#[cfg(test)]
use super::super::OZ_MESSAGE_LISTENER_STATE_ROOT_ENV;

pub(crate) struct ClaudeHarness;
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for ClaudeHarness {
    fn harness(&self) -> Harness {
        Harness::Claude
    }

    fn cli_agent(&self) -> CLIAgent {
        CLIAgent::Claude
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        Some("https://code.claude.com/docs/en/quickstart")
    }

    fn auth_check_command(&self) -> Option<String> {
        let cli = self.cli_agent().command_prefix();
        Some(format!("{cli} auth status --json"))
    }

    fn runtime_error_patterns(&self) -> &'static [&'static str] {
        &[
            // Out-of-credits / billing.
            "Credit balance too low",
            // Plan/usage limits emitted as `You've hit your <kind> limit`.
            // We match on the common prefix so the variants (session,
            // weekly, Opus, etc.) all hit.
            "You've hit your",
            // Invalid or malformed API key.
            "Invalid API key",
            "This organization has been disabled",
            "belongs to a disabled organization",
            // OAuth / login state.
            "Not logged in",
            "OAuth token revoked",
            "OAuth token has expired",
            // Routines disabled by org policy.
            "Routines are disabled by your organization's policy",
            // Generic upstream API failures Claude Code surfaces verbatim.
            "API Error: Request rejected (429)",
            "authentication_error",
        ]
    }

    /// Fetch the Claude Code transcript for the current task's conversation and wrap it
    /// into a [`ResumePayload::Claude`]. Maps a server 404 to
    /// [`AgentDriverError::ConversationResumeStateMissing`] tagged as the `claude` harness
    /// so the user sees a resume-specific error rather than a generic load failure.
    async fn fetch_resume_payload(
        &self,
        conversation_id: &AIConversationId,
        harness_support_client: Arc<dyn HarnessSupportClient>,
    ) -> Result<Option<ResumePayload>, AgentDriverError> {
        let envelope: ClaudeTranscriptEnvelope =
            super::fetch_transcript_envelope("claude", conversation_id, harness_support_client)
                .await?;
        let session_id = envelope.uuid;
        Ok(Some(ResumePayload::Claude(ClaudeResumeInfo {
            conversation_id: *conversation_id,
            session_id,
            envelope,
        })))
    }

    fn build_runner(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        resumption_prompt: Option<&str>,
        context: Option<&str>,
        working_dir: &Path,
        task_id: Option<AmbientAgentTaskId>,
        server_api: Arc<ServerApi>,
        terminal_driver: ModelHandle<TerminalDriver>,
        resume: Option<ResumePayload>,
        resolved_env_vars: &HashMap<OsString, OsString>,
        _resolved_secrets: &HashMap<String, ManagedSecretValue>,
        resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
        _third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        // Prepare the environment config files.
        prepare_claude_environment_config(working_dir, resolved_env_vars).map_err(|error| {
            AgentDriverError::HarnessConfigSetupFailed {
                harness: self.cli_agent().command_prefix().to_owned(),
                error,
            }
        })?;

        // The ResumePayload shouldn't contain non-Claude information, error if it does.
        let claude_resume = resume.map(ClaudeResumeInfo::try_from).transpose()?;
        // Claude treats the user-turn message as immediate intent, so the resumption preamble
        // and server context are most reliable when prepended directly to the prompt that gets
        // piped into the CLI. Order: resumption_prompt → context → prompt
        let mut parts: Vec<&str> = Vec::new();
        if let Some(preamble) = resumption_prompt {
            if !preamble.is_empty() {
                parts.push(preamble);
            }
        }
        if let Some(ctx) = context {
            if !ctx.is_empty() {
                parts.push(ctx);
            }
        }
        parts.push(prompt);
        let owned_prompt = parts.join("\n\n");
        Ok(Box::new(ClaudeHarnessRunner::new(
            self.cli_agent().command_prefix(),
            &owned_prompt,
            system_prompt,
            working_dir,
            task_id,
            server_api,
            terminal_driver,
            claude_resume,
            resolved_mcp_servers,
        )?))
    }
}

/// Format slug sent to the server when creating a Claude Code conversation.
const CLAUDE_CODE_FORMAT: &str = "claude_code_cli";
/// Command used to exit claude.
const CLAUDE_EXIT_COMMAND: &str = "/exit";

/// Build the shell command that launches the Claude CLI for a given session and
/// prompt file.
///
/// When `resuming` is true we pass `--resume <uuid>` so Claude picks up the
/// existing on-disk session; otherwise we pass `--session-id <uuid>` to pin a
/// fresh session to that id. If `system_prompt_path` is provided, the CLI is
/// told to append its contents to the base system prompt.
fn claude_command(
    cli_name: &str,
    session_id: &Uuid,
    prompt_path: &str,
    system_prompt_path: Option<&str>,
    mcp_config_path: Option<&str>,
    resuming: bool,
) -> String {
    let flag = if resuming { "--resume" } else { "--session-id" };
    let mut cmd = format!("{cli_name} {flag} {session_id} --dangerously-skip-permissions");
    if let Some(sp_path) = system_prompt_path {
        let _ = write!(cmd, " --append-system-prompt-file '{sp_path}'");
    }
    if let Some(mcp_path) = mcp_config_path {
        let _ = write!(cmd, " --mcp-config '{mcp_path}'");
    }
    format!("{cmd} < '{prompt_path}'")
}

/// Runtime state of a [`ClaudeHarnessRunner`].
enum ClaudeRunnerState {
    /// Runner is built but [`HarnessRunner::start`] has not been called yet.
    Preexec,
    /// The harness command is running (or has finished).
    Running {
        conversation_id: AIConversationId,
        block_id: BlockId,
    },
}

struct ClaudeHarnessRunner {
    command: String,
    /// The CLI name used to invoke Claude Code.
    cli_name: String,
    /// Held so the temp file is cleaned up when the runner is dropped.
    _temp_prompt_file: NamedTempFile,
    /// Held so the system prompt temp file is cleaned up when the runner is dropped.
    _temp_system_prompt_file: Option<NamedTempFile>,
    /// Held so the MCP config temp file lives until the CLI exits.
    _temp_mcp_config_file: Option<NamedTempFile>,
    client: Arc<dyn HarnessSupportClient>,
    server_api: Arc<ServerApi>,
    terminal_driver: ModelHandle<TerminalDriver>,
    state: Mutex<ClaudeRunnerState>,
    session_id: Uuid,
    working_dir: PathBuf,
    parent_bridge: Option<MessageBridge>,
    /// Lazily cached output of `claude --version`.
    claude_version: Mutex<Option<String>>,
    /// When resuming an existing conversation, we pin the runner's server conversation id
    /// up front instead of calling `create_external_conversation` in [`HarnessRunner::start`].
    /// Subsequent saves overwrite the same GCS objects keyed by this id.
    preexisting_conversation_id: Option<AIConversationId>,
}

impl ClaudeHarnessRunner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        cli_command: &str,
        prompt: &str,
        system_prompt: Option<&str>,
        working_dir: &Path,
        task_id: Option<AmbientAgentTaskId>,
        server_api: Arc<ServerApi>,
        terminal_driver: ModelHandle<TerminalDriver>,
        resume: Option<ClaudeResumeInfo>,
        resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
    ) -> Result<Self, AgentDriverError> {
        // Write the prompt to a temp file so we can feed it via stdin redirect,
        // avoiding shell-quoting issues with complex content (e.g. skill instructions).
        let temp_file = write_temp_file("oz_prompt_", prompt, ".txt")?;
        let prompt_path = temp_file.path().display().to_string();

        let (session_id, preexisting_conversation_id) = match resume {
            Some(ClaudeResumeInfo {
                conversation_id,
                session_id,
                mut envelope,
            }) => {
                // Rehydrate the stored envelope under the current working directory so
                // `claude --resume <uuid>` finds the jsonl under ~/.claude/projects/<encoded_cwd>/.
                // The original envelope's cwd usually points at the cloud sandbox path, which
                // doesn't exist locally.
                envelope.cwd = working_dir.to_path_buf();
                let config_root = claude_config_dir().map_err(|e| {
                    AgentDriverError::ConfigBuildFailed(
                        e.context("Failed to resolve Claude config dir"),
                    )
                })?;
                write_envelope(&envelope, &config_root).map_err(|e| {
                    AgentDriverError::ConfigBuildFailed(
                        e.context("Failed to rehydrate Claude transcript"),
                    )
                })?;
                // Index write is best-effort: upstream Claude versions vary in how they use
                // `sessions-index.json`, so losing the index entry shouldn't abort the run.
                if let Err(e) = write_session_index_entry(session_id, working_dir, &config_root) {
                    log::warn!("Failed to update Claude sessions-index.json: {e:#}");
                }
                (session_id, Some(conversation_id))
            }
            None => (Uuid::new_v4(), None),
        };

        let temp_system_prompt_file = system_prompt
            .map(|sp| write_temp_file("oz_system_prompt_", sp, ".txt"))
            .transpose()?;
        let system_prompt_path = temp_system_prompt_file
            .as_ref()
            .map(|f| f.path().display().to_string());

        let temp_mcp_config_file = (!resolved_mcp_servers.is_empty())
            .then(|| {
                let mcp_json = serialize_claude_mcp_config(resolved_mcp_servers)
                    .map_err(AgentDriverError::ConfigBuildFailed)?;
                write_temp_file("oz_mcp_config_", &mcp_json, ".json")
            })
            .transpose()?;
        let mcp_config_path = temp_mcp_config_file
            .as_ref()
            .map(|f| f.path().display().to_string());

        let parent_bridge = task_id
            .map(|task_id| MessageBridge::new(task_id.to_string(), session_id))
            .transpose()
            .map_err(AgentDriverError::ConfigBuildFailed)?;
        let client: Arc<dyn HarnessSupportClient> = server_api.clone();

        Ok(Self {
            command: claude_command(
                cli_command,
                &session_id,
                &prompt_path,
                system_prompt_path.as_deref(),
                mcp_config_path.as_deref(),
                preexisting_conversation_id.is_some(),
            ),
            cli_name: cli_command.to_string(),
            _temp_prompt_file: temp_file,
            _temp_system_prompt_file: temp_system_prompt_file,
            _temp_mcp_config_file: temp_mcp_config_file,
            client,
            server_api,
            terminal_driver,
            state: Mutex::new(ClaudeRunnerState::Preexec),
            session_id,
            working_dir: working_dir.to_path_buf(),
            parent_bridge,
            claude_version: Mutex::new(None),
            preexisting_conversation_id,
        })
    }
}

impl ClaudeHarnessRunner {
    async fn handle_parent_bridge_session_update(&self) -> Result<()> {
        let Some(parent_bridge) = self.parent_bridge.as_ref() else {
            return Ok(());
        };
        parent_bridge
            .handle_session_update(self.server_api.clone())
            .await
    }

    async fn flush_parent_bridge_acks(&self) -> Result<()> {
        let Some(parent_bridge) = self.parent_bridge.as_ref() else {
            return Ok(());
        };
        parent_bridge.flush_acks(self.server_api.clone()).await
    }
    /// Return the cached Claude Code version, or resolve it by running
    /// `<cli_name> --version`.
    async fn resolve_claude_version(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Option<String> {
        if let Some(cached) = self.claude_version.lock().clone() {
            return Some(cached);
        }

        let terminal_driver = self.terminal_driver.clone();
        let session = foreground
            .spawn(move |_, ctx| {
                let tv = terminal_driver.as_ref(ctx).terminal_view().as_ref(ctx);
                tv.active_session().as_ref(ctx).session(ctx)
            })
            .await
            .ok()?;
        let session = session?;

        let cli_name = &self.cli_name;
        let output = session
            .execute_command(
                &format!("{cli_name} --version"),
                None,
                None,
                ExecuteCommandOptions::default(),
            )
            .await
            .ok()?;

        let version = output.to_string().ok()?.trim().to_string();
        if version.is_empty() {
            return None;
        }

        *self.claude_version.lock() = Some(version.clone());
        Some(version)
    }

    async fn start_parent_bridge(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        let Some(parent_bridge) = self.parent_bridge.as_ref() else {
            return Ok(());
        };
        parent_bridge
            .start(foreground, self.server_api.clone())
            .await
    }

    async fn should_preserve_parent_bridge(
        &self,
        cleanup_disposition: HarnessCleanupDisposition,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> bool {
        if !matches!(
            cleanup_disposition,
            HarnessCleanupDisposition::PreserveResumptionStateIfSupported
        ) {
            return false;
        }

        !matches!(
            cli_agent_session_status(&self.terminal_driver, foreground).await,
            Some(crate::terminal::cli_agent_sessions::CLIAgentSessionStatus::Blocked { .. })
                | Some(crate::terminal::cli_agent_sessions::CLIAgentSessionStatus::InProgress)
        )
    }

    fn cleanup_parent_bridge(&self, preserve_state: bool) -> Result<()> {
        if let Some(parent_bridge) = self.parent_bridge.as_ref() {
            let cleanup_disposition = if preserve_state {
                MessageBridgeCleanupDisposition::PreserveState
            } else {
                MessageBridgeCleanupDisposition::RemoveState
            };
            parent_bridge.cleanup(cleanup_disposition)?;
        }
        Ok(())
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for ClaudeHarnessRunner {
    fn harness_name(&self) -> &str {
        &self.cli_name
    }

    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<CommandHandle, AgentDriverError> {
        // When resuming, we already have a server conversation id from the prior run.
        // Otherwise create a fresh external conversation record for this run.
        // TODO(REMOTE-1149): `create_external_conversation` currently won't work for local CLI
        // runs. We should either support it or have a fallback.
        let conversation_id = match self.preexisting_conversation_id {
            Some(id) => {
                log::info!("Resuming external conversation {id}");
                id
            }
            None => {
                let id = setup_events
                    .record_result(SetupStep::ThirdPartyHarnessExternalConversation, async {
                        self.client
                            .create_external_conversation(CLAUDE_CODE_FORMAT)
                            .await
                            .map_err(|e| {
                                log::error!("Failed to create external conversation: {e}");
                                AgentDriverError::ConfigBuildFailed(e)
                            })
                    })
                    .await?;
                log::info!("Created external conversation {id}");
                id
            }
        };
        self.start_parent_bridge(foreground)
            .await
            .map_err(AgentDriverError::ConfigBuildFailed)?;

        let command = self.command.clone();
        let terminal_driver = self.terminal_driver.clone();
        let command_handle = match foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.execute_command(&command, ctx))
            })
            .await??
            .await
        {
            Ok(command_handle) => command_handle,
            Err(err) => {
                self.cleanup_parent_bridge(false)
                    .map_err(AgentDriverError::ConfigBuildFailed)?;
                return Err(err);
            }
        };

        // Only store conversation info once the CLI command has started.
        *self.state.lock() = ClaudeRunnerState::Running {
            conversation_id,
            block_id: command_handle.block_id().clone(),
        };

        Ok(command_handle)
    }

    async fn exit(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        log::info!("Sending /exit to Claude Code CLI");
        let terminal_driver = self.terminal_driver.clone();
        foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| {
                    driver.send_text_to_cli(CLAUDE_EXIT_COMMAND.to_string(), ctx);
                });
            })
            .await
            .map_err(|_| anyhow::anyhow!("Agent driver dropped while sending /exit"))
    }

    async fn handle_session_update(&self, _foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        self.handle_parent_bridge_session_update().await
    }

    async fn save_conversation(
        &self,
        save_point: SavePoint,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        if matches!(save_point, SavePoint::Periodic)
            && !super::has_running_cli_agent(&self.terminal_driver, foreground).await
        {
            log::debug!("Will not save conversation, Claude Code not in progress");
            return Ok(());
        }

        let (conversation_id, block_id) = match &*self.state.lock() {
            ClaudeRunnerState::Preexec => {
                log::warn!("save_conversation called before start");
                return Ok(());
            }
            ClaudeRunnerState::Running {
                conversation_id,
                block_id,
            } => (*conversation_id, block_id.clone()),
        };

        let claude_version = self.resolve_claude_version(foreground).await;

        let client = self.client.as_ref();
        let session_id = self.session_id;
        let working_dir = &self.working_dir;

        futures::try_join!(
            super::upload_current_block_snapshot(
                foreground,
                &self.terminal_driver,
                client,
                conversation_id,
                block_id,
            ),
            upload_transcript(
                client,
                conversation_id,
                session_id,
                working_dir,
                claude_version
            ),
        )?;

        Ok(())
    }
    async fn cleanup(
        &self,
        cleanup_disposition: HarnessCleanupDisposition,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        self.flush_parent_bridge_acks().await?;
        let preserve_state = self
            .should_preserve_parent_bridge(cleanup_disposition, foreground)
            .await;
        self.cleanup_parent_bridge(preserve_state)
    }
}

/// Upload the Claude Code session transcript to the server.
async fn upload_transcript(
    client: &dyn HarnessSupportClient,
    conversation_id: AIConversationId,
    session_id: Uuid,
    working_dir: &Path,
    claude_version: Option<String>,
) -> Result<()> {
    log::info!("Uploading Claude Code transcript to conversation {conversation_id}");

    let config_dir = claude_config_dir().context("Failed to resolve Claude config dir")?;
    let working_dir = working_dir.to_path_buf();
    let body = tokio::task::spawn_blocking(move || {
        let mut envelope = read_envelope(session_id, &working_dir, &config_dir)
            .with_context(|| format!("Failed to read transcript for session {session_id}"))?;
        envelope.claude_version = claude_version;
        serde_json::to_vec(&envelope).context("Failed to serialize transcript envelope")
    })
    .await
    .context("read_envelope task panicked")??;
    let target = client
        .get_transcript_upload_target(&conversation_id)
        .await
        .with_context(|| format!("Failed to get transcript upload target for {conversation_id}"))?;
    upload_to_target(client.http_client(), &target, body).await
}
pub(crate) fn prepare_claude_environment_config(
    working_dir: &Path,
    resolved_env_vars: &HashMap<OsString, OsString>,
) -> Result<()> {
    let claude_json_path = claude_global_config_path()?;
    let claude_settings_path = claude_config_dir()?.join(CLAUDE_SETTINGS_FILE_NAME);
    let api_key_suffix = resolve_anthropic_api_key_suffix(resolved_env_vars);
    prepare_claude_config(&claude_json_path, working_dir, api_key_suffix.as_deref())?;
    prepare_claude_settings(&claude_settings_path)?;
    Ok(())
}

// This function is used specifically for determining where to land `.claude.json`.
fn claude_global_config_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir).join(CLAUDE_JSON_FILE_NAME));
        }
    }

    home_dir_for_claude_config()
        .map(|home| home.join(CLAUDE_JSON_FILE_NAME))
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
}

fn prepare_claude_config(
    claude_json_path: &Path,
    working_dir: &Path,
    api_key_suffix: Option<&str>,
) -> Result<()> {
    let mut claude_config: ClaudeConfig = read_json_file_or_default(claude_json_path)?;
    claude_config.has_completed_onboarding = true;
    claude_config.lsp_recommendation_disabled = true;
    claude_config
        .projects
        .entry(working_dir.to_string_lossy().into_owned())
        .or_default()
        .has_trust_dialog_accepted = true;
    if let Some(suffix) = api_key_suffix {
        let responses = claude_config
            .custom_api_key_responses
            .get_or_insert_with(CustomApiKeyResponses::default);
        if !responses.approved.iter().any(|s| s == suffix) {
            responses.approved.push(suffix.to_owned());
        }
    }
    write_json_file(
        claude_json_path,
        &claude_config,
        "Failed to serialize Claude config",
    )?;
    Ok(())
}

fn prepare_claude_settings(claude_settings_path: &Path) -> Result<()> {
    let mut settings: ClaudeSettings = read_json_file_or_default(claude_settings_path)?;
    settings.skip_dangerous_mode_permission_prompt = true;
    write_json_file(
        claude_settings_path,
        &settings,
        "Failed to serialize Claude settings",
    )?;
    Ok(())
}

const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const CLAUDE_JSON_FILE_NAME: &str = ".claude.json";
const CLAUDE_SETTINGS_FILE_NAME: &str = "settings.json";
const ANTHROPIC_API_KEY_SUFFIX_LEN: usize = 20;

#[derive(Default, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ClaudeConfig {
    #[serde(default)]
    has_completed_onboarding: bool,
    #[serde(default)]
    lsp_recommendation_disabled: bool,
    #[serde(default)]
    projects: HashMap<String, ClaudeProjectConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_api_key_responses: Option<CustomApiKeyResponses>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Default, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CustomApiKeyResponses {
    #[serde(default)]
    approved: Vec<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Default, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ClaudeProjectConfig {
    #[serde(default)]
    has_trust_dialog_accepted: bool,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Default, Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct ClaudeSettings {
    #[serde(default)]
    skip_dangerous_mode_permission_prompt: bool,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Try to get the last 20 chars of the ANTHROPIC_API_KEY, where 20 chars is the
/// suffix length that Claude Code truncates keys to.
fn resolve_anthropic_api_key_suffix(
    resolved_env_vars: &HashMap<OsString, OsString>,
) -> Option<String> {
    // Worker-injected process env wins.
    if let Ok(key) = std::env::var(ANTHROPIC_API_KEY_ENV) {
        if !key.is_empty() {
            return suffix_of(&key).map(str::to_owned);
        }
    }
    // Otherwise use the resolved value from the secrets map.
    resolved_env_vars
        .get(OsStr::new(ANTHROPIC_API_KEY_ENV))
        .and_then(|v| v.to_str())
        .and_then(suffix_of)
        .map(str::to_owned)
}

fn suffix_of(key: &str) -> Option<&str> {
    if key.len() >= ANTHROPIC_API_KEY_SUFFIX_LEN {
        key.get(key.len() - ANTHROPIC_API_KEY_SUFFIX_LEN..)
    } else {
        None
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeMcpConfig {
    mcp_servers: HashMap<String, ClaudeMcpServerEntry>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ClaudeMcpServerEntry {
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        args: Vec<String>,
        #[serde(skip_serializing_if = "HashMap::is_empty")]
        env: HashMap<String, String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    #[serde(rename = "http")]
    Http {
        url: String,
        #[serde(skip_serializing_if = "HashMap::is_empty")]
        headers: HashMap<String, String>,
    },
}

impl ClaudeMcpServerEntry {
    fn from_json_mcp_server(server: &JSONMCPServer) -> Self {
        match &server.transport_type {
            JSONTransportType::CLIServer {
                command,
                args,
                env,
                working_directory,
            } => Self::Stdio {
                command: command.clone(),
                args: args.clone(),
                env: env.clone(),
                cwd: working_directory.clone(),
            },
            JSONTransportType::SSEServer { url, headers } => Self::Http {
                url: url.clone(),
                headers: headers.clone(),
            },
        }
    }
}

/// Serialize resolved MCP servers into Claude Code's `--mcp-config` JSON format.
///
/// Produces `{ "mcpServers": { "name": { "type": "stdio"|"http", ... }, ... } }`.
pub(crate) fn serialize_claude_mcp_config(
    servers: &HashMap<String, JSONMCPServer>,
) -> Result<String> {
    let config = ClaudeMcpConfig {
        mcp_servers: servers
            .iter()
            .map(|(name, server)| {
                (
                    name.clone(),
                    ClaudeMcpServerEntry::from_json_mcp_server(server),
                )
            })
            .collect(),
    };
    serde_json::to_string_pretty(&config).context("Failed to serialize Claude MCP config")
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
