use serde::{Deserialize, Serialize};

use super::redaction::{RedactedText, RedactedValue, TruncationMetadata};
use super::{HookConfigSource, HookEventName, MAX_PAYLOAD_BYTES, PAYLOAD_SCHEMA_VERSION};

#[derive(Clone, Debug)]
pub(crate) struct HookPayloadContext {
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) conversation_id: String,
    pub(crate) cwd: String,
    pub(crate) model: String,
    pub(crate) permission_mode: String,
}

#[derive(Clone, Debug)]
pub(crate) struct HookPayloadTemplate {
    pub(crate) context: HookPayloadContext,
    pub(crate) event: HookEventFields,
}

impl HookPayloadTemplate {
    pub(crate) fn event_name(&self) -> HookEventName {
        self.event.event_name()
    }

    pub(crate) fn matcher_subject(&self) -> Option<&str> {
        self.event.matcher_subject()
    }

    pub(crate) fn serialize_for_source(
        &self,
        source: HookConfigSource,
    ) -> Result<Vec<u8>, PayloadError> {
        let payload = HookPayload {
            schema_version: PAYLOAD_SCHEMA_VERSION,
            hook_event_name: self.event.event_name(),
            session_id: &self.context.session_id,
            run_id: &self.context.run_id,
            conversation_id: &self.context.conversation_id,
            cwd: &self.context.cwd,
            hook_source: source,
            model: &self.context.model,
            permission_mode: &self.context.permission_mode,
            event: &self.event,
        };
        let bytes = serde_json::to_vec(&payload).map_err(PayloadError::Serialize)?;
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(PayloadError::Oversized(bytes.len()));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum HookEventFields {
    SessionStart {
        source: SessionStartSource,
    },
    SessionEnd {
        reason: SessionEndReason,
    },
    UserPromptSubmit {
        prompt: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt_truncation: Option<TruncationMetadata>,
    },
    Stop {
        turn_status: TurnStatus,
    },
    PreToolUse {
        tool_name: String,
        tool_use_id: String,
        tool_input: RedactedValue,
    },
    PostToolUse {
        tool_name: String,
        tool_use_id: String,
        tool_input: RedactedValue,
        tool_response: RedactedValue,
    },
    PreCompact {
        trigger: CompactTrigger,
    },
}

impl HookEventFields {
    pub(crate) fn user_prompt(prompt: RedactedText) -> Self {
        Self::UserPromptSubmit {
            prompt: prompt.value,
            prompt_truncation: prompt.truncation,
        }
    }

    pub(crate) const fn event_name(&self) -> HookEventName {
        match self {
            Self::SessionStart { .. } => HookEventName::SessionStart,
            Self::SessionEnd { .. } => HookEventName::SessionEnd,
            Self::UserPromptSubmit { .. } => HookEventName::UserPromptSubmit,
            Self::Stop { .. } => HookEventName::Stop,
            Self::PreToolUse { .. } => HookEventName::PreToolUse,
            Self::PostToolUse { .. } => HookEventName::PostToolUse,
            Self::PreCompact { .. } => HookEventName::PreCompact,
        }
    }

    pub(crate) fn matcher_subject(&self) -> Option<&str> {
        match self {
            Self::SessionStart { source } => Some(source.as_str()),
            Self::SessionEnd { reason } => Some(reason.as_str()),
            Self::PreToolUse { tool_name, .. } | Self::PostToolUse { tool_name, .. } => {
                Some(tool_name)
            }
            Self::PreCompact { trigger } => Some(trigger.as_str()),
            Self::UserPromptSubmit { .. } | Self::Stop { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionStartSource {
    Startup,
    Resume,
}

impl SessionStartSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Resume => "resume",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SessionEndReason {
    Completed,
    Failed,
    Cancelled,
    Shutdown,
}

impl SessionEndReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CompactTrigger {
    Auto,
    Manual,
}

impl CompactTrigger {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnStatus {
    Idle,
    Blocked,
    Failed,
    Completed,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PayloadError {
    #[error("failed to serialize hook payload: {0}")]
    Serialize(serde_json::Error),
    #[error("serialized hook payload is {0} bytes")]
    Oversized(usize),
}

#[derive(Serialize)]
struct HookPayload<'a> {
    schema_version: &'static str,
    hook_event_name: HookEventName,
    session_id: &'a str,
    run_id: &'a str,
    conversation_id: &'a str,
    cwd: &'a str,
    hook_source: HookConfigSource,
    model: &'a str,
    permission_mode: &'a str,
    #[serde(flatten)]
    event: &'a HookEventFields,
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
