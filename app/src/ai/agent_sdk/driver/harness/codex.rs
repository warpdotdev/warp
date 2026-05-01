use std::collections::HashMap;
use std::fs;
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

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::harness_support::HarnessSupportClient;
use crate::server::server_api::ServerApi;
use crate::terminal::model::block::BlockId;
use crate::terminal::CLIAgent;

use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::json_utils::read_json_file_or_default;
use super::{write_temp_file, HarnessRunner, ResumePayload, SavePoint, ThirdPartyHarness};

pub(crate) struct CodexHarness;

/// Format slug sent to the server when creating a Codex conversation.
const CODEX_CLI_FORMAT: &str = "codex_cli";
/// Slash command Codex's TUI recognises as a graceful shutdown.
const CODEX_EXIT_COMMAND: &str = "/exit";

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

    fn prepare_environment_config(
        &self,
        working_dir: &Path,
        system_prompt: Option<&str>,
        secrets: &HashMap<String, ManagedSecretValue>,
    ) -> Result<(), AgentDriverError> {
        prepare_codex_environment_config(working_dir, system_prompt, secrets).map_err(|error| {
            AgentDriverError::HarnessConfigSetupFailed {
                harness: self.cli_agent().command_prefix().to_owned(),
                error,
            }
        })
    }

    fn build_runner(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        _resumption_prompt: Option<&str>,
        working_dir: &Path,
        _task_id: Option<AmbientAgentTaskId>,
        server_api: Arc<ServerApi>,
        terminal_driver: ModelHandle<TerminalDriver>,
        _resume: Option<ResumePayload>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        // TODO(REMOTE-1503): support resume for Codex.
        let client: Arc<dyn HarnessSupportClient> = server_api;
        Ok(Box::new(CodexHarnessRunner::new(
            self.cli_agent().command_prefix(),
            prompt,
            system_prompt,
            working_dir,
            client,
            terminal_driver,
        )?))
    }
}

/// Build the shell command that launches the Codex TUI.
///
/// `--dangerously-bypass-approvals-and-sandbox` disables both the sandbox and approval
/// prompts so the agent can run autonomously.
fn codex_command(cli_name: &str, prompt_path: &str) -> String {
    format!("{cli_name} --dangerously-bypass-approvals-and-sandbox \"$(cat '{prompt_path}')\"")
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
    /// Held so the temp file is cleaned up when the runner is dropped.
    _temp_prompt_file: NamedTempFile,
    client: Arc<dyn HarnessSupportClient>,
    terminal_driver: ModelHandle<TerminalDriver>,
    state: Mutex<CodexRunnerState>,
}

impl CodexHarnessRunner {
    fn new(
        cli_command: &str,
        prompt: &str,
        _system_prompt: Option<&str>,
        _working_dir: &Path,
        client: Arc<dyn HarnessSupportClient>,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Self, AgentDriverError> {
        let temp_file = write_temp_file("oz_prompt_", prompt)?;
        let prompt_path = temp_file.path().display().to_string();

        Ok(Self {
            command: codex_command(cli_command, &prompt_path),
            _temp_prompt_file: temp_file,
            client,
            terminal_driver,
            state: Mutex::new(CodexRunnerState::Preexec),
        })
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for CodexHarnessRunner {
    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<CommandHandle, AgentDriverError> {
        let conversation_id = self
            .client
            .create_external_conversation(CODEX_CLI_FORMAT)
            .await
            .map_err(|e| {
                log::error!("Failed to create external conversation: {e}");
                AgentDriverError::ConfigBuildFailed(e)
            })?;
        log::info!("Created external conversation {conversation_id}");

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

        // TODO(REMOTE-1504) Also save the conversation transcript.
        super::upload_current_block_snapshot(
            foreground,
            &self.terminal_driver,
            self.client.as_ref(),
            conversation_id,
            block_id,
        )
        .await
    }
}

const CODEX_CONFIG_DIR: &str = ".codex";
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
/// US data-residency endpoint. Our OpenAI keys are issued under a US-residency project,
/// which rejects requests to the global host with `401 incorrect_hostname`.
/// TODO(REMOTE-1509): plumb a region-tagged auth secret instead of hardcoding the URL.
const CODEX_OPENAI_BASE_URL: &str = "https://us.api.openai.com/v1";

fn prepare_codex_environment_config(
    working_dir: &Path,
    system_prompt: Option<&str>,
    secrets: &HashMap<String, ManagedSecretValue>,
) -> Result<()> {
    let home_dir =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    let codex_dir = home_dir.join(CODEX_CONFIG_DIR);

    if let Some(prompt) = system_prompt {
        write_codex_agents_override(&codex_dir, prompt)?;
    }

    match resolve_openai_api_key(secrets) {
        Some(api_key) => prepare_codex_auth(&codex_dir.join(CODEX_AUTH_FILE_NAME), &api_key)?,
        None => log::info!("No OPENAI_API_KEY available; skipping Codex auth.json seed"),
    }

    prepare_codex_config_toml(&codex_dir.join(CODEX_CONFIG_TOML_FILE_NAME), working_dir)?;
    Ok(())
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
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to open {} for writing", path.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    #[cfg(not(unix))]
    fs::write(path, &bytes).with_context(|| format!("Failed to write {}", path.display()))?;

    Ok(())
}

/// Returns the OpenAI API key for Codex auth, preferring the `OPENAI_API_KEY` env
/// var so the seeded `auth.json` matches the credential the launched Codex process
/// will see. [`AgentDriver::new`] skips a managed `OPENAI_API_KEY` secret when the
/// env var is already set, so we mirror that precedence here.
fn resolve_openai_api_key(secrets: &HashMap<String, ManagedSecretValue>) -> Option<String> {
    if let Ok(value) = std::env::var(OPENAI_API_KEY_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    if let Some(ManagedSecretValue::RawValue { value }) = secrets.get(OPENAI_API_KEY_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

/// Edit `~/.codex/config.toml` via `toml_edit` to seed the harness defaults
/// while preserving anything that might already exist there. We handle:
/// - project trust: for a working dir and all of its git repo subdirectories,
///   set the projects to `trusted`.
/// - base URL: set `openai_base_url = "<US data-residency endpoint>"` so we
///   hit the regional host our API keys require.
fn prepare_codex_config_toml(config_toml_path: &Path, working_dir: &Path) -> Result<()> {
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

    set_codex_openai_base_url(&mut doc, CODEX_OPENAI_BASE_URL);

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

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
