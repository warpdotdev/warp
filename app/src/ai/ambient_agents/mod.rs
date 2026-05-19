use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::{
    AIAgentOutputStatus, CancellationReason, FinishedAIAgentOutput, RenderableAIError,
};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;
use uuid::{NonNilUuid, Uuid};

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
    if let ConversationStatus::Blocked { blocked_action } = conversation.status() {
        return Some(AmbientConversationStatus::Blocked {
            blocked_action: blocked_action.clone(),
        });
    }
    if let ConversationStatus::Error = conversation.status() {
        if let Some(error_message) = conversation.status_error_message() {
            return Some(AmbientConversationStatus::Error {
                error: RenderableAIError::Other {
                    error_message: error_message.to_string(),
                    will_attempt_resume: false,
                    waiting_for_network: false,
                },
            });
        }
    }

    if let Some(last_exchange) = conversation.root_task_exchanges().last() {
        if let AIAgentOutputStatus::Finished { finished_output } = &last_exchange.output_status {
            let status = match finished_output {
                FinishedAIAgentOutput::Cancelled { output: _, reason } => {
                    AmbientConversationStatus::Cancelled { reason: *reason }
                }
                FinishedAIAgentOutput::Error { output: _, error } => {
                    AmbientConversationStatus::Error {
                        error: error.clone(),
                    }
                }
                FinishedAIAgentOutput::Success { output: _ } => AmbientConversationStatus::Success,
            };
            return Some(status);
        }
    }

    None
}
