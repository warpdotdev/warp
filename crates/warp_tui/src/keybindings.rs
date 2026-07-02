//! TUI keybinding registration and cross-surface validation.
//!
//! Mirrors the GUI convention: each TUI view module exposes a top-level
//! `init(app)` that registers its keybindings, aggregated here and called once
//! at TUI startup (from [`crate::session`]'s mount). Fixed bindings are
//! reserved keys (ctrl-c); editable bindings are named `tui:*` so they are
//! user-remappable by name via `keybindings.yaml` (loading overrides in the
//! TUI process is a follow-up — the names registered here are the stable
//! contract).
//!
//! # Cross-surface isolation
//! GUI bindings cannot fire in the TUI even though the TUI process registers
//! them all: predicate-scoped bindings never match TUI keymap contexts, and
//! even a predicate-less binding dispatches an action type that no TUI view
//! handles, so the keystroke falls through to the element pass unharmed. The
//! debug-time validators below enforce the remaining convention: any
//! *keystroke* binding that matches a TUI view's context must be TUI-owned.
//! This catches GUI bindings registered without a context predicate — which
//! would otherwise match everywhere and, for multi-keystroke chords, swallow
//! prefix keys via a pending match.

use warpui_core::keymap::{BindingLens, IsBindingValid, Trigger};
use warpui_core::AppContext;

use crate::input::TuiInputView;
use crate::root_view::RootTuiView;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::transcript_view::TuiTranscriptView;

/// Group tag set on every TUI-registered binding. The validators treat it (or
/// a `tui:` name prefix) as proof of TUI ownership.
pub(crate) const TUI_BINDING_GROUP: &str = "tui";

/// Registers all TUI view keybindings and the cross-surface binding
/// validators. Called once at TUI startup, before the driver starts.
pub(crate) fn init(app: &mut AppContext) {
    crate::root_view::init(app);
    crate::terminal_session_view::init(app);
    crate::input::init(app);

    register_binding_validators(app);
}

/// Debug-time guard (no-op in release): every keystroke binding that matches a
/// TUI view's default keymap context must be TUI-owned.
fn register_binding_validators(app: &mut AppContext) {
    app.register_tui_binding_validator::<RootTuiView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiTerminalSessionView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiInputView>(is_tui_owned_binding);
    app.register_tui_binding_validator::<TuiTranscriptView>(is_tui_owned_binding);
}

fn is_tui_owned_binding(binding: BindingLens) -> IsBindingValid {
    // Non-keystroke triggers (palette-only `Empty`, `Standard`, `Custom`)
    // can never fire from TUI keyboard input, so they are exempt.
    if !matches!(binding.trigger, Trigger::Keystrokes(_)) {
        return IsBindingValid::Yes;
    }
    if is_tui_owned(binding.name, binding.group) {
        IsBindingValid::Yes
    } else {
        IsBindingValid::No
    }
}

/// Whether a binding's identity marks it as TUI-owned: a `tui:`-prefixed name
/// (editable bindings) or the [`TUI_BINDING_GROUP`] group (fixed bindings).
fn is_tui_owned(name: &str, group: Option<&str>) -> bool {
    name.starts_with("tui:") || group == Some(TUI_BINDING_GROUP)
}

#[cfg(test)]
#[path = "keybindings_tests.rs"]
mod tests;
