use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use shell_words::quote as shell_quote;
use uuid::Uuid;
use warp_cli::agent::Harness;

use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::agent_sdk::driver::harness::claude_code::prepare_claude_environment_config;
use crate::ai::agent_sdk::driver::harness::{
    HarnessKind, harness_kind, harness_model_env_vars,
    remove_claude_externally_managed_listener_env_vars,
};
use crate::ai::agent_sdk::{task_env_vars, validate_cli_installed};
use crate::ai::ambient_agents::task::{
    HarnessConfig, HarnessModelConfig, normalize_orchestrator_agent_name,
};
use crate::ai::ambient_agents::{AgentConfigSnapshot, AmbientAgentTaskId};
use crate::ai::local_harness_setup::local_harness_product_disabled_message;
use crate::server::server_api::ai::AIClient;
use crate::terminal::cli_agent_sessions::plugin_manager::{
    CliAgentPluginManager, plugin_manager_for,
};
use crate::terminal::shell::ShellType;

#[derive(Clone)]
pub(super) struct PreparedLocalHarnessLaunch {
    pub command: String,
    pub env_vars: HashMap<OsString, OsString>,
    pub run_id: String,
    pub task_id: AmbientAgentTaskId,
}

async fn ensure_local_claude_child_plugins(manager: &dyn CliAgentPluginManager) {
    // Most environments should follow the standard Claude plugin setup path so
    // hidden local children retain the same notification support as regular
    // Claude sessions. The exception is local marketplace override testing:
    // installing/updating the notification plugin re-adds the public
    // claude-code-warp marketplace, which clobbers a developer's local
    // claude-code-warp-internal override used for oz-harness-support testing.
    if !manager.has_local_marketplace_override() {
        let plugin_result = if manager.needs_update() {
            manager.update().await
        } else if !manager.is_installed() {
            manager.install().await
        } else {
            Ok(())
        };
        if let Err(error) = plugin_result {
            log::warn!("Claude notification plugin setup failed for child harness: {error}");
        }
    }

    let platform_plugin_result = if manager.platform_plugin_needs_update() {
        manager.update_platform_plugin().await
    } else if !manager.is_platform_plugin_installed() {
        manager.install_platform_plugin().await
    } else {
        Ok(())
    };
    if let Err(error) = platform_plugin_result {
        log::warn!("Claude platform plugin setup failed for child harness: {error}");
    }
}

pub(super) fn normalize_local_child_harness(harness_type: &str) -> Option<Harness> {
    Harness::parse_local_child_harness(harness_type)
}

pub(super) fn validate_local_harness_shell(shell_type: Option<ShellType>) -> Result<(), String> {
    match shell_type {
        Some(ShellType::Bash) | Some(ShellType::Zsh) | Some(ShellType::Fish) => Ok(()),
        Some(ShellType::PowerShell) => Err(
            "Local child harnesses currently require bash, zsh, or fish; PowerShell is not supported."
                .to_string(),
        ),
        None => Err(
            "Local child harnesses currently require a detected bash, zsh, or fish session."
                .to_string(),
        ),
    }
}

const LOCAL_CLAUDE_CHILD_ORCHESTRATION_INSTRUCTIONS: &str = r#"You are a local Claude Code child agent launched by a lead agent in Warp.

Coordinate with the lead agent through the Oz CLI messaging environment:
- Your run id is in OZ_RUN_ID.
- The lead agent id is in OZ_PARENT_RUN_ID.
- The Oz CLI command is in OZ_CLI.

If OZ_CLI, OZ_RUN_ID, or OZ_PARENT_RUN_ID is missing, report that blocker in your final response.
Do not use Claude Code Agent or SendMessage tools to contact the lead agent; use the Oz CLI commands below.
Do not ask to inspect help before messaging. The command shapes below are complete.

Send a message to the lead agent at start, when blocked, and when complete:
"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"
All four send arguments are required: --sender-run-id "$OZ_RUN_ID", --to "$OZ_PARENT_RUN_ID", --subject, and --body.
Do not pass "$OZ_PARENT_RUN_ID" as a positional argument to send.

After sending a message, and before ending or standing by, check recent inbox messages:
"$OZ_CLI" run message list "$OZ_RUN_ID" --limit 25

The plugin may already have read incoming messages while staging them, so do not rely on --unread.
If recent messages from "$OZ_PARENT_RUN_ID" are present and you have not handled them, read them and use the latest lead-agent mailbox message as task context:
"$OZ_CLI" run message read "$MESSAGE_ID"

If a surfaced message requires acknowledgement, mark it delivered:
"$OZ_CLI" run message mark-delivered "$MESSAGE_ID"
"#;

pub(super) fn local_claude_child_prompt(task_prompt: &str) -> String {
    format!(
        "{LOCAL_CLAUDE_CHILD_ORCHESTRATION_INSTRUCTIONS}\nTask:\n{}",
        task_prompt
    )
}
pub(super) fn build_local_claude_child_command(prompt: &str) -> String {
    let session_id = Uuid::new_v4();
    let quoted_prompt = shell_quote(prompt);
    // Local child harness panes are launched off-screen. We intentionally skip
    // Claude's own permission prompts here so the child can start unattended
    // instead of hanging on an approval UI the user cannot see in that hidden
    // pane.
    format!("claude --session-id {session_id} --dangerously-skip-permissions {quoted_prompt}")
}

pub(super) fn build_local_opencode_child_command(prompt: &str) -> String {
    let quoted_prompt = shell_quote(prompt);
    format!("opencode --prompt {quoted_prompt}")
}
pub(super) fn build_local_codex_child_command(prompt: &str) -> String {
    let quoted_prompt = shell_quote(prompt);
    format!("codex --dangerously-bypass-approvals-and-sandbox {quoted_prompt}")
}

pub(super) fn local_child_task_config(
    harness: Harness,
    agent_name: Option<String>,
) -> Option<AgentConfigSnapshot> {
    let agent_name = agent_name
        .as_deref()
        .and_then(normalize_orchestrator_agent_name);
    match harness {
        Harness::Oz | Harness::Unknown => None,
        Harness::Claude | Harness::OpenCode | Harness::Gemini | Harness::Codex => {
            Some(AgentConfigSnapshot {
                name: agent_name,
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare_local_harness_child_launch(
    prompt: String,
    harness_type: String,
    model_id: Option<String>,
    parent_run_id: Option<String>,
    agent_name: Option<String>,
    shell_type: Option<ShellType>,
    startup_directory: Option<PathBuf>,
    ai_client: Arc<dyn AIClient>,
) -> Result<PreparedLocalHarnessLaunch, String> {
    let harness_model_config =
        model_id
            .filter(|id| !id.is_empty())
            .map(|model_id| HarnessModelConfig {
                model_id,
                reasoning_level: None,
            });
    let Some(harness) = normalize_local_child_harness(&harness_type) else {
        let harness_name = harness_type.trim();
        return Err(if harness_name.is_empty() {
            "Local child harness type is missing.".to_string()
        } else {
            format!("Unsupported local child harness '{harness_name}'.")
        });
    };
    if let Some(message) = local_harness_product_disabled_message(harness) {
        return Err(message.to_string());
    }
    validate_local_harness_shell(shell_type)?;
    let command = match harness {
        Harness::Oz => unreachable!("normalize_local_child_harness filters out Oz"),
        Harness::Unknown => unreachable!("normalize_local_child_harness filters out Unknown"),
        Harness::Claude => {
            let working_dir = startup_directory
                .or_else(|| std::env::current_dir().ok())
                .ok_or_else(|| {
                    format!(
                        "Could not resolve a working directory for the local {} child.",
                        harness.display_name()
                    )
                })?;
            let HarnessKind::ThirdParty(third_party_harness) =
                harness_kind(harness).map_err(|error: AgentDriverError| error.to_string())?
            else {
                unreachable!("Claude resolves to a third-party harness")
            };
            third_party_harness
                .validate()
                .map_err(|error: AgentDriverError| error.to_string())?;
            // Local child harness panes inherit the user's existing local
            // auth/session state. We still prepare harness config files here,
            // but there are no Warp-managed secrets to materialize into the
            // hidden child pane.
            prepare_claude_environment_config(&working_dir, &HashMap::new())
                .map_err(|error| error.to_string())?;
            if let Some(manager) = plugin_manager_for(third_party_harness.cli_agent()) {
                ensure_local_claude_child_plugins(manager.as_ref()).await;
            }

            build_local_claude_child_command(&local_claude_child_prompt(&prompt))
        }
        Harness::Codex => {
            let HarnessKind::ThirdParty(third_party_harness) =
                harness_kind(harness).map_err(|error: AgentDriverError| error.to_string())?
            else {
                unreachable!("Codex resolves to a third-party harness")
            };
            third_party_harness
                .validate()
                .map_err(|error: AgentDriverError| error.to_string())?;

            // Local Codex child panes must rely on the user's existing local
            // auth/session state. Do not run the shared Codex environment prep
            // here: it can seed OPENAI_API_KEY into ~/.codex/auth.json and
            // rewrite ~/.codex/config.toml for the whole machine.
            build_local_codex_child_command(&prompt)
        }
        Harness::OpenCode => {
            validate_cli_installed("opencode", Some("https://opencode.ai/docs"))
                .map_err(|error: AgentDriverError| error.to_string())?;
            build_local_opencode_child_command(&prompt)
        }
        Harness::Gemini => unreachable!("normalize_local_child_harness filters out Gemini"),
    };

    let task_id = ai_client
        .create_agent_task(
            prompt.clone(),
            None,
            parent_run_id.clone(),
            local_child_task_config(harness, agent_name),
        )
        .await
        .map_err(|error| {
            format!(
                "Failed to create local {} child task: {error}",
                harness.display_name()
            )
        })?;

    let mut env_vars = task_env_vars(Some(&task_id), parent_run_id.as_deref(), harness);
    if harness == Harness::Claude {
        // Local Claude child panes are launched directly in hidden terminals,
        // not through AgentDriver's ClaudeHarnessRunner. Let the Claude plugin
        // manage its own listener instead of waiting for a non-existent
        // external MessageBridge.
        remove_claude_externally_managed_listener_env_vars(&mut env_vars);
    }
    // Propagate the selected model to Claude Code via ANTHROPIC_MODEL.
    // Codex local children never receive a model override — the UI
    // ensures model_id is empty for local Codex.
    env_vars.extend(harness_model_env_vars(
        harness,
        harness_model_config.as_ref(),
    ));

    Ok(PreparedLocalHarnessLaunch {
        command,
        env_vars,
        run_id: task_id.to_string(),
        task_id,
    })
}

#[cfg(test)]
#[path = "local_harness_launch_tests.rs"]
mod tests;
