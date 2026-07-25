//! Non-composer hint text rendered in the TUI session's input slot.

/// Ghosted hint row shown in the input's slot while a user-controlled
/// long-running command owns input (the input box itself stays hidden).
/// ctrl-c is the reserved interrupt key in both the TUI keymap and the PTY.
pub(crate) const LONG_RUNNING_COMMAND_HINT: &str = "ctrl-c to interrupt";
