use std::fmt::Display;
use std::ops::Deref;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A globally unique ID for a conversation with an AI agent.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AIConversationId(Uuid);

impl Display for AIConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AIConversationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AIConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl TryFrom<String> for AIConversationId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(Uuid::try_parse(&value)?))
    }
}

/// A ID for an AI action generated as part of an `AIAgentOutput`.
///
/// The internal ID itself should be opaque to all callers. This ID may be relayed back to the AI with
/// the `AIAgentActionResult` from the action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AIAgentActionId(String);

impl From<String> for AIAgentActionId {
    fn from(value: String) -> Self {
        AIAgentActionId(value)
    }
}

impl From<AIAgentActionId> for String {
    fn from(value: AIAgentActionId) -> Self {
        value.0
    }
}

impl Display for AIAgentActionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(id: String) -> Self {
        TaskId(id)
    }
}

impl From<TaskId> for String {
    fn from(id: TaskId) -> Self {
        id.0
    }
}

impl Deref for TaskId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum EntrypointType {
    PromptSuggestion {
        is_static: bool,
        is_coding: bool,
    },
    ZeroStateAgentModePromptSuggestion,
    InitProjectRules,
    TriggerPassiveSuggestion {
        trigger: Option<PassiveSuggestionTriggerType>,
    },
    UserInitiated,
    AgentInitiated,
    SharedSession,
    CloneRepository,
    ResumeConversation,
}

impl EntrypointType {
    pub fn entrypoint(&self) -> String {
        match self {
            Self::PromptSuggestion {
                is_static,
                is_coding,
            } => match (is_static, is_coding) {
                (true, true) => "PROMPT_SUGGESTION.CODING_STATIC".to_string(),
                (true, false) => "PROMPT_SUGGESTION.STATIC".to_string(),
                (false, true) => "PROMPT_SUGGESTION.CODING".to_string(),
                (false, false) => "PROMPT_SUGGESTION.SIMPLE".to_string(),
            },
            Self::ZeroStateAgentModePromptSuggestion => {
                "ZERO_STATE_AGENT_MODE_PROMPT_SUGGESTION".to_string()
            }
            Self::InitProjectRules => "INIT_PROJECT_RULES".to_string(),
            Self::UserInitiated => "USER_INITIATED".to_string(),
            Self::AgentInitiated => "AGENT_INITIATED".to_string(),
            Self::TriggerPassiveSuggestion { trigger } => {
                let trigger_name = match trigger {
                    Some(PassiveSuggestionTriggerType::FilesChanged) => "FILES_CHANGED",
                    Some(PassiveSuggestionTriggerType::CommandRun) => "COMMAND_RUN",
                    Some(PassiveSuggestionTriggerType::ShellCommandCompleted) => {
                        "SHELL_COMMAND_COMPLETED"
                    }
                    Some(PassiveSuggestionTriggerType::AgentResponseCompleted) => {
                        "AGENT_RESPONSE_COMPLETED"
                    }
                    None => "NONE",
                };
                format!("TRIGGER_SUGGEST_PROMPT.{trigger_name}")
            }
            Self::CloneRepository => "CLONE_REPOSITORY".to_string(),
            Self::SharedSession => "SHARED_SESSION".to_string(),
            Self::ResumeConversation => "RESUME_CONVERSATION".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum PassiveSuggestionTriggerType {
    /// Used for unit test generation.
    FilesChanged,
    /// Used for unit test generation.
    CommandRun,

    ShellCommandCompleted,
    AgentResponseCompleted,
}
