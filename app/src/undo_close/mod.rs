pub mod settings;
mod stack;

use black_ui::keymap::EditableBinding;
use black_ui::AppContext;

pub use self::settings::UndoCloseSettings;
pub use self::stack::{UndoCloseStack, UndoCloseStackEvent};
use crate::util::bindings::CustomAction;
use crate::workspace::WorkspaceAction;

/// Register keybindings for undo close functionality.
pub fn init(ctx: &mut AppContext) {
    ctx.register_editable_bindings([EditableBinding::new(
        "app:reopen_closed_session",
        "Reopen closed session",
        // Trigger ReopenClosedSession on the active workspace when
        // the action is taken from the command palette.
        WorkspaceAction::ReopenClosedSession,
    )
    .with_custom_action(CustomAction::ReopenClosedSession)]);
}
