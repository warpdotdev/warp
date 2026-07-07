use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::{NonNilUuid, Uuid};

use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::{
    AIAgentOutputStatus, CancellationReason, FinishedAIAgentOutput, RenderableAIError,
};

pub mod github_auth_notifier;
pub mod github_auth_url;
pub mod scheduled;
pub mod spawn;
pub mod task;
pub mod telemetry;

pub use task::{
    cancel_task_silently, cancel_task_with_toast, AgentConfigSnapshot, AgentSource,
    AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskState, TaskStatusMessage,
};
pub const OUT_OF_CREDITS_TASK_FAILURE_MESSAGE: &str =
    "Out of credits. Upgrade your Warp plan to continue running cloud agents.";
pub const SERVER_OVERLOADED_TASK_FAILURE_MESSAGE: &str =
    "Warp is temporarily overloaded. Please try again shortly.";

#[derive(Debug, thiserror::Error)]
#[error("Invalid task ID: {0}")]
pub struct ParseAmbientAgentTaskIdError(#[from] uuid::Error);

/// A globally unique ID for an ambient agent task.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmbientAgentTaskId(NonNilUuid);

impl Display for AmbientAgentTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for AmbientAgentTaskId {
    type Err = ParseAmbientAgentTaskIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::try_parse(s)?;
        Ok(Self(NonNilUuid::try_from(uuid)?))
    }
}

impl From<AmbientAgentTaskId> for cynic::Id {
    fn from(id: AmbientAgentTaskId) -> Self {
        Self::new(id.to_string())
    }
}

/// High-level outcome of an ambient agent conversation.
#[derive(Clone, Debug)]
pub enum AmbientConversationStatus {
    Success,
    Error {
        error: RenderableAIError,
    },
    #[allow(dead_code)]
    Cancelled {
        reason: CancellationReason,
    },
    #[allow(dead_code)]
    Blocked {
        blocked_action: String,
    },
}

/// Derive an [`AmbientConversationStatus`] from the given conversation, if it has
/// reached a terminal state that we care about for ambient agents.
pub fn conversation_output_status_from_conversation(
    conversation: &AIConversation,
) -> Option<AmbientConversationStatus> {
    match conversation.status() {
        // A pending recovery is not a terminal outcome.
        ConversationStatus::TransientError => None,

        ConversationStatus::Blocked { blocked_action } => {
            Some(AmbientConversationStatus::Blocked {
                blocked_action: blocked_action.clone(),
            })
        }

        ConversationStatus::Error => {
            // Prefer the structured error on the last exchange: it carries the precise
            // error variant and rendering hints. Fall back to the conversation-level
            // `status_error` for out-of-band failures (e.g. shell exit) recorded
            // without an exchange. Both drive FAILED-vs-ERROR classification downstream.
            if let Some(AIAgentOutputStatus::Finished {
                finished_output: FinishedAIAgentOutput::Error { error, .. },
            }) = conversation
                .root_task_exchanges()
                .last()
                .map(|exchange| &exchange.output_status)
            {
                return Some(AmbientConversationStatus::Error {
                    error: error.clone(),
                });
            }
            if let Some(error) = conversation.status_error() {
                return Some(AmbientConversationStatus::Error {
                    error: error.clone(),
                });
            }
            // No structured error anywhere; fall back to whatever terminal outcome
            // the last exchange carries.
            terminal_status_from_last_exchange(conversation)
        }

        // `InProgress` and `WaitingForEvents` are not terminal, but we preserve the
        // existing behavior of reporting a terminal outcome whenever the last exchange
        // has already finished.
        ConversationStatus::InProgress
        | ConversationStatus::Success
        | ConversationStatus::Cancelled
        | ConversationStatus::WaitingForEvents => terminal_status_from_last_exchange(conversation),
    }
}

/// Derive a terminal [`AmbientConversationStatus`] from the conversation's last
/// exchange, if that exchange has finished.
fn terminal_status_from_last_exchange(
    conversation: &AIConversation,
) -> Option<AmbientConversationStatus> {
    let AIAgentOutputStatus::Finished { finished_output } =
        &conversation.root_task_exchanges().last()?.output_status
    else {
        return None;
    };
    Some(match finished_output {
        FinishedAIAgentOutput::Cancelled { reason, .. } => {
            AmbientConversationStatus::Cancelled { reason: *reason }
        }
        FinishedAIAgentOutput::Error { error, .. } => AmbientConversationStatus::Error {
            error: error.clone(),
        },
        FinishedAIAgentOutput::Success { .. } => AmbientConversationStatus::Success,
    })
}
