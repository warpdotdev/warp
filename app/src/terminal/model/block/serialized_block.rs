use std::collections::HashSet;

use chrono::{DateTime, Local, TimeZone as _};
use serde::{Deserialize, Serialize};
use serde_bytes_repr::{ByteFmtDeserializer, ByteFmtSerializer};
use warp_core::command::ExitCode;

use super::AgentInteractionMetadata;
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::blocklist::block::cli_controller::LongRunningCommandControlState;
use crate::terminal::ShellHost;
use crate::terminal::model::BlockId;
use crate::terminal::model::block::{
    AgentViewVisibility, Block, BlockState, MAX_SERIALIZED_STYLIZED_OUTPUT_LINES, PromptInfo,
    has_block_failed,
};
use crate::terminal::model::session::SessionId;
use crate::util::extensions::TrimStringExt;

/// Serialization-stable representation of [`AgentViewVisibility`].
///
/// This type decouples the persisted format from the in-app format, allowing
/// internal changes to `AgentViewVisibility` without breaking serialization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SerializedAgentViewVisibility {
    Terminal {
        #[serde(default)]
        pending_conversation_ids: HashSet<AIConversationId>,
        conversation_ids: HashSet<AIConversationId>,
    },
    Agent {
        #[serde(alias = "conversation_id")]
        origin_conversation_id: AIConversationId,
        #[serde(default)]
        pending_other_conversation_ids: HashSet<AIConversationId>,
        #[serde(default)]
        other_conversation_ids: HashSet<AIConversationId>,
    },
}

impl From<AgentViewVisibility> for SerializedAgentViewVisibility {
    fn from(value: AgentViewVisibility) -> Self {
        match value {
            AgentViewVisibility::Terminal {
                pending_conversation_ids,
                conversation_ids,
            } => SerializedAgentViewVisibility::Terminal {
                pending_conversation_ids,
                conversation_ids,
            },
            AgentViewVisibility::Agent {
                origin_conversation_id,
                pending_other_conversation_ids,
                other_conversation_ids,
            } => SerializedAgentViewVisibility::Agent {
                origin_conversation_id,
                pending_other_conversation_ids,
                other_conversation_ids,
            },
        }
    }
}

impl From<SerializedAgentViewVisibility> for AgentViewVisibility {
    fn from(value: SerializedAgentViewVisibility) -> Self {
        match value {
            SerializedAgentViewVisibility::Terminal {
                pending_conversation_ids,
                conversation_ids,
            } => AgentViewVisibility::Terminal {
                pending_conversation_ids,
                conversation_ids,
            },
            SerializedAgentViewVisibility::Agent {
                origin_conversation_id,
                pending_other_conversation_ids,
                other_conversation_ids,
            } => AgentViewVisibility::Agent {
                origin_conversation_id,
                pending_other_conversation_ids,
                other_conversation_ids,
            },
        }
    }
}

fn default_as_true() -> bool {
    true
}

/// Blocklist AI metadata associated with this block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SerializedAIMetadata {
    /// The ID of the `AIAgentAction` associated with this block's requested command execution.
    /// This is optional because not all AI-related blocks are associated with a requested command.
    #[serde(alias = "action_id")]
    requested_command_action_id: Option<AIAgentActionId>,

    /// The ID of the conversation to which this action belongs.
    conversation_id: AIConversationId,

    subagent_task_id: Option<TaskId>,

    /// State governing user/agent interaction with the command in this block.
    long_running_control_state: Option<LongRunningCommandControlState>,

    /// `true` if the agent has previously written to this block.
    has_agent_written_to_block: bool,

    /// `true` if this block should be hidden from the user (as is the case with AI-requested
    /// commands, for example).
    #[serde(default = "default_as_true", skip)]
    should_hide_block: bool,
}

impl From<AgentInteractionMetadata> for SerializedAIMetadata {
    fn from(value: AgentInteractionMetadata) -> Self {
        SerializedAIMetadata {
            requested_command_action_id: value.requested_command_action_id().cloned(),
            conversation_id: *value.conversation_id(),
            subagent_task_id: value.subagent_task_id().cloned(),
            long_running_control_state: value.long_running_control_state().cloned(),
            has_agent_written_to_block: value.has_agent_written_to_block(),
            should_hide_block: value.should_hide_block(),
        }
    }
}

impl From<SerializedAIMetadata> for AgentInteractionMetadata {
    fn from(value: SerializedAIMetadata) -> Self {
        AgentInteractionMetadata::new(
            value.requested_command_action_id,
            value.conversation_id,
            value.subagent_task_id,
            value.long_running_control_state,
            value.has_agent_written_to_block,
            value.should_hide_block,
        )
    }
}

#[derive(Clone, Debug, Serialize, Default, Deserialize, PartialEq)]
pub struct SerializedBlock {
    pub id: BlockId,
    /// The input lines with their corresponding escape sequences so it can be rendered outside of
    /// the terminal.
    #[serde(with = "serde_bytes")]
    pub stylized_command: Vec<u8>,

    /// The output lines with their corresponding escape sequences so it can be rendered outside of
    /// the terminal.
    /// They are truncated to MAX_SERIALIZED_STYLIZED_OUTPUT_LINES lines.
    #[serde(with = "serde_bytes")]
    pub stylized_output: Vec<u8>,

    /// The current working directory of the block.
    pub pwd: Option<String>,

    #[serde(alias = "git_branch")]
    pub git_head: Option<String>,

    #[serde(default)]
    pub git_branch_name: Option<String>,

    pub virtual_env: Option<String>,

    pub conda_env: Option<String>,

    pub node_version: Option<String>,

    pub exit_code: ExitCode,

    /// True iff the block _started_ executing (i.e. preexec was received) or it's a static block.
    pub did_execute: bool,

    pub completed_ts: Option<DateTime<Local>>,

    pub start_ts: Option<DateTime<Local>>,

    pub ps1: Option<String>,

    pub rprompt: Option<String>,

    pub honor_ps1: bool,

    pub is_background: bool,

    pub session_id: Option<SessionId>,

    pub shell_host: Option<ShellHost>,

    /// JSON-serialized representation of the Warp prompt snapshot (Context Chips). Note that this
    /// is different from PS1 and RPROMPT1
    pub prompt_snapshot: Option<String>,

    /// JSON-serialized representation of [`SerializedAIMetadata`].
    pub ai_metadata: Option<String>,

    /// Whether this block was created locally (true) or remotely (false)
    #[serde(default)]
    pub is_local: Option<bool>,

    /// Tracks which views (terminal and/or agent conversations) this block should be visible in.
    #[serde(default)]
    pub agent_view_visibility: Option<SerializedAgentViewVisibility>,
}

impl SerializedBlock {
    /// Sets the command & output and `did_execute` to true.
    /// Everything else is a default value.
    #[cfg(any(test, feature = "test-util"))]
    pub fn new_for_test(stylized_command: Vec<u8>, stylized_output: Vec<u8>) -> SerializedBlock {
        SerializedBlock {
            stylized_command,
            stylized_output,
            did_execute: true,
            start_ts: Some(Local::now()),
            completed_ts: Some(Local::now()),
            ..Default::default()
        }
    }

    /// Sets only the command with no output, and `did_execute` to false.
    /// Everything else is a default value.
    #[cfg(test)]
    pub fn new_active_block_for_test() -> SerializedBlock {
        SerializedBlock::default()
    }

    /// Serialize this block to JSON bytes.
    ///
    /// The command and output contents are base64-encoded. This is *not* the default serde behavior,
    /// and blocks encoded this way must be deserialized with [`Self::from_json`].
    pub fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::new(&mut buf);
        let base64_config = base64::engine::GeneralPurposeConfig::new();
        let base64_ser =
            ByteFmtSerializer::base64(&mut ser, base64::alphabet::STANDARD, base64_config);
        self.serialize(base64_ser)
            .map_err(|e| anyhow::anyhow!("Failed to serialize block to JSON: {e}"))?;
        Ok(buf)
    }

    /// Deserialize a block from JSON bytes produced by [`Self::to_json`]
    /// or [`serde_json`].
    pub fn from_json(json: &[u8]) -> anyhow::Result<Self> {
        let mut de = serde_json::Deserializer::from_slice(json);
        let base64_config = base64::engine::GeneralPurposeConfig::new();
        let base64_de =
            ByteFmtDeserializer::new_base64(&mut de, base64::alphabet::STANDARD, base64_config);
        Self::deserialize(base64_de)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize block from JSON: {e}"))
    }

    pub fn has_failed(&self) -> bool {
        let block_state = match self.did_execute {
            true => BlockState::DoneWithExecution,
            false => BlockState::DoneWithNoExecution,
        };
        has_block_failed(self.exit_code, block_state)
    }
}

/// We should only be serializing a block that has finished.
impl From<&Block> for SerializedBlock {
    fn from(block: &Block) -> Self {
        let stylized_command = block
            .command_with_secrets_unobfuscated(true /*include_escape_sequences*/)
            .into_bytes();
        let stylized_output = block
            .output_grid()
            .contents_to_string_with_secrets_unobfuscated(
                true, /*include_escape_sequences*/
                Some(MAX_SERIALIZED_STYLIZED_OUTPUT_LINES),
            )
            .into_bytes();
        let ps1 =
            (!block.is_prompt_empty()).then(|| hex::encode(block.prompt_contents_to_string(true)));
        let rprompt = (!block.rprompt_grid().is_empty()).then(|| {
            let mut grid_content = block.rprompt_grid().contents_to_string(true, None);
            // We mustn't allow trailing newlines in the rprompt grid. This is b/c a trailing
            // newline will cause the Grid::max_cursor::column to go to 0. That is a problem b/c we
            // assume that the Grid::max_cursor::column is the length of the rprompt, and that
            // value is used to calculate the left-alignment position when painting the block.
            grid_content.trim_trailing_newline();
            hex::encode(grid_content)
        });
        let prompt_snapshot = block
            .prompt_snapshot
            .as_ref()
            .and_then(|prompt_snapshot| serde_json::to_string(prompt_snapshot).ok());
        let prompt_info = PromptInfo {
            pwd: block.pwd().map(String::from),
            git_branch: block.git_branch.clone(),
            git_branch_name: block.git_branch_name.clone(),
            virtual_env: block.virtual_env.clone(),
            conda_env: block.conda_env.clone(),
            node_version: block.node_version.clone(),
            ps1,
            rprompt,
            honor_ps1: block.honor_ps1(),
            prompt_snapshot,
        };

        let ai_metadata = block
            .agent_interaction_metadata()
            .cloned()
            .map(Into::<SerializedAIMetadata>::into)
            .and_then(|metadata| serde_json::to_string(&metadata).ok());

        SerializedBlock {
            id: block.id.clone(),
            stylized_command,
            stylized_output,
            pwd: prompt_info.pwd,
            git_head: prompt_info.git_branch,
            git_branch_name: prompt_info.git_branch_name,
            virtual_env: prompt_info.virtual_env,
            conda_env: prompt_info.conda_env,
            node_version: prompt_info.node_version,
            exit_code: block.exit_code,
            did_execute: block.state == BlockState::Executing
                || block.state == BlockState::DoneWithExecution
                || block.state == BlockState::Static,
            completed_ts: block.completed_ts,
            start_ts: block.start_ts,
            is_background: block.is_background(),
            ps1: prompt_info.ps1,
            rprompt: prompt_info.rprompt,
            honor_ps1: prompt_info.honor_ps1,
            session_id: block.session_id,
            shell_host: block.shell_host.clone(),
            prompt_snapshot: prompt_info.prompt_snapshot,
            ai_metadata,
            is_local: None,
            agent_view_visibility: Some(block.agent_view_visibility().clone().into()),
        }
    }
}

impl From<crate::persistence::model::Block> for SerializedBlock {
    fn from(block: crate::persistence::model::Block) -> Self {
        let exit_code = ExitCode::from(block.exit_code);
        Self {
            shell_host: ShellHost::try_from_persisted_block(&block),
            id: block.block_id.into(),
            stylized_command: block.stylized_command,
            stylized_output: block.stylized_output,
            pwd: block.pwd,
            git_head: block.git_branch,
            git_branch_name: block.git_branch_name,
            virtual_env: block.virtual_env,
            conda_env: block.conda_env,
            node_version: None, // Database does not store node_version yet
            exit_code,
            did_execute: block.did_execute,
            completed_ts: block
                .completed_ts
                .map(|naive_ts| Local.from_utc_datetime(&naive_ts)),
            start_ts: block
                .start_ts
                .map(|naive_ts| Local.from_utc_datetime(&naive_ts)),
            ps1: block.ps1,
            rprompt: block.rprompt,
            honor_ps1: block.honor_ps1,
            session_id: None,
            is_background: block.is_background,
            prompt_snapshot: block.prompt_snapshot,
            ai_metadata: block.ai_metadata,
            is_local: block.is_local,
            agent_view_visibility: block
                .agent_view_visibility
                .and_then(|json| serde_json::from_str(&json).ok()),
        }
    }
}

#[cfg(test)]
#[path = "serialized_block_tests.rs"]
mod tests;
