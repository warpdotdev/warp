//! What a failed prompt turn means for the session.
//!
//! ACP gives an errored `session/prompt` no fatal or non-fatal meaning, and the adapters disagree
//! on what they reject a prompt for, so the reading rests on what every agent shares: whether its
//! connection is still open, and the protocol's code for a session it no longer has. What one
//! adapter is known to mean by an error refines that reading and never replaces it.
use warp_cli::agent::Harness;

use super::connection::RpcError;

/// The Codex adapter's code for its app-server exiting under a turn.
const CODEX_APP_SERVER_EXITED: i64 = 1001;

/// The Claude adapter's error kind for a turn that ended with tool calls left without a result.
const CLAUDE_INCOMPLETE_TOOL_CALL: &str = "incomplete_tool_call";

/// The prefix of Anthropic's ids for tool calls the API runs itself.
const SERVER_TOOL_CALL_ID_PREFIX: &str = "srvtoolu_";

/// Why a prompt turn failed, as far as the session is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnFailure {
    /// The turn failed; the agent can take another prompt.
    TurnFailed,
    /// The agent, or its session, is gone.
    SessionLost,
}

impl TurnFailure {
    /// A bounded label for metrics.
    pub(super) fn metric_label(self) -> &'static str {
        match self {
            Self::TurnFailed => "error_turn_failed",
            Self::SessionLost => "error_session_lost",
        }
    }
}

/// How the agent's error answer to a prompt is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PromptErrorReading {
    /// The agent rejected the prompt although the turn's work completed.
    Completed,
    Failed(TurnFailure),
}

/// Reads the error `harness`'s agent answered a prompt with. `agent_disconnected` is whether the
/// agent has closed its side of the connection.
pub(super) fn read_prompt_error(
    harness: Harness,
    error: &RpcError,
    agent_disconnected: bool,
) -> PromptErrorReading {
    if agent_disconnected || error.is_session_not_found() {
        return PromptErrorReading::Failed(TurnFailure::SessionLost);
    }
    let hint = match harness {
        Harness::Claude => claude_hint(error),
        Harness::Codex => codex_hint(error),
        Harness::Gemini | Harness::Pi | Harness::Oz | Harness::OpenCode | Harness::Unknown => None,
    };
    hint.unwrap_or(PromptErrorReading::Failed(TurnFailure::TurnFailed))
}

/// Classifies any error a prompt turn ended with, including ones the agent did not send.
pub(super) fn classify_turn_failure(
    harness: Harness,
    error: &anyhow::Error,
    agent_disconnected: bool,
) -> TurnFailure {
    let Some(rpc_error) = error.downcast_ref::<RpcError>() else {
        return if agent_disconnected {
            TurnFailure::SessionLost
        } else {
            TurnFailure::TurnFailed
        };
    };
    match read_prompt_error(harness, rpc_error, agent_disconnected) {
        PromptErrorReading::Failed(failure) => failure,
        PromptErrorReading::Completed => TurnFailure::TurnFailed,
    }
}

/// The adapter counts a tool call the API runs itself as one it owes a result for, and rejects the
/// prompt when the API never answered it, although nothing the turn did depended on that answer
/// (agentclientprotocol/claude-agent-acp#1486). A client tool call left without a result is a real
/// failure and is not read this way.
fn claude_hint(error: &RpcError) -> Option<PromptErrorReading> {
    if error.error_kind()? != CLAUDE_INCOMPLETE_TOOL_CALL {
        return None;
    }
    only_server_tool_calls(&error.message).then_some(PromptErrorReading::Completed)
}

fn codex_hint(error: &RpcError) -> Option<PromptErrorReading> {
    if error.code != CODEX_APP_SERVER_EXITED {
        return None;
    }
    let restartable = error.data.as_ref()?.get("restartable")?.as_bool()?;
    (!restartable).then_some(PromptErrorReading::Failed(TurnFailure::SessionLost))
}

/// Whether `message` ends in a non-empty, comma-separated list of tool call ids that are all
/// server tool calls. The adapter names the calls only in its message text.
fn only_server_tool_calls(message: &str) -> bool {
    let Some((_, ids)) = message.rsplit_once(": ") else {
        return false;
    };
    ids.split(',').map(str::trim).all(|id| {
        id.strip_prefix(SERVER_TOOL_CALL_ID_PREFIX)
            .is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
    })
}

#[cfg(test)]
#[path = "turn_failure_tests.rs"]
mod tests;
