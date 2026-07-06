//! Transient footer-hint state for the TUI.
//!
//! A short-lived notice (e.g. a rejected shell submission) that replaces the
//! footer's persistent hint for a fixed duration, then reverts. Kept free of
//! view/context dependencies so the generation-guarded state machine is
//! directly unit-testable, mirroring
//! [`ExitConfirmation`](crate::exit_confirmation::ExitConfirmation).

use std::time::Duration;

/// How long a transient footer hint stays visible before reverting to the
/// persistent content.
pub(crate) const TRANSIENT_HINT_DURATION: Duration = Duration::from_secs(3);

/// A transient footer notice guarded by a generation counter: each
/// [`Self::show`] supersedes earlier notices, so a deferred expiry only clears
/// the notice it was started for.
#[derive(Debug, Default)]
pub(crate) struct TransientHint {
    /// The currently displayed notice, if any.
    text: Option<String>,
    /// Incremented per notice; identifies which notice an expiry belongs to.
    generation: u64,
}

impl TransientHint {
    /// Displays `text`, superseding any current notice. Returns the new
    /// notice's generation for the caller's expiry timer to pass to
    /// [`Self::clear_expired`].
    pub(crate) fn show(&mut self, text: String) -> u64 {
        self.text = Some(text);
        self.generation += 1;
        self.generation
    }

    /// Clears the notice if `generation` is still the current one, returning
    /// whether it cleared. An expiry belonging to a superseded notice no-ops
    /// instead of clearing the newer notice early.
    pub(crate) fn clear_expired(&mut self, generation: u64) -> bool {
        if self.generation == generation && self.text.is_some() {
            self.text = None;
            true
        } else {
            false
        }
    }

    /// The currently displayed notice, if any.
    pub(crate) fn current(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

#[cfg(test)]
#[path = "transient_hint_tests.rs"]
mod tests;
