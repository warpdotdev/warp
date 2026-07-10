pub mod settings;
mod stack;

use warpui::keymap::EditableBinding;
use warpui::AppContext;

pub use self::settings::UndoCloseSettings;
pub use self::stack::{UndoCloseStack, UndoCloseStackEvent};
use crate::util::bindings::CustomAction;
use crate::workspace::WorkspaceAction;

/// Register keybindings for undo close functionality.
pub fn init(ctx: &mut AppContext) {
    use warpui::keymap::macros::*;

    ctx.register_editable_bindings([EditableBinding::new(
        "app:reopen_closed_session",
        "Reopen closed session",
        // Trigger ReopenClosedSession on the active workspace when
        // the action is taken from the command palette.
        WorkspaceAction::ReopenClosedSession,
    )
    .with_custom_action(CustomAction::ReopenClosedSession)
    // Scope to the GUI `Workspace` context. This action only applies to the GUI
    // desktop app, and the shared app init registers it in the headless TUI
    // process too. Without a predicate it defaults to `Just(true)` (matches every
    // context), so its default keystroke (`ctrl-alt-t` / `cmd-shift-t`) leaks
    // into TUI keymap contexts and trips the TUI's debug-only cross-surface
    // binding validator, panicking the debug TUI at startup. TUI views never
    // carry the "Workspace" context, so this keeps GUI behavior identical while
    // excluding the TUI (mirrors the sibling `workspace:*` bindings).
    .with_context_predicate(id!("Workspace"))]);
}
