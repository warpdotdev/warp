//! OSC 777 lifecycle notification emitter for the Warp headless TUI.
//!
//! The TUI emits structured OSC 777 `notify` events to signal CLI-agent
//! session lifecycle transitions to the parent terminal (the Warp GUI). This
//! lets the GUI drive its Heads-up Agent (HoA) status display using the same
//! v1 schema the third-party plugin ecosystem uses (e.g. `claude-code-warp`).
//!
//! ## Sequence format
//! ```text
//! ESC ] 777 ; notify ; warp://cli-agent ; <json> BEL
//! ```
//!
//! ## JSON schema (v1)
//! ```json
//! {
//!   "v": 1,
//!   "agent": "warp-tui",
//!   "event": "<event>",
//!   "session_id": "<optional>",
//!   "cwd": "<optional>",
//!   "query": "<optional>",
//!   "tool_name": "<optional>",
//!   "summary": "<optional>",
//!   "error_type": "<optional>"
//! }
//! ```
//!
//! Events and their trigger points:
//! - `session_start` — emitted once when the TUI session view is created.
//! - `prompt_submit` — emitted when the user submits a prompt.
//! - `permission_request` — emitted when the agent is blocked on a tool-call
//!   permission confirmation.
//! - `question_asked` — emitted when the agent is blocked on an
//!   `AskUserQuestion` interaction.
//! - `permission_replied` — emitted when a blocked tool permission is answered
//!   and the conversation returns to `InProgress`.
//! - `tool_complete` — emitted when an `AskUserQuestion` interaction is answered.
//! - `stop` — emitted when the conversation reaches `Success`.
//! - `stop_failure` — emitted when the conversation reaches `Error` or
//!   `Cancelled`.

use std::io::{self, Write};

use warp_core::cli_agent_protocol::{CLI_AGENT_NOTIFICATION_SENTINEL, CLIAgentNotification};
use warp_terminal::model::escape_sequences::{C0, C1, tmux_passthrough};

/// The agent identifier embedded in every OSC 777 payload. Matches the entry
/// in `CLIAgent::WarpTui::command_prefixes()` so the GUI's v1 parser resolves
/// it to `CLIAgent::WarpTui`.
pub(crate) const WARP_TUI_AGENT_NAME: &str = "warp-tui";

/// Builds the complete OSC 777 byte sequence for a notification.
///
/// The returned string is the literal bytes to write to stdout, including
/// the leading ESC and the trailing BEL. Both `emit` and tests use this
/// function so they exercise exactly the same wire format.
pub(crate) fn build_sequence(n: &CLIAgentNotification) -> String {
    let json = build_json(n);
    // Format: ESC ] 777 ; notify ; <title> ; <body> BEL
    let osc = C1::to_utf8(C1::OSC);
    let bell = char::from(C0::BEL);
    format!(
        "{osc}777;notify;{};{json}{bell}",
        CLI_AGENT_NOTIFICATION_SENTINEL
    )
}

/// Emits an OSC 777 `notify` sequence to stdout.
///
/// This is a best-effort write: failures (e.g. broken pipe when stdout is not
/// a terminal) are silently ignored so the TUI never panics or stalls due to a
/// notification write.
///
/// When the TUI is running inside tmux (detected via the `$TMUX` env variable),
/// the sequence is automatically wrapped in a DCS passthrough so it reaches the
/// hosting Warp GUI rather than being swallowed by tmux.
pub(crate) fn emit(notification: CLIAgentNotification) {
    let sequence = build_sequence(&notification);
    let in_tmux = std::env::var_os("TMUX").is_some();
    let sequence = if in_tmux {
        tmux_passthrough(&sequence)
    } else {
        sequence
    };
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(sequence.as_bytes());
    let _ = stdout.flush();
}

/// Build the compact JSON payload for an OSC 777 notification.
///
/// Optional fields are omitted rather than serialized as `null` so the GUI's
/// `v1::parse` function and the `CLIAgentSessionContext` accumulation logic
/// only see populated fields.
fn build_json(n: &CLIAgentNotification) -> String {
    serde_json::to_string(n).expect("CLI-agent notification is serializable")
}

#[cfg(test)]
#[path = "osc_notifications_tests.rs"]
mod tests;
