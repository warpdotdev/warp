use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_managed_secrets::ManagedSecretValue;
use warpui::{ModelHandle, ModelSpawner, SingletonEntity};

use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::claude_transcript::read_jsonl;
use super::codex_transcript::{
    codex_sessions_root, find_session_file, parse_session_meta, rehydrate_codex_transcript,
    CodexResumeInfo, CodexTranscriptEnvelope,
};
use super::json_utils::read_json_file_or_default;
use super::{
    write_temp_file, HarnessRunner, JSONMCPServer, ResumePayload, SavePoint, ThirdPartyHarness,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::setup_observability::{
    OzRunTimelineEvent, SetupClientEventReporter, SetupStep,
};
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::mcp::JSONTransportType;
use crate::server::server_api::harness_support::{upload_to_target, HarnessSupportClient};
use crate::server::server_api::ServerApi;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::model::block::BlockId;
use crate::terminal::CLIAgent;

pub(crate) struct CodexHarness;

/// Format slug sent to the server when creating a Codex conversation.
const CODEX_CLI_FORMAT: &str = "codex_cli";
/// Slash command Codex's TUI recognises as a graceful shutdown.
const CODEX_EXIT_COMMAND: &str = "/exit";
/// Allow the Warp-installed Codex plugin hooks to run in vetted driver sessions
/// without requiring an unattended `/hooks` review step.
const CODEX_BYPASS_HOOK_TRUST_FLAG: &str = "--dangerously-bypass-hook-trust";

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for CodexHarness {
    fn harness(&self) -> Harness {
        Harness::Codex
    }

    fn cli_agent(&self) -> CLIAgent {
        CLIAgent::Codex
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        Some("https://developers.openai.com/codex/cli")
    }

    fn auth_check_command(&self) -> Option<String> {
        let cli = self.cli_agent().command_prefix();
        Some(format!("{cli} login status"))
    }

    fn runtime_error_patterns(&self) -> &'static [&'static str] {
        &[
            // Quota / billing.
            "Quota exceeded. Check your plan and billing details.",
            "You've hit your usage limit",
            // Upstream HTTP failures Codex surfaces verbatim. The 401 form
            // matches invalid-API-key and wrong-endpoint variants.
            "unexpected status 401",
            "Incorrect API key provided",
            "invalid API key",
            // Region/endpoint block (Anthropic-style global vs US-only
            // routing surfaced through Codex's upstream client).
            "Access blocked by Cloudflare",
            // OAuth refresh failures — all five Codex variants share this
            // substring (see upstream session/token messages).
            "could not be refreshed",
        ]
    }

    fn requires_verified_platform_plugin(&self) -> bool {
        FeatureFlag::CodexPlugin.is_enabled()
    }

    /// Fetch the codex transcript for the current task's conversation and wrap it into a
    /// [`ResumePayload::Codex`].
    async fn fetch_resume_payload(
        &self,
        conversation_id: &AIConversationId,
        harness_support_client: Arc<dyn HarnessSupportClient>,
    ) -> Result<Option<ResumePayload>, AgentDriverError> {
        let envelope: CodexTranscriptEnvelope =
            super::fetch_transcript_envelope("codex", conversation_id, harness_support_client)
                .await?;
        let session_id = envelope.session_id;
        Ok(Some(ResumePayload::Codex(CodexResumeInfo {
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
        _task_id: Option<AmbientAgentTaskId>,
        server_api: Arc<ServerApi>,
        terminal_driver: ModelHandle<TerminalDriver>,
        resume: Option<ResumePayload>,
        resolved_env_vars: &HashMap<OsString, OsString>,
        resolved_secrets: &HashMap<String, ManagedSecretValue>,
        resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
        third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        // Prepare the environment config files.
        prepare_codex_environment_config(
            working_dir,
            system_prompt,
            resolved_env_vars,
            resolved_secrets,
            resolved_mcp_servers,
            third_party_harness_model_config,
        )
        .map_err(|error| AgentDriverError::HarnessConfigSetupFailed {
            harness: self.cli_agent().command_prefix().to_owned(),
            error,
        })?;

        // The ResumePayload shouldn't contain non-Codex information, error if it does.
        let codex_resume = resume.map(CodexResumeInfo::try_from).transpose()?;

        // Mirror Claude harness behavior: prepend the resumption preamble and server context
        // to the user-turn prompt so codex treats it as immediate intent.
        // Order: resumption_prompt → context → prompt
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
        let client: Arc<dyn HarnessSupportClient> = server_api;
        Ok(Box::new(CodexHarnessRunner::new(
            self.cli_agent().command_prefix(),
            &owned_prompt,
            system_prompt,
            working_dir,
            client,
            terminal_driver,
            codex_resume,
        )?))
    }
}

/// Build the shell command that launches the Codex TUI.
///
/// `--dangerously-bypass-approvals-and-sandbox` disables both the sandbox and approval
/// prompts so the agent can run autonomously.
/// `--dangerously-bypass-hook-trust` allows the orchestration plugin hooks installed by
/// Warp to run without a manual hook review in unattended driver sessions. Driver setup
/// verifies the Codex platform plugin before launching commands with this flag.
/// `Some(session_id)` indicates that we want to resume that prior session. Unlike claude,
/// codex does not support assigning a session_id to a new conversation.
fn codex_command(cli_name: &str, session_id: Option<&Uuid>, prompt_path: &str) -> String {
    match session_id {
        Some(session_id) => format!(
            "{cli_name} resume --dangerously-bypass-approvals-and-sandbox {CODEX_BYPASS_HOOK_TRUST_FLAG} {session_id} \
             \"$(cat '{prompt_path}')\""
        ),
        None => {
            format!(
                "{cli_name} --dangerously-bypass-approvals-and-sandbox {CODEX_BYPASS_HOOK_TRUST_FLAG} \"$(cat '{prompt_path}')\""
            )
        }
    }
}

enum CodexRunnerState {
    Preexec,
    Running {
        conversation_id: AIConversationId,
        block_id: BlockId,
    },
}

struct CodexHarnessRunner {
    command: String,
    cli_name: String,
    /// Held so the temp file is cleaned up when the runner is dropped.
    _temp_prompt_file: NamedTempFile,
    client: Arc<dyn HarnessSupportClient>,
    terminal_driver: ModelHandle<TerminalDriver>,
    state: Mutex<CodexRunnerState>,
    /// Codex session UUID. Populated lazily by [`HarnessRunner::handle_session_update`]
    /// once the codex hooks emit `SessionStart`. Set once (using `OnceLock`).
    session_id: OnceLock<Uuid>,
    /// Path to the codex session rollout JSONL file. Populated by the first
    /// successful [`find_session_file`] walk so that subsequent saves skip the YYYY/MM/DD
    /// directory walk and read the JSONL file directly.
    transcript_path: OnceLock<PathBuf>,
    /// Optionally supply an existing conversation ID.
    preexisting_conversation_id: Option<AIConversationId>,
}

impl CodexHarnessRunner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        cli_command: &str,
        prompt: &str,
        _system_prompt: Option<&str>,
        _working_dir: &Path,
        client: Arc<dyn HarnessSupportClient>,
        terminal_driver: ModelHandle<TerminalDriver>,
        resume: Option<CodexResumeInfo>,
    ) -> Result<Self, AgentDriverError> {
        let temp_file = write_temp_file("oz_prompt_", prompt, ".txt")?;
        let prompt_path = temp_file.path().display().to_string();

        let (session_id, preexisting_conversation_id, transcript_path) = match resume {
            Some(CodexResumeInfo {
                conversation_id,
                session_id,
                mut envelope,
            }) => {
                let continuation = rehydrate_codex_transcript(&mut envelope, _working_dir)
                    .map_err(AgentDriverError::ConfigBuildFailed)?;
                (
                    Some(session_id),
                    Some(conversation_id),
                    Some(continuation.transcript_path),
                )
            }
            None => (None, None, None),
        };

        let command = codex_command(cli_command, session_id.as_ref(), &prompt_path);

        let session_id_cell: OnceLock<Uuid> = OnceLock::new();
        if let Some(id) = session_id {
            let _ = session_id_cell.set(id);
        }
        let transcript_path_cell: OnceLock<PathBuf> = OnceLock::new();
        if let Some(p) = transcript_path {
            let _ = transcript_path_cell.set(p);
        }

        Ok(Self {
            command,
            cli_name: cli_command.to_string(),
            _temp_prompt_file: temp_file,
            client,
            terminal_driver,
            state: Mutex::new(CodexRunnerState::Preexec),
            session_id: session_id_cell,
            transcript_path: transcript_path_cell,
            preexisting_conversation_id,
        })
    }

    /// Return the filepath for the session transcript, walking the codex sessions tree to find it on the
    /// first save call.
    async fn resolve_transcript_path(&self) -> Option<PathBuf> {
        if let Some(cached) = self.transcript_path.get() {
            return Some(cached.clone());
        }
        let session_id = self.session_id.get().copied()?;
        let resolved = tokio::task::spawn_blocking(move || -> Option<PathBuf> {
            let root = codex_sessions_root().ok()?;
            find_session_file(&root, session_id)
        })
        .await
        .ok()
        .flatten()?;
        let _ = self.transcript_path.set(resolved.clone());
        Some(resolved)
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for CodexHarnessRunner {
    fn harness_name(&self) -> &str {
        &self.cli_name
    }

    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<CommandHandle, AgentDriverError> {
        // Resume runs reuse the prior server conversation id; fresh runs mint a new one.
        let conversation_id = match self.preexisting_conversation_id {
            Some(id) => {
                log::info!("Resuming external conversation {id}");
                id
            }
            None => {
                let id = setup_events
                    .record_result(SetupStep::ThirdPartyHarnessExternalConversation, async {
                        self.client
                            .create_external_conversation(CODEX_CLI_FORMAT)
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

        let command = self.command.clone();
        let terminal_driver = self.terminal_driver.clone();
        let command_handle = foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.execute_command(&command, ctx))
            })
            .await??
            .await?;

        *self.state.lock() = CodexRunnerState::Running {
            conversation_id,
            block_id: command_handle.block_id().clone(),
        };

        setup_events
            .post_timeline_event(OzRunTimelineEvent::AgentStarted)
            .await;

        Ok(command_handle)
    }

    async fn exit(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        log::info!("Sending /exit to Codex CLI");
        let terminal_driver = self.terminal_driver.clone();
        foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| {
                    driver.send_text_to_cli(CODEX_EXIT_COMMAND.to_string(), ctx);
                });
            })
            .await
            .map_err(|_| anyhow::anyhow!("Agent driver dropped while sending /exit"))
    }

    /// Capture the codex session ID from the `SessionStart` event picked up by the `CLIAgentSessionsModel`.
    ///
    /// Relies on codex hooks being set up to emit this event correctly.
    async fn handle_session_update(&self, foreground: &ModelSpawner<AgentDriver>) -> Result<()> {
        if self.session_id.get().is_some() {
            return Ok(());
        }
        let terminal_driver = self.terminal_driver.clone();
        let session_id_str = foreground
            .spawn(move |_, ctx| {
                let terminal_view_id = terminal_driver.as_ref(ctx).terminal_view().id();
                CLIAgentSessionsModel::handle(ctx)
                    .as_ref(ctx)
                    .session(terminal_view_id)
                    .and_then(|s| s.session_context.session_id.clone())
            })
            .await
            .ok()
            .flatten();
        let Some(session_id_str) = session_id_str else {
            return Ok(());
        };
        match Uuid::parse_str(&session_id_str) {
            Ok(uuid) => {
                log::info!("Captured codex session id {uuid}");
                let _ = self.session_id.set(uuid);
            }
            Err(e) => log::warn!("Failed to parse codex session id '{session_id_str}': {e}"),
        }
        Ok(())
    }

    async fn save_conversation(
        &self,
        save_point: SavePoint,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        if matches!(save_point, SavePoint::Periodic)
            && !super::has_running_cli_agent(&self.terminal_driver, foreground).await
        {
            log::debug!("Will not save conversation, Codex not in progress");
            return Ok(());
        }

        let (conversation_id, block_id) = match &*self.state.lock() {
            CodexRunnerState::Preexec => {
                log::warn!("save_conversation called before start");
                return Ok(());
            }
            CodexRunnerState::Running {
                conversation_id,
                block_id,
            } => (*conversation_id, block_id.clone()),
        };

        let session_id = self.session_id.get().copied();
        let rollout_path = self.resolve_transcript_path().await;
        let client = self.client.as_ref();

        let is_final = matches!(save_point, SavePoint::Final);
        futures::try_join!(
            super::upload_current_block_snapshot(
                foreground,
                &self.terminal_driver,
                client,
                conversation_id,
                block_id,
            ),
            upload_transcript(client, conversation_id, session_id, rollout_path, is_final),
        )?;
        Ok(())
    }
}

/// Upload the codex session transcript to the server. No-ops if the session UUID hasn't
/// been captured yet or no rollout file is on disk yet.
async fn upload_transcript(
    client: &dyn HarnessSupportClient,
    conversation_id: AIConversationId,
    session_id: Option<Uuid>,
    transcript_path: Option<PathBuf>,
    is_final: bool,
) -> Result<()> {
    let Some(session_id) = session_id else {
        if is_final {
            log::warn!(
                "Codex session id still unknown at final save; transcript was never uploaded"
            );
        } else {
            log::debug!("Codex session id not yet known; skipping transcript upload");
        }
        return Ok(());
    };
    let Some(transcript_path) = transcript_path else {
        if is_final {
            log::warn!(
                "No codex rollout file found at final save for session {session_id}; transcript was never uploaded"
            );
        } else {
            log::debug!("No codex rollout file yet for session {session_id}");
        }
        return Ok(());
    };
    log::info!("Uploading codex transcript to conversation {conversation_id}");

    let body = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let entries = read_jsonl(&transcript_path)?;
        let metadata = parse_session_meta(entries.first()).unwrap_or_default();
        let envelope = CodexTranscriptEnvelope::new(session_id, metadata, entries);
        serde_json::to_vec(&envelope).context("Failed to serialize codex transcript")
    })
    .await
    .context("read_envelope task panicked")??;

    let target = client
        .get_transcript_upload_target(&conversation_id)
        .await
        .with_context(|| format!("Failed to get transcript upload target for {conversation_id}"))?;
    upload_to_target(client.http_client(), &target, body).await?;
    Ok(())
}

const CODEX_CONFIG_DIR: &str = ".codex";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const CODEX_AGENTS_OVERRIDE_FILE_NAME: &str = "AGENTS.override.md";
const CODEX_AUTH_FILE_NAME: &str = "auth.json";
const CODEX_CONFIG_TOML_FILE_NAME: &str = "config.toml";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
const CODEX_AUTH_MODE_API_KEY: &str = "apikey";
/// Lowercase string Codex's `TrustLevel` enum serializes to (codex
/// `protocol/src/config_types.rs::TrustLevel`).
const CODEX_TRUST_LEVEL_TRUSTED: &str = "trusted";
/// Top-level config key codex reads to override the built-in `openai` provider's base URL
/// (codex `core/src/config/mod.rs`).
const CODEX_OPENAI_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_CHECK_FOR_UPDATE_ON_STARTUP_KEY: &str = "check_for_update_on_startup";
const CODEX_MODEL_KEY: &str = "model";
const CODEX_MODEL_REASONING_EFFORT_KEY: &str = "model_reasoning_effort";
/// Target model for the `[notice.model_migrations]` table that suppresses Codex's
/// "choose a newer model" upgrade prompt at session launch. We stamp this for any
/// pinned model id (even when it already matches the target) so the unattended
/// cloud run never blocks on the prompt.
///
/// TODO: Ideally, we would make this server-driven so we don't depend on a client
/// release to change this.
const CODEX_MODEL_MIGRATIONS_TARGET: &str = "gpt-5.4";
fn prepare_codex_environment_config(
    working_dir: &Path,
    system_prompt: Option<&str>,
    resolved_env_vars: &HashMap<OsString, OsString>,
    resolved_secrets: &HashMap<String, ManagedSecretValue>,
    resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
    third_party_harness_model_config: Option<&HarnessModelConfig>,
) -> Result<()> {
    let codex_dir = codex_config_dir()?;

    if let Some(prompt) = system_prompt {
        write_codex_agents_override(&codex_dir, prompt)?;
    }

    match resolve_openai_api_key(resolved_env_vars) {
        Some(api_key) => prepare_codex_auth(&codex_dir.join(CODEX_AUTH_FILE_NAME), &api_key)?,
        None => log::info!("No OPENAI_API_KEY available; skipping Codex auth.json seed"),
    }

    // Resolve the base URL directly from the typed OpenAI secret. This avoids
    // leaking base_url into the child process environment and ensures we only
    // apply it when the typed secret is the active API key source.
    let openai_base_url = resolve_openai_base_url_from_secret(resolved_secrets, resolved_env_vars);

    prepare_codex_config_toml(
        &codex_dir.join(CODEX_CONFIG_TOML_FILE_NAME),
        working_dir,
        resolved_mcp_servers,
        third_party_harness_model_config,
        openai_base_url.as_deref(),
    )?;
    Ok(())
}

fn codex_config_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var(CODEX_HOME_ENV) {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    dirs::home_dir()
        .map(|home| home.join(CODEX_CONFIG_DIR))
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
}

fn write_codex_agents_override(codex_dir: &Path, system_prompt: &str) -> Result<()> {
    fs::create_dir_all(codex_dir).with_context(|| {
        format!(
            "Failed to create Codex config dir at {}",
            codex_dir.display()
        )
    })?;

    // Note: this currently works because we are only doing this for cloud agents; if we enable
    // this for local runs we'll want to make sure we don't clobber any existing file overrides.
    let prompt_path = codex_dir.join(CODEX_AGENTS_OVERRIDE_FILE_NAME);
    fs::write(&prompt_path, system_prompt).with_context(|| {
        format!(
            "Failed to write Codex system prompt to {}",
            prompt_path.display()
        )
    })
}

/// Mirrors the subset of Codex's `AuthDotJson` (codex `login/src/auth/storage.rs`) that we
/// need to seed. Unknown fields (`tokens`, `last_refresh`, `agent_identity`, ...) are
/// preserved via `extra` so we don't clobber an existing login.
#[derive(Default, Deserialize, Serialize, Debug)]
struct CodexAuthDotJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_mode: Option<String>,
    #[serde(
        rename = "OPENAI_API_KEY",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    openai_api_key: Option<String>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn prepare_codex_auth(auth_path: &Path, api_key: &str) -> Result<()> {
    let mut auth: CodexAuthDotJson = read_json_file_or_default(auth_path)?;
    auth.openai_api_key = Some(api_key.to_owned());
    if auth.auth_mode.is_none() {
        auth.auth_mode = Some(CODEX_AUTH_MODE_API_KEY.to_owned());
    }
    write_codex_auth_json(auth_path, &auth)
}

/// Write Codex's `auth.json` with restrictive (0o600) permissions, mirroring how
/// codex sets up this file itself.
fn write_codex_auth_json(path: &Path, auth: &CodexAuthDotJson) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(auth).context("Failed to serialize Codex auth.json")?;

    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to open {} for writing", path.display()))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    #[cfg(not(unix))]
    fs::write(path, &bytes).with_context(|| format!("Failed to write {}", path.display()))?;

    Ok(())
}

/// Returns the OpenAI API key for Codex auth.
///
/// Checks the worker-injected process env first (not in the resolved map since
/// `build_secret_env_vars` skips env vars already present in the process env),
/// then falls back to the resolved secret env vars map.
fn resolve_openai_api_key(resolved_env_vars: &HashMap<OsString, OsString>) -> Option<String> {
    // Worker-injected process env wins.
    if let Ok(value) = std::env::var(OPENAI_API_KEY_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    // Otherwise use the resolved value from the secrets map.
    resolved_env_vars
        .get(OsStr::new(OPENAI_API_KEY_ENV))
        .and_then(|v| v.to_str())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Returns the OpenAI base URL from the typed secret, if applicable.
///
/// The base URL is only used when the typed `OpenaiApiKey` secret is the active
/// source of `OPENAI_API_KEY`. If a worker-injected process env already provides
/// the API key, the typed-secret base URL is not applied (the worker controls
/// both the key and endpoint).
fn resolve_openai_base_url_from_secret(
    secrets: &HashMap<String, ManagedSecretValue>,
    resolved_env_vars: &HashMap<OsString, OsString>,
) -> Option<String> {
    // If the worker already injected an API key, the typed secret lost
    // precedence — do not apply its base URL.
    if std::env::var(OPENAI_API_KEY_ENV)
        .ok()
        .is_some_and(|v| !v.trim().is_empty())
    {
        return None;
    }

    // Only apply when the resolved env vars actually contain OPENAI_API_KEY
    // from the typed secret (i.e. the secret was not skipped).
    resolved_env_vars.get(OsStr::new(OPENAI_API_KEY_ENV))?;

    secrets.values().find_map(|secret| match secret {
        ManagedSecretValue::OpenaiApiKey { base_url, .. } => base_url
            .as_ref()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        _ => None,
    })
}

/// Edit `~/.codex/config.toml` via `toml_edit` to seed the harness defaults
/// while preserving anything that might already exist there. We handle:
/// - project trust: for a working dir and all of its git repo subdirectories,
///   set the projects to `trusted`.
/// - base URL: when `openai_base_url` is provided (from the secret's `base_url`
///   field), write it to config.toml. When absent, skip the key entirely so
///   Codex uses the provider's default global endpoint.
/// - update checks: disable Codex's startup update prompt for unattended runs.
/// - model override: when a non-default harness model config is
///   supplied, write the top-level `model` key so Codex pins the chosen model
///   for new sessions.
fn prepare_codex_config_toml(
    config_toml_path: &Path,
    working_dir: &Path,
    resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
    third_party_harness_model_config: Option<&HarnessModelConfig>,
    openai_base_url: Option<&str>,
) -> Result<()> {
    let existing = match fs::read_to_string(config_toml_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(anyhow::Error::from(e).context(format!(
                "Failed to read Codex config.toml at {}",
                config_toml_path.display()
            )));
        }
    };
    let mut doc: toml_edit::DocumentMut = existing.parse().with_context(|| {
        format!(
            "Failed to parse Codex config.toml at {}",
            config_toml_path.display()
        )
    })?;

    // Only write openai_base_url when the secret specifies one.
    if let Some(url) = openai_base_url {
        set_codex_openai_base_url(&mut doc, url);
    }
    set_codex_check_for_update_on_startup(&mut doc, false);
    set_codex_model(&mut doc, third_party_harness_model_config);
    set_codex_model_reasoning_effort(&mut doc, third_party_harness_model_config);

    let canonical = working_dir.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize Codex working dir at {}",
            working_dir.display()
        )
    })?;
    let project_key = canonical.to_string_lossy().into_owned();
    set_codex_project_trust_level(&mut doc, &project_key, CODEX_TRUST_LEVEL_TRUSTED);

    // Codex's trust check is not recursive (see openai/codex#19426) -- since we
    // clone the git repos into workspace/ for cloud agents, we usually have git
    // repo children that we also want to trust.
    for child_repo in find_child_git_repos(&canonical) {
        let key = child_repo.to_string_lossy().into_owned();
        set_codex_project_trust_level(&mut doc, &key, CODEX_TRUST_LEVEL_TRUSTED);
    }

    write_codex_mcp_servers(&mut doc, resolved_mcp_servers);

    if let Some(parent) = config_toml_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create Codex config dir at {}", parent.display())
        })?;
    }
    fs::write(config_toml_path, doc.to_string()).with_context(|| {
        format!(
            "Failed to write Codex config.toml at {}",
            config_toml_path.display()
        )
    })
}

/// Set the top-level `openai_base_url` key, overwriting any existing value.
fn set_codex_openai_base_url(doc: &mut toml_edit::DocumentMut, base_url: &str) {
    doc[CODEX_OPENAI_BASE_URL_KEY] = toml_edit::value(base_url);
}

fn set_codex_check_for_update_on_startup(doc: &mut toml_edit::DocumentMut, enabled: bool) {
    doc[CODEX_CHECK_FOR_UPDATE_ON_STARTUP_KEY] = toml_edit::value(enabled);
}

fn set_codex_model_reasoning_effort(
    doc: &mut toml_edit::DocumentMut,
    third_party_harness_model_config: Option<&HarnessModelConfig>,
) {
    let Some(reasoning_level) = third_party_harness_model_config
        .and_then(|config| config.reasoning_level.as_deref())
        .filter(|level| !level.is_empty())
    else {
        doc.remove(CODEX_MODEL_REASONING_EFFORT_KEY);
        return;
    };
    doc[CODEX_MODEL_REASONING_EFFORT_KEY] = toml_edit::value(reasoning_level);
}

fn set_codex_model(
    doc: &mut toml_edit::DocumentMut,
    third_party_harness_model_config: Option<&HarnessModelConfig>,
) {
    let Some(model_id) = third_party_harness_model_config
        .map(|config| config.model_id.as_str())
        .filter(|id| !id.is_empty() && *id != "default")
    else {
        // No model specified or "default" selected — remove any pre-existing
        // key so Codex uses its own default.
        doc.remove(CODEX_MODEL_KEY);
        return;
    };
    doc[CODEX_MODEL_KEY] = toml_edit::value(model_id);

    // Codex's TUI prompts the user to upgrade older models on session launch even when
    // a `model` key has been pinned. Stamping a migration entry keyed on the chosen
    // model id suppresses that prompt for the unattended cloud run. We do this
    // unconditionally rather than enumerating a list of "old" models on the client:
    // mapping the migration target to itself (e.g. `gpt-5.4 = "gpt-5.4"`) is a no-op
    // for Codex, and keeping the client free of model-version knowledge means we
    // don't have to ship a client update every time Anthropic/OpenAI ages out a model.
    set_codex_model_migration(doc, model_id, CODEX_MODEL_MIGRATIONS_TARGET);
}

fn set_codex_model_migration(
    doc: &mut toml_edit::DocumentMut,
    from_model_id: &str,
    to_model_id: &str,
) {
    if !doc.contains_table("notice") {
        let mut notice_tbl = toml_edit::Table::new();
        notice_tbl.set_implicit(true);
        doc.insert("notice", toml_edit::Item::Table(notice_tbl));
    }
    let migrations_tbl = doc["notice"]
        .as_table_mut()
        .expect("notice table inserted above")
        .entry("model_migrations")
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .expect("model_migrations entry is a table");
    migrations_tbl.set_implicit(false);
    migrations_tbl[from_model_id] = toml_edit::value(to_model_id);
}

/// Return immediate subdirectories of `dir` that contain a `.git`.
fn find_child_git_repos(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_dir() && path.join(".git").exists()).then_some(path)
        })
        .collect()
}

/// Insert/update `[projects."<project_key>"] trust_level = <trust_level>`.
///
/// Codex itself always writes `projects` as an explicit table, so we don't
/// handle the inline-table form here.
fn set_codex_project_trust_level(
    doc: &mut toml_edit::DocumentMut,
    project_key: &str,
    trust_level: &str,
) {
    if !doc.contains_table("projects") {
        let mut projects_tbl = toml_edit::Table::new();
        projects_tbl.set_implicit(true);
        doc.insert("projects", toml_edit::Item::Table(projects_tbl));
    }
    let proj_tbl = doc["projects"]
        .as_table_mut()
        .expect("projects table inserted above")
        .entry(project_key)
        .or_insert_with(toml_edit::table)
        .as_table_mut()
        .expect("project entry is a table");
    proj_tbl.set_implicit(false);
    proj_tbl["trust_level"] = toml_edit::value(trust_level);
}

/// Write resolved MCP servers into `[mcp_servers.<name>]` sections in the Codex config.
fn write_codex_mcp_servers(
    doc: &mut toml_edit::DocumentMut,
    servers: &HashMap<String, JSONMCPServer>,
) {
    if servers.is_empty() {
        return;
    }
    if !doc.contains_table("mcp_servers") {
        let mut tbl = toml_edit::Table::new();
        tbl.set_implicit(true);
        doc.insert("mcp_servers", toml_edit::Item::Table(tbl));
    }
    let mcp_tbl = doc["mcp_servers"]
        .as_table_mut()
        .expect("mcp_servers table inserted above");

    for (name, server) in servers {
        let entry = mcp_tbl
            .entry(name)
            .or_insert_with(toml_edit::table)
            .as_table_mut()
            .expect("mcp_servers entry is a table");
        entry.set_implicit(false);

        match &server.transport_type {
            JSONTransportType::CLIServer {
                command,
                args,
                env,
                working_directory,
            } => {
                entry["command"] = toml_edit::value(command.as_str());
                if !args.is_empty() {
                    let mut arr = toml_edit::Array::new();
                    for arg in args {
                        arr.push(arg.as_str());
                    }
                    entry["args"] = toml_edit::value(arr);
                }
                if !env.is_empty() {
                    let mut env_tbl = toml_edit::InlineTable::new();
                    for (k, v) in env {
                        env_tbl.insert(k, v.as_str().into());
                    }
                    entry["env"] = toml_edit::value(env_tbl);
                }
                if let Some(cwd) = working_directory {
                    entry["cwd"] = toml_edit::value(cwd.as_str());
                }
            }
            JSONTransportType::SSEServer { url, headers } => {
                entry["url"] = toml_edit::value(url.as_str());
                if !headers.is_empty() {
                    let mut hdrs_tbl = toml_edit::InlineTable::new();
                    for (k, v) in headers {
                        hdrs_tbl.insert(k, v.as_str().into());
                    }
                    entry["http_headers"] = toml_edit::value(hdrs_tbl);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
