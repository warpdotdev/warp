//! Tracks whether an inline rename editor currently owns focus.
//!
//! Renaming a tab group opens an inline editor *and* spawns a terminal. Roughly a
//! second later that terminal's bootstrap block becomes visible and asks for focus,
//! which blurs the editor mid-typing. Blur used to be treated as confirmation, so the
//! fragment typed so far became the group's persisted name (issue #14241); blur now
//! discards the rename, and this singleton keeps the terminal from interrupting it in
//! the first place.
//!
//! `WorkspaceState` already knows a rename is in progress, but it lives on
//! `WorkspaceView` and the terminal view has no path to it. This singleton is the
//! narrowest bridge: the workspace publishes the fact, the terminal reads it before
//! taking focus for itself.

use warpui::{AppContext, Entity, GetSingletonModelHandle, SingletonEntity, UpdateModel};

#[derive(Default)]
pub struct InlineRenameState {
    editor_has_focus: bool,
}

impl InlineRenameState {
    /// Whether an inline rename editor is currently expecting the user's keystrokes.
    pub fn editor_has_focus(ctx: &AppContext) -> bool {
        Self::as_ref(ctx).editor_has_focus
    }

    pub fn set_editor_has_focus<T>(has_focus: bool, ctx: &mut T)
    where
        T: GetSingletonModelHandle + UpdateModel,
    {
        Self::handle(ctx).update(ctx, |state, _| {
            state.editor_has_focus = has_focus;
        });
    }
}

impl Entity for InlineRenameState {
    type Event = ();
}

impl SingletonEntity for InlineRenameState {}

pub fn register(app: &mut impl warpui::AddSingletonModel) {
    app.add_singleton_model(|_ctx| InlineRenameState::default());
}
