//! Mode-dependent ghosted placeholder hints for the TUI prompt input.
//!
//! Content policy for the input's empty-buffer ghost text: keybinding
//! guidance whose entries adapt to the transcript and orchestration state. The
//! keys referenced here are typed characters (`!`, `/`) or fixed navigation
//! keys (`←`, `Shift+↑`), not remappable bindings, so the strings are static;
//! binding-backed hints must resolve their keystroke display through the live
//! keymap instead (see `crate::keybindings::plan_toggle_hint`).

const ASK_AGENT_HINT: &str = "Ask the agent anything";
const ORCHESTRATION_HINT: &str = "Shift + ↑ for other agents";
const SHELL_MODE_HINT: &str = "! for shell mode";
const COMMANDS_HINT: &str = "/ for commands";
const CONVERSATIONS_HINT: &str = "← for conversations";
const HINT_SEPARATOR: &str = " • ";

/// Ghost text for an empty `!` shell-mode input: how to run and how to get
/// back to agent mode (esc is the input's contextual escape; backspace on the
/// empty input exits too).
pub(crate) const SHELL_HINT: &str = "Run a shell command • esc for agent mode";

/// Ghosted hint row shown in the input's slot while a user-controlled
/// long-running command owns input (the input box itself stays hidden).
/// ctrl-c is the reserved interrupt key in both the TUI keymap and the PTY.
pub(crate) const LONG_RUNNING_COMMAND_HINT: &str = "ctrl-c to interrupt";

/// The agent-mode placeholder hint for the current transcript and orchestration
/// state.
pub(crate) fn agent_input_hint(
    transcript_is_empty: bool,
    orchestration_tabs_available: bool,
) -> String {
    let mut hints = Vec::with_capacity(4);
    if transcript_is_empty {
        if orchestration_tabs_available {
            hints.push(ORCHESTRATION_HINT);
        }
        hints.extend([COMMANDS_HINT, CONVERSATIONS_HINT]);
    } else {
        hints.push(ASK_AGENT_HINT);
        if orchestration_tabs_available {
            hints.push(ORCHESTRATION_HINT);
        }
        hints.extend([SHELL_MODE_HINT, COMMANDS_HINT]);
    }
    hints.join(HINT_SEPARATOR)
}

#[cfg(test)]
#[path = "input_hints_tests.rs"]
mod tests;
