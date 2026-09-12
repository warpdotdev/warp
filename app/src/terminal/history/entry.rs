use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use warp_core::command::ExitCode;
use warp_terminal::shell::ShellType;

use crate::server::ids::SyncId;
use crate::terminal::model::block::{
    AgentInteractionMetadata, Block, SerializedAIMetadata, SerializedBlock,
};
use crate::terminal::model::session::{Session, SessionId};

/// Data model for a history command persisted to sqlite, used as an intermediate representation
/// between the sqlite schema (sqlite::model::Command) and the `History` model.
#[derive(Debug)]
pub struct PersistedCommand {
    pub id: i32,
    pub command: String,
    pub exit_code: Option<ExitCode>,
    pub start_ts: Option<DateTime<Local>>,
    pub completed_ts: Option<DateTime<Local>>,
    pub pwd: Option<String>,
    pub shell_host: Option<ShellHost>,
    pub session_id: Option<SessionId>,
    pub git_branch: Option<String>,
    pub workflow_id: Option<SyncId>,
    pub workflow_command: Option<String>,
    pub is_agent_executed: bool,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub struct ShellHost {
    // This field was originally named `shell` so mark it as an alias for
    // backwards compatibility.
    #[serde(alias = "shell")]
    pub shell_type: ShellType,
    pub user: String,
    pub hostname: String,
}

impl ShellHost {
    pub fn from_session(session: &Session) -> Self {
        Self {
            shell_type: session.shell().shell_type(),
            user: session.user().to_owned(),
            hostname: session.hostname().to_owned(),
        }
    }

    #[cfg(test)]
    pub fn from_session_info(session_info: &crate::terminal::model::session::SessionInfo) -> Self {
        Self {
            shell_type: session_info.shell.shell_type(),
            user: session_info.user.clone(),
            hostname: session_info.hostname.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkedWorkflowData {
    /// The history entry is linked to a `CloudWorkflow` by its ID.
    Id(SyncId),

    /// The history entry is linked to a local `Workflow` by its command.
    ///
    /// Local workflows are not keyed by any common ID.
    Command(String),
}

/// For history entries coming from the shell history file, only the command is populated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub session_id: Option<SessionId>,
    pub command: String,
    pub pwd: Option<String>,
    pub start_ts: Option<DateTime<Local>>,
    pub completed_ts: Option<DateTime<Local>>,
    pub exit_code: Option<ExitCode>,
    pub git_head: Option<String>,
    pub shell_host: Option<ShellHost>,

    /// The ID of the `CloudWorkflow` used to construct this command.
    workflow_id: Option<SyncId>,

    /// The templated command contained in the `Workflow` used to construct the executed
    /// command.
    workflow_command: Option<String>,

    pub is_for_restored_block: bool,

    /// Whether this command was executed by an AI agent.
    pub is_agent_executed: bool,
}

fn serialized_block_is_agent_executed(block: &SerializedBlock) -> bool {
    let Some(ai_metadata) = block.ai_metadata.as_ref() else {
        return false;
    };

    serde_json::from_str::<SerializedAIMetadata>(ai_metadata)
        .ok()
        .map(AgentInteractionMetadata::from)
        .is_some_and(|metadata| metadata.requested_command_action_id().is_some())
}

impl HistoryEntry {
    pub fn command_only<S: Into<String>>(command: S) -> Self {
        Self {
            command: command.into(),
            session_id: None,
            pwd: None,
            start_ts: None,
            completed_ts: None,
            workflow_id: None,
            workflow_command: None,
            exit_code: None,
            git_head: None,
            shell_host: None,
            is_for_restored_block: false,
            is_agent_executed: false,
        }
    }

    pub fn command_at_time(
        command: String,
        start_ts: DateTime<Local>,
        session_id: Option<SessionId>,
        is_for_restored_block: bool,
    ) -> Self {
        let mut entry = Self::command_only(command);
        entry.start_ts = Some(start_ts);
        entry.session_id = session_id;
        entry.is_for_restored_block = is_for_restored_block;
        entry
    }

    pub fn for_session_command(
        command: String,
        active_block: &Block,
        session: &Session,
        workflow_id: Option<SyncId>,
        workflow_command: Option<String>,
        is_agent_executed: bool,
    ) -> Self {
        HistoryEntry {
            session_id: Some(session.id()),
            command,
            pwd: active_block.pwd().map(|pwd| pwd.to_owned()),
            start_ts: active_block.start_ts().copied(),
            workflow_id,
            workflow_command,
            git_head: active_block
                .git_branch()
                .map(|git_branch| git_branch.to_owned()),
            completed_ts: None,
            exit_code: None,
            shell_host: active_block.shell_host().clone(),
            is_for_restored_block: false,
            is_agent_executed,
        }
    }

    pub fn for_restored_block(command: String, block: &Block) -> Self {
        HistoryEntry {
            session_id: block.session_id(),
            command,
            pwd: block.pwd().map(|pwd| pwd.to_owned()),
            start_ts: block.start_ts().copied(),
            workflow_id: None,
            workflow_command: None,
            git_head: block.git_branch().map(|git_branch| git_branch.to_owned()),
            shell_host: block.shell_host().clone(),
            completed_ts: block.completed_ts().copied(),
            exit_code: Some(block.exit_code()),
            is_for_restored_block: true,
            is_agent_executed: block.requested_command_action_id().is_some(),
        }
    }

    pub fn for_completed_block(command: String, block: &SerializedBlock) -> Self {
        HistoryEntry {
            session_id: block.session_id,
            command,
            pwd: block.pwd.clone(),
            start_ts: block.start_ts,
            completed_ts: block.completed_ts,
            workflow_id: None,
            workflow_command: None,
            exit_code: Some(block.exit_code),
            git_head: block.git_head.clone(),
            shell_host: block.shell_host.clone(),
            is_for_restored_block: false,
            is_agent_executed: serialized_block_is_agent_executed(block),
        }
    }

    /// Indicates that at least one of the optional rich history fields is Some.
    pub fn has_metadata(&self) -> bool {
        // Destructure this so that we _must_ update this method when new metadata fields are added
        // to Self. `completed_ts` isn't useful without start_ts, so that is omitted in this check.
        let HistoryEntry {
            session_id: _,
            command: _,
            is_for_restored_block: _,
            is_agent_executed: _,
            pwd,
            start_ts,
            completed_ts: _,
            workflow_id,
            exit_code,
            git_head,
            workflow_command,
            shell_host: _,
        } = self;
        pwd.is_some()
            || start_ts.is_some()
            || workflow_id.is_some()
            || exit_code.is_some()
            || git_head.is_some()
            || workflow_command.is_some()
    }

    /// Returns `LinkedWorkflowData` referring to the workflow used to create this history command,
    /// if any.
    pub fn linked_workflow_data(&self) -> Option<LinkedWorkflowData> {
        match (&self.workflow_id, &self.workflow_command) {
            (Some(workflow_id), _) => Some(LinkedWorkflowData::Id(*workflow_id)),
            (_, Some(workflow_command)) => {
                Some(LinkedWorkflowData::Command(workflow_command.clone()))
            }
            _ => None,
        }
    }

    /// Sets the linked workflow references for this entry.
    #[cfg(test)]
    pub fn set_linked_workflow(
        &mut self,
        workflow_id: Option<SyncId>,
        workflow_command: Option<String>,
    ) {
        self.workflow_id = workflow_id;
        self.workflow_command = workflow_command;
    }
}

impl From<PersistedCommand> for HistoryEntry {
    fn from(command: PersistedCommand) -> Self {
        HistoryEntry {
            session_id: command.session_id,
            command: command.command,
            exit_code: command.exit_code,
            start_ts: command.start_ts,
            completed_ts: command.completed_ts,
            pwd: command.pwd,
            git_head: command.git_branch,
            workflow_id: command.workflow_id,
            workflow_command: command.workflow_command,
            shell_host: command.shell_host,
            is_for_restored_block: false,
            is_agent_executed: command.is_agent_executed,
        }
    }
}
