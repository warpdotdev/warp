//! Mode-dependent ghosted placeholder hints for the TUI prompt input.
//!
//! Content policy for the input's empty-buffer ghost text: keybinding
//! guidance whose entries adapt to the transcript state. The keys referenced
//! here are typed characters (`!`, `/`) or fixed navigation keys (`←`), not
//! remappable bindings, so the strings are static; binding-backed hints must
//! resolve their keystroke display through the live keymap instead (see
//! `crate::keybindings::plan_toggle_hint`).

/// Ghost text for an empty agent-mode input before the transcript has any
/// visible content. Per design review, the zero state teaches the entry
/// points instead of pitching an example prompt: slash commands, and the
/// conversation list (`←` opens it from an empty input).
pub(crate) const ZERO_STATE_AGENT_HINT: &str = "/ for commands • ← for conversations";

/// Ghost text for an empty agent-mode input once the transcript has content.
/// The design's `? for shortcuts` segment is intentionally omitted until the
/// TUI has a shortcuts menu to open.
pub(crate) const AGENT_HINT: &str = "Ask the agent anything • ! for shell mode • / for commands";

/// The agent-mode placeholder hint for the current transcript state.
pub(crate) fn agent_input_hint(transcript_is_empty: bool) -> &'static str {
    if transcript_is_empty {
        ZERO_STATE_AGENT_HINT
    } else {
        AGENT_HINT
    }
}

#[cfg(test)]
#[path = "input_hints_tests.rs"]
mod tests;
