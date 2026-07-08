//! Transient footer-hint state for the TUI.
//!
//! A short-lived notice (e.g. a rejected shell submission) that replaces the
//! footer's persistent hint for a fixed duration, then reverts. Each new
//! notice aborts the superseded notice's expiry timer, so at most one live
//! expiry exists and it always belongs to the current notice — no
//! generation guard is needed. Owned by any view with a `TransientHint`
//! field; see [`TransientHint::show`].

use std::time::Duration;

use warpui_core::r#async::{SpawnedFutureHandle, Timer};
use warpui_core::{Entity, ViewContext};

/// How long a transient footer hint stays visible before reverting to the
/// persistent content.
pub(crate) const TRANSIENT_HINT_DURATION: Duration = Duration::from_secs(3);

/// A view-owned transient notice: [`Self::show`] displays a notice and spawns
/// its expiry timer, aborting the superseded notice's timer.
#[derive(Debug, Default)]
pub(crate) struct TransientHint {
    /// The currently displayed notice, if any.
    text: Option<String>,
    /// The pending expiry for the current notice; aborted when superseded.
    timer: Option<SpawnedFutureHandle>,
}

impl TransientHint {
    /// Displays `text` for [`TRANSIENT_HINT_DURATION`], then clears it back to
    /// no notice. Supersedes any current notice, aborting its expiry timer so
    /// the newer notice always gets its full duration. `transient_hint`
    /// projects this state back out of the owning view for the deferred
    /// expiry.
    pub(crate) fn show<V: Entity>(
        &mut self,
        text: String,
        ctx: &mut ViewContext<V>,
        transient_hint: impl Fn(&mut V) -> &mut TransientHint + 'static,
    ) {
        self.text = Some(text);
        let timer = ctx.spawn(
            Timer::after(TRANSIENT_HINT_DURATION),
            move |view, _, ctx| {
                let hint = transient_hint(view);
                hint.text = None;
                hint.timer = None;
                ctx.notify();
            },
        );
        self.set_timer(timer);
        ctx.notify();
    }

    /// Installs the expiry timer for the current notice, aborting any
    /// previously pending timer so a superseded expiry can never fire.
    fn set_timer(&mut self, timer: SpawnedFutureHandle) {
        if let Some(superseded_timer) = self.timer.replace(timer) {
            superseded_timer.abort();
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
