use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use warp_cli::agent::Harness;
use warp_managed_secrets::ManagedSecretValue;
use warpui::{ModelHandle, ModelSpawner};

use crate::terminal::CLIAgent;

use super::super::terminal::{CommandHandle, TerminalDriver};
use super::super::{AgentDriver, AgentDriverError};
use super::json_utils::{read_json_file_or_default, write_json_file};
use super::{write_temp_file, HarnessRunner, SavePoint, ThirdPartyHarness};

pub(crate) struct GeminiHarness;

/// Slash command Gemini's TUI recognises as a graceful shutdown.
const GEMINI_EXIT_COMMAND: &str = "/quit";

#[async_trait]
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

    fn prepare_environment_config(
        &self,
        working_dir: &Path,
        system_prompt: Option<&str>,
        _secrets: &HashMap<String, ManagedSecretValue>,
    ) -> Result<(), AgentDriverError> {
        prepare_gemini_environment_config(working_dir, system_prompt).map_err(|error| {
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
        working_dir: &Path,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        Ok(Box::new(GeminiHarnessRunner::new(
            self.cli_agent().command_prefix(),
            prompt,
            system_prompt,
            working_dir,
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

struct GeminiHarnessRunner {
    command: String,
    /// Held so the temp file is cleaned up when the runner is dropped.
    _temp_prompt_file: NamedTempFile,
    terminal_driver: ModelHandle<TerminalDriver>,
}

impl GeminiHarnessRunner {
    fn new(
        cli_command: &str,
        prompt: &str,
        _system_prompt: Option<&str>,
        _working_dir: &Path,
        terminal_driver: ModelHandle<TerminalDriver>,
    ) -> Result<Self, AgentDriverError> {
        let temp_file = write_temp_file("gemini_prompt_", prompt)?;
        let prompt_path = temp_file.path().display().to_string();

        Ok(Self {
            command: gemini_command(cli_command, &prompt_path),
            _temp_prompt_file: temp_file,
            terminal_driver,
        })
    }
}

#[async_trait]
impl HarnessRunner for GeminiHarnessRunner {
    async fn start(
        &self,
        foreground: &ModelSpawner<AgentDriver>,
    ) -> Result<CommandHandle, AgentDriverError> {
        let command = self.command.clone();
        let terminal_driver = self.terminal_driver.clone();
        foreground
            .spawn(move |_, ctx| {
                terminal_driver.update(ctx, |driver, ctx| driver.execute_command(&command, ctx))
            })
            .await??
            .await
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

        log::debug!("Skipping hosted Gemini conversation save");
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
const GEMINI_SYSTEM_PROMPT_FILE_NAME: &str = "GEMINI_SYSTEM_PROMPT.md";
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
