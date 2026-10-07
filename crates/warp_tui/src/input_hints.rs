//! Non-composer hint text rendered in the TUI session's input slot.

/// Ghosted hint row shown in the input's slot while a user-controlled
/// long-running command owns input (the input box itself stays hidden).
pub(crate) fn long_running_command_hint(attach_key: Option<&str>) -> Option<String> {
    attach_key.map(|key| format!("{key}  to use agent"))
}
