//! [`RootTuiView`]: the login-gated root view of the `warp-tui` front-end.

use warp::{TuiLoginModel, TuiLoginPhase};
use warpui::SingletonEntity as _;
use warpui_core::elements::tui::{TuiChildView, TuiElement};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::FixedBinding;
use warpui_core::platform::TerminationMode;
use warpui_core::{
    keymap, AppContext, Entity, EntityId, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::keybindings::TUI_BINDING_GROUP;
use crate::session_registry::TuiSessions;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::ui::{login_failed, login_placeholder, terminal_starting};

/// Typed actions handled by [`RootTuiView`].
#[derive(Debug, Clone)]
pub enum RootTuiAction {
    /// Exits the app while no terminal session is focused.
    ExitApp,
}

/// The app-level TUI shell, projecting only the focused full session view.
pub struct RootTuiView;

/// Registers the root view's keybindings.
pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        RootTuiAction::ExitApp,
        id!(RootTuiView::ui_name()),
    )
    .with_group(TUI_BINDING_GROUP)]);
}

impl RootTuiView {
    /// Creates the login-gated root view.
    pub(crate) fn new() -> Self {
        Self
    }

    fn focused_session_view(&self, ctx: &AppContext) -> Option<ViewHandle<TuiTerminalSessionView>> {
        if !ctx.has_singleton_model::<TuiSessions>() {
            return None;
        }

        TuiSessions::as_ref(ctx)
            .focused_session()
            .map(|session| session.view().clone())
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn child_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        self.focused_session_view(ctx)
            .map(|view| vec![view.id()])
            .unwrap_or_default()
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match self.focused_session_view(ctx) {
            Some(view) => TuiChildView::new(&view).finish(),
            None => match TuiLoginModel::as_ref(ctx).phase() {
                TuiLoginPhase::LoggedIn => terminal_starting(),
                TuiLoginPhase::AwaitingLogin {
                    verification_uri,
                    user_code,
                } => login_placeholder(verification_uri.as_deref(), user_code.as_deref()),
                TuiLoginPhase::Failed { message } => login_failed(message.as_str()),
            },
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = RootTuiAction;

    fn handle_action(&mut self, action: &RootTuiAction, ctx: &mut ViewContext<Self>) {
        match action {
            RootTuiAction::ExitApp => ctx.terminate_app(TerminationMode::ForceTerminate, None),
        }
    }
}

#[cfg(test)]
#[path = "root_view_tests.rs"]
mod tests;
