//! Non-composer hint text rendered in the TUI session's input slot.

/// Ghosted hint row shown in the input's slot while a user-controlled
/// long-running command owns input (the input box itself stays hidden).
/// ctrl-c is included only when the transcript contains visible command
/// content; the zero state has nothing visible to interrupt.
pub(crate) fn long_running_command_hint(
    attach_key: Option<&str>,
    include_interrupt: bool,
) -> Option<String> {
    let mut hints = Vec::with_capacity(2);
    if let Some(key) = attach_key {
        hints.push(format!("{key} to use agent"));
    }
    if include_interrupt {
        hints.push("ctrl-c to interrupt".to_owned());
    }
    (!hints.is_empty()).then(|| hints.join(" • "))
}
