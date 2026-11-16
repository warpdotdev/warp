//! TUI conversation-resume lifecycle state.

use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::ServerConversationToken;

/// Carries the selected server token across application teardown.
#[derive(Clone, Default)]
pub(crate) struct TuiExitSummaryHandle(Rc<RefCell<Option<ServerConversationToken>>>);

impl TuiExitSummaryHandle {
    /// Replaces the token to print after the TUI exits.
    pub(crate) fn set_token(&self, token: Option<ServerConversationToken>) {
        *self.0.borrow_mut() = token;
    }

    /// Returns the selected token captured before teardown.
    pub(crate) fn token(&self) -> Option<ServerConversationToken> {
        self.0.borrow().clone()
    }
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
