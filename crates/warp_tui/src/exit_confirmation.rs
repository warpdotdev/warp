//! Double-ctrl-c exit confirmation state for the TUI.
//!
//! One ctrl-c performs a contextual action (cancel the running conversation,
//! else clear the input) and arms this confirmation; a second ctrl-c while it
//! is armed exits the TUI. Kept free of view/context dependencies so the
//! timing state machine is directly unit-testable.

use std::time::Duration;

use instant::Instant;

/// How long after a ctrl-c press a second press exits the TUI. Matches the
/// GUI agent view's press-again-to-exit confirmation window.
pub(crate) const CTRL_C_EXIT_WINDOW: Duration = Duration::from_secs(1);

/// The armed/disarmed state of the press-ctrl-c-again-to-exit window.
#[derive(Debug, Default)]
pub(crate) struct ExitConfirmation {
    /// While armed, the instant the confirmation window lapses.
    expires_at: Option<Instant>,
}

impl ExitConfirmation {
    /// Whether the confirmation is armed (drives the footer hint).
    pub(crate) fn is_armed(&self) -> bool {
        self.expires_at.is_some()
    }

    /// Whether a ctrl-c pressed at `now` should exit: the confirmation is
    /// armed and its window has not lapsed.
    pub(crate) fn should_exit(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|expires_at| now < expires_at)
    }

    /// Arms (or re-arms) the window starting at `now`, returning the instant
    /// it expires so a deferred [`Self::disarm_expired`] can identify its own
    /// window.
    pub(crate) fn arm(&mut self, now: Instant) -> Instant {
        let expires_at = now + CTRL_C_EXIT_WINDOW;
        self.expires_at = Some(expires_at);
        expires_at
    }

    /// Disarms unconditionally, returning whether the confirmation was armed.
    pub(crate) fn disarm(&mut self) -> bool {
        self.expires_at.take().is_some()
    }

    /// Disarms the confirmation if the currently armed window is the one
    /// expiring at `window_expires_at`, returning whether it disarmed.
    ///
    /// Called by the deferred expiry timer, which identifies its own window by
    /// the expiry instant [`Self::arm`] returned: a timer belonging to a
    /// superseded (re-armed) window no longer matches and no-ops instead of
    /// clearing the newer window.
    pub(crate) fn disarm_expired(&mut self, window_expires_at: Instant) -> bool {
        if self.expires_at == Some(window_expires_at) {
            self.expires_at = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
#[path = "exit_confirmation_tests.rs"]
mod tests;
