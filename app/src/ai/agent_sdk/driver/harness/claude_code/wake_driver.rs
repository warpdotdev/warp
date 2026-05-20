use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::sync::Arc;

use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent_events::{AgentMessageEventMetadata, MessageHydrator};
use crate::ai::ambient_agents::{AmbientAgentTaskId, AmbientAgentTaskState};
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::harness_support::ResolvePromptRequest;
use crate::server::server_api::ServerApi;
use crate::terminal::CLIAgent;
use anyhow::{Context, Result};
use shell_words::quote as shell_quote;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_graphql::ai::AgentTaskState;

use super::super::claude_transcript::{
    claude_config_dir, write_envelope, write_session_index_entry, ClaudeTranscriptEnvelope,
};
use super::super::task_env_vars;
use super::parent_bridge::{
    ensure_parent_bridge_state_dir, parent_bridge_root, prime_parent_bridge_for_wake,
};
use super::{claude_command, prepare_claude_environment_config, ClaudeHarness};

const CLAUDE_WAKE_PROMPT: &str = "New lead-agent messages are available. Read the latest lead-agent updates and continue the task accordingly.";
pub(super) const CLAUDE_WAKE_PROMPT_FILE_NAME: &str = "wake-turn-prompt.txt";
const CLAUDE_WAKE_EXTERNALLY_MANAGED_LISTENER_ENV_VARS: &[&str] = &[
    "OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY",
    "OZ_PARENT_LISTENER_MANAGED_EXTERNALLY",
];

#[derive(Debug)]
pub(super) struct ClaudeWakeRemoteContext {
    pub(super) session_id: Uuid,
    pub(super) envelope: ClaudeTranscriptEnvelope,
    pub(super) wake_prompt: String,
}

struct ClaudeWakeCandidate {
    task_id: AmbientAgentTaskId,
    parent_run_id: Option<String>,
    working_dir: Option<PathBuf>,
}

impl ClaudeHarness {
    pub(crate) async fn wake_dormant_session(
        server_api: Arc<ServerApi>,
        conversation: AIConversation,
        parent_conversation: Option<AIConversation>,
        working_dir: Option<PathBuf>,
        wake_message: Option<AgentMessageEventMetadata>,
    ) -> Result<Option<String>> {
        let Some(candidate) =
            Self::local_wake_candidate(&conversation, parent_conversation.as_ref(), working_dir)
        else {
            return Ok(None);
        };
        let ClaudeWakeCandidate {
            task_id,
            parent_run_id,
            working_dir,
        } = candidate;

        let task = server_api.get_ambient_agent_task(&task_id).await?;
        let harness = task
            .agent_config_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.harness.as_ref())
            .map(|config| config.harness_type);
        log::info!(
            "Evaluating dormant Claude wake: task_id={task_id} server_task_state={:?} harness={harness:?}",
            task.state
        );
        if !is_local_wake_task_state_ready(task.state.clone()) || harness != Some(Harness::Claude) {
            log::info!(
                "Skipping dormant Claude wake: task_id={task_id} server_task_state={:?} harness={harness:?}",
                task.state
            );
            return Ok(None);
        }

        let remote = Self::fetch_local_wake_remote_context(task_id, server_api.clone()).await?;
        let command = Self::prepare_local_wake_command(
            server_api.clone(),
            task_id,
            parent_run_id,
            working_dir,
            remote,
            wake_message,
        )
        .await?;

        log::info!("Reopening dormant Claude task before wake command: task_id={task_id}");
        server_api
            .update_agent_task(task_id, Some(AgentTaskState::InProgress), None, None, None)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "Failed to reopen dormant Claude task {task_id} before wake: {err:#}"
                )
            })?;
        log::info!("Reopened dormant Claude task before wake command: task_id={task_id}");

        Ok(Some(command))
    }

    fn local_wake_candidate(
        conversation: &AIConversation,
        parent_conversation: Option<&AIConversation>,
        working_dir: Option<PathBuf>,
    ) -> Option<ClaudeWakeCandidate> {
        let conversation_id = conversation.id();
        if !matches!(conversation.status(), ConversationStatus::Success) {
            log::info!(
                "Skipping dormant Claude wake candidate: conversation_id={conversation_id:?} reason=not_success status={:?}",
                conversation.status()
            );
            return None;
        }
        if !conversation.is_child_agent_conversation() || conversation.is_remote_child() {
            log::info!(
                "Skipping dormant Claude wake candidate: conversation_id={conversation_id:?} reason=not_local_child is_child_agent_conversation={} is_remote_child={}",
                conversation.is_child_agent_conversation(),
                conversation.is_remote_child()
            );
            return None;
        }
        let Some(task_id) = conversation.task_id() else {
            log::info!(
                "Skipping dormant Claude wake candidate: conversation_id={conversation_id:?} reason=missing_task_id"
            );
            return None;
        };
        let parent_run_id = conversation
            .parent_agent_id()
            .map(str::to_owned)
            .or_else(|| parent_conversation.and_then(AIConversation::run_id));

        Some(ClaudeWakeCandidate {
            task_id,
            parent_run_id,
            working_dir,
        })
    }

    async fn fetch_local_wake_remote_context(
        task_id: AmbientAgentTaskId,
        server_api: Arc<ServerApi>,
    ) -> Result<ClaudeWakeRemoteContext> {
        let resolved = server_api
            .resolve_prompt_for_task(
                &task_id,
                ResolvePromptRequest {
                    skill: None,
                    attachments_dir: None,
                },
            )
            .await
            .with_context(|| format!("Failed to resolve Claude wake prompt for task {task_id}"))?;
        let bytes = server_api
            .fetch_transcript_for_task(&task_id)
            .await
            .with_context(|| format!("Failed to fetch Claude transcript for task {task_id}"))?;
        let envelope: ClaudeTranscriptEnvelope =
            serde_json::from_slice(&bytes).with_context(|| {
                format!("Failed to deserialize Claude transcript for wake task {task_id}")
            })?;
        let wake_prompt = match resolved.resumption_prompt {
            Some(resumption_prompt) if !resumption_prompt.is_empty() => {
                format!(
                    "{resumption_prompt}

{CLAUDE_WAKE_PROMPT}"
                )
            }
            _ => CLAUDE_WAKE_PROMPT.to_string(),
        };
        Ok(ClaudeWakeRemoteContext {
            session_id: envelope.uuid,
            envelope,
            wake_prompt,
        })
    }

    pub(super) async fn prepare_local_wake_command(
        server_api: Arc<ServerApi>,
        task_id: AmbientAgentTaskId,
        parent_run_id: Option<String>,
        working_dir: Option<PathBuf>,
        mut remote: ClaudeWakeRemoteContext,
        wake_message: Option<AgentMessageEventMetadata>,
    ) -> Result<String> {
        let working_dir = working_dir.unwrap_or_else(|| remote.envelope.cwd.clone());
        prepare_claude_environment_config(&working_dir, &HashMap::new())
            .context("Failed to prepare Claude environment for wake")?;

        remote.envelope.cwd = working_dir.clone();
        let config_root = claude_config_dir().context("Failed to resolve Claude config dir")?;
        write_envelope(&remote.envelope, &config_root)
            .context("Failed to rehydrate Claude transcript for wake")?;
        if let Err(error) = write_session_index_entry(remote.session_id, &working_dir, &config_root)
        {
            log::warn!("Failed to update Claude sessions-index.json for wake: {error:#}");
        }

        let state_dir = parent_bridge_root()?.join(remote.session_id.to_string());
        ensure_parent_bridge_state_dir(&state_dir)?;
        let hydrator = MessageHydrator::for_task(server_api, task_id);
        prime_parent_bridge_for_wake(&hydrator, &state_dir, wake_message.as_ref()).await?;
        let prompt_path = state_dir.join(CLAUDE_WAKE_PROMPT_FILE_NAME);
        std::fs::write(&prompt_path, remote.wake_prompt.as_bytes())
            .with_context(|| format!("Failed to write {}", prompt_path.display()))?;

        let command = claude_command(
            CLIAgent::Claude.command_prefix(),
            &remote.session_id,
            &prompt_path.display().to_string(),
            None,
            None,
            true,
        );
        let env_vars = local_wake_task_env_vars(Some(&task_id), parent_run_id.as_deref());

        Ok(prefix_command_with_env_vars(command, env_vars))
    }
}

fn local_wake_task_env_vars(
    task_id: Option<&AmbientAgentTaskId>,
    parent_run_id: Option<&str>,
) -> HashMap<OsString, OsString> {
    let mut env_vars = task_env_vars(task_id, parent_run_id, Harness::Claude);
    // The local wake command is executed directly in the existing child
    // terminal, not through `AgentDriver::run_harness`, so Warp does not start
    // `MessageBridge` for this resumed Claude process. Leave the listener in
    // the Claude plugin's self-managed mode; otherwise the hook waits for
    // state files that no managed bridge is producing and the wake message is
    // never surfaced to Claude.
    for env_name in CLAUDE_WAKE_EXTERNALLY_MANAGED_LISTENER_ENV_VARS {
        env_vars.remove(OsStr::new(env_name));
    }
    env_vars
}

fn is_local_wake_task_state_ready(state: AmbientAgentTaskState) -> bool {
    match state {
        AmbientAgentTaskState::Succeeded => true,
        // The local conversation status is already gated on `Success` before
        // this function is called. The server task update is fire-and-forget,
        // so it can still report `InProgress` for a short window after the
        // local Claude process has actually stopped. Treat that stale server
        // state as wakeable for local children.
        AmbientAgentTaskState::InProgress => true,
        AmbientAgentTaskState::Queued
        | AmbientAgentTaskState::Pending
        | AmbientAgentTaskState::Claimed
        | AmbientAgentTaskState::Failed
        | AmbientAgentTaskState::Error
        | AmbientAgentTaskState::Blocked
        | AmbientAgentTaskState::Cancelled
        | AmbientAgentTaskState::Unknown => false,
    }
}

fn prefix_command_with_env_vars(command: String, env_vars: HashMap<OsString, OsString>) -> String {
    if env_vars.is_empty() {
        return command;
    }

    let mut env_pairs = env_vars
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
    env_pairs.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));

    let assignments = env_pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={}", shell_quote(&value)))
        .collect::<Vec<_>>()
        .join(" ");

    format!("env {assignments} {command}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_wake_task_state_ready_allows_success_and_stale_in_progress() {
        assert!(is_local_wake_task_state_ready(
            AmbientAgentTaskState::Succeeded
        ));
        assert!(is_local_wake_task_state_ready(
            AmbientAgentTaskState::InProgress
        ));

        for state in [
            AmbientAgentTaskState::Queued,
            AmbientAgentTaskState::Pending,
            AmbientAgentTaskState::Claimed,
            AmbientAgentTaskState::Failed,
            AmbientAgentTaskState::Error,
            AmbientAgentTaskState::Blocked,
            AmbientAgentTaskState::Cancelled,
            AmbientAgentTaskState::Unknown,
        ] {
            assert!(!is_local_wake_task_state_ready(state));
        }
    }
}
