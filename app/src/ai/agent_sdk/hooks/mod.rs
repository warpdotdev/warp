use std::fmt;

use serde::{Deserialize, Serialize};

pub(crate) mod config;
pub(crate) mod payload;
pub(crate) mod permissions;
pub(crate) mod redaction;
pub(crate) mod runtime;
pub(crate) mod trust;

pub(crate) const CONFIG_SCHEMA_VERSION: &str = "warp.oz_hooks.config.v1";
pub(crate) const PAYLOAD_SCHEMA_VERSION: &str = "warp.oz_hook.v1";
pub(crate) const MAX_CONFIG_BYTES: usize = 256 * 1024;
pub(crate) const MAX_HANDLERS_PER_FILE: usize = 64;
pub(crate) const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
pub(crate) const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TOOL_INPUT_BYTES: usize = 128 * 1024;
pub(crate) const MAX_TOOL_RESPONSE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_DENIAL_REASON_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) enum HookEventName {
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    Stop,
    PreToolUse,
    PostToolUse,
    PreCompact,
}

impl HookEventName {
    pub(crate) const ALL: [Self; 7] = [
        Self::SessionStart,
        Self::SessionEnd,
        Self::UserPromptSubmit,
        Self::Stop,
        Self::PreToolUse,
        Self::PostToolUse,
        Self::PreCompact,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::Stop => "Stop",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PreCompact => "PreCompact",
        }
    }

    pub(crate) const fn ignores_matcher(self) -> bool {
        matches!(self, Self::UserPromptSubmit | Self::Stop)
    }
}

impl fmt::Display for HookEventName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HookConfigSource {
    User,
    Project,
}

impl HookConfigSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FailureMode {
    #[default]
    Continue,
    Deny,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
