//! Resume/continue command construction for CLI coding agents.
//!
//! Pure functions: given a [`CLIAgent`] and a validated session id, produce the
//! exact command line that re-enters that agent's session. Used by the project
//! rail's task rows (prefill, never auto-run) and by "Copy command".
//!
//! "Never auto-run" is a property of how the command is delivered, and the
//! obvious-looking delivery is the wrong one: `TerminalView::set_pending_command`
//! marks the input as a *pending command*, meaning "the user submitted this and
//! it has not reached the shell yet", which `execute_pending_command` then runs
//! on the next `BootstrapPrecmdDone` or `BlockCompleted`. Resuming used it and
//! consequently executed on its own — most visibly with a deferred shell, whose
//! bootstrap fires moments after the tab is opened. Use
//! `TerminalView::prefill_command`, which inserts the text and arms nothing.
//!
//! Safety contract:
//! - Session ids arrive from external sources (plugin hook JSON, rollout
//!   files, `~/.cursor/chats` metadata) and are untrusted. Every id must pass
//!   [`is_valid_session_id`] before a command is built; the builders return
//!   `None` otherwise rather than quoting their way around bad input.
//! - These commands are user-facing. They must never carry the headless
//!   driver's flags (`--dangerously-skip-permissions`, prompt redirects) that
//!   `agent_sdk`'s `claude_command` embeds.

use crate::terminal::cli_agent::CLIAgent;

/// Maximum accepted session-id length. Generous for UUIDs (36 chars) while
/// still bounding what can reach a command line.
const MAX_SESSION_ID_LEN: usize = 64;

/// Whether `id` is safe to embed in a resume command.
///
/// Accepts only `[A-Za-z0-9_-]{1,64}` — every agent in scope uses UUIDs or
/// UUID-like tokens, which this covers. Everything else (whitespace, shell
/// metacharacters, control characters, non-ASCII) is rejected outright: a
/// newline alone would submit the prefilled line the instant it lands in the
/// input, so validation, not quoting, is the primary defence.
pub fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SESSION_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The command that resumes a specific session of `agent`, or `None` when the
/// agent has no (verified) resume-by-id verb or the id fails validation.
///
/// Verbs verified against each CLI's own docs/source:
/// - Claude: `claude --resume <uuid>`
/// - Codex: `codex resume <id>` (no flag form)
/// - Cursor: `agent --resume <id>` — the id is mandatory here: a bare
///   `agent --resume` opens Cursor's own interactive picker instead.
/// - OpenCode: `opencode --session <id>`
///
/// Unsupported variants return `None` so callers render a non-resumable row
/// rather than guessing at syntax.
pub fn resume_command(agent: CLIAgent, session_id: &str) -> Option<String> {
    if !is_valid_session_id(session_id) {
        return None;
    }
    let prefix = agent.command_prefix();
    match agent {
        CLIAgent::Claude => Some(format!("{prefix} --resume {session_id}")),
        CLIAgent::Codex => Some(format!("{prefix} resume {session_id}")),
        CLIAgent::CursorCli => Some(format!("{prefix} --resume {session_id}")),
        CLIAgent::OpenCode => Some(format!("{prefix} --session {session_id}")),
        // Gemini has no resume support upstream yet; the rest have no
        // verified resume-by-id verb. Listed exhaustively so adding a
        // variant forces a decision here.
        // `WarpTui` is Warp's own TUI front-end rather than a third-party CLI
        // with a resume verb, so it has no command to prefill either.
        CLIAgent::WarpTui
        | CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::OhMyPi
        | CLIAgent::Auggie
        | CLIAgent::Goose
        | CLIAgent::Hermes
        | CLIAgent::Vibe
        | CLIAgent::Antigravity
        | CLIAgent::Unknown => None,
    }
}

/// The command that resumes `agent`'s most recent session in the current
/// directory, or `None` when the agent has no verified continue verb.
pub fn continue_command(agent: CLIAgent) -> Option<String> {
    let prefix = agent.command_prefix();
    match agent {
        CLIAgent::Claude => Some(format!("{prefix} --continue")),
        CLIAgent::CursorCli => Some(format!("{prefix} --continue")),
        CLIAgent::OpenCode => Some(format!("{prefix} --continue")),
        CLIAgent::WarpTui
        | CLIAgent::Codex
        | CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::OhMyPi
        | CLIAgent::Auggie
        | CLIAgent::Goose
        | CLIAgent::Hermes
        | CLIAgent::Vibe
        | CLIAgent::Antigravity
        | CLIAgent::Unknown => None,
    }
}

#[cfg(test)]
#[path = "cli_agent_resume_tests.rs"]
mod tests;
