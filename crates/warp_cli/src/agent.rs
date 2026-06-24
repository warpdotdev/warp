use std::fmt;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Output format for agent results.
#[derive(Debug, Copy, Clone, ValueEnum, Eq, PartialEq, Default)]
pub enum OutputFormat {
    /// Output as JSON.
    #[value(name = "json")]
    Json,
    /// Output as newline-delimited JSON.
    #[value(name = "ndjson")]
    Ndjson,
    /// Output as human-readable text.
    #[default]
    #[value(name = "pretty")]
    Pretty,
    /// Output as plain text.
    #[value(name = "text")]
    Text,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no values are skipped");
        f.write_str(value.get_name())
    }
}

/// The execution harness for an agent run.
#[derive(Debug, Copy, Clone, ValueEnum, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    /// Delegate to the `claude` CLI.
    #[value(name = "claude", alias = "claude-code")]
    Claude,
    /// Delegate to the `opencode` CLI.
    #[value(name = "opencode", alias = "open-code")]
    OpenCode,
    /// Delegate to the `gemini` CLI.
    #[value(name = "gemini")]
    Gemini,
    /// Delegate to the `codex` CLI.
    #[default]
    #[value(name = "codex")]
    Codex,
    /// A harness produced by a newer client/server that this client doesn't
    /// recognize. Surfaced via deserialization fallbacks (e.g. unknown GraphQL
    /// enum values, unknown `harness_type` strings); never selectable from the
    /// CLI or harness dropdown.
    #[serde(other)]
    #[value(skip)]
    Unknown,
}

impl Harness {
    pub fn parse_orchestration_harness(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        <Self as ValueEnum>::from_str(&normalized, true).ok()
    }

    pub fn parse_local_child_harness(value: &str) -> Option<Self> {
        match Self::parse_orchestration_harness(value) {
            Some(harness @ (Self::Claude | Self::OpenCode | Self::Codex)) => Some(harness),
            Some(Self::Gemini) | Some(Self::Unknown) | None => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::Gemini => "Gemini CLI",
            Self::Codex => "Codex",
            Self::Unknown => "Unknown",
        }
    }

    /// Parses a harness config-name string (the lowercase name written into
    /// `HarnessConfig::harness_type` by the spawner, e.g. `"claude"`, `"gemini"`, `"codex"`)
    /// into a [`Harness`] variant. Inverse of [`Harness::config_name`]. Returns `None` for
    /// unrecognized names so callers can distinguish a future-server harness from a
    /// round-tripped [`Harness::Unknown`]; callers that want to fall back to `Unknown`
    /// should `.unwrap_or(Harness::Unknown)`. UI surfaces should treat `Unknown` as a
    /// non-runnable harness.
    pub fn from_config_name(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Harness::Claude),
            "opencode" => Some(Harness::OpenCode),
            "gemini" => Some(Harness::Gemini),
            "codex" => Some(Harness::Codex),
            "unknown" => Some(Harness::Unknown),
            _ => None,
        }
    }

    /// Canonical config name for this harness (the lowercase string written into
    /// `HarnessConfig::harness_type`). Inverse of [`Harness::from_config_name`].
    /// The exhaustive match here forces every new [`Harness`] variant to declare a
    /// canonical name, which prevents `from_config_name` from silently falling back to
    /// `Unknown` when a new variant is added.
    pub fn config_name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::OpenCode => "opencode",
            Harness::Gemini => "gemini",
            Harness::Codex => "codex",
            Harness::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.config_name())
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
