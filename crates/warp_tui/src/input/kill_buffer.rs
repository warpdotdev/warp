/// A single-entry kill buffer for the TUI input view.
///
/// Stores the last text killed by `Ctrl+K`, `Ctrl+U`, `Ctrl+W`, or `Alt+D`.
/// `Ctrl+Y` yanks (pastes) the stored text back into the input.
///
/// This is intentionally simple — a kill ring (multi-entry yank cycle) is
/// listed as a follow-up in `specs/tui-input-view/TECH.md`.
#[derive(Debug, Default)]
pub struct KillBuffer {
    content: String,
}

impl KillBuffer {
    /// Store `text` as the killed content, replacing any previous entry.
    pub fn kill(&mut self, text: impl Into<String>) {
        self.content = text.into();
    }

    /// Append `text` to the current kill buffer content.
    /// Used when multiple consecutive kills are combined (e.g. `Ctrl+K` at
    /// the end of one line followed immediately by another `Ctrl+K`).
    pub fn kill_append(&mut self, text: impl Into<String>) {
        self.content.push_str(&text.into());
    }

    /// Return the killed text for yanking, if any.
    pub fn yank(&self) -> Option<&str> {
        if self.content.is_empty() {
            None
        } else {
            Some(&self.content)
        }
    }

    /// Return whether the kill buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Clear the kill buffer.
    pub fn clear(&mut self) {
        self.content.clear();
    }
}
