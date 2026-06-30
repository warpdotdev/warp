//! [`RootTuiView`]: the login-gated root view of the `warp-tui` front-end.

use warp::{TuiLoginModel, TuiLoginPhase};
use warpui_core::elements::tui::{TuiChildView, TuiElement};
use warpui_core::{
    keymap, AppContext, Entity, EntityId, SingletonEntity, TuiView, TypedActionView, ViewHandle,
};

use crate::terminal_session_view::TuiTerminalSessionView;
use crate::ui::{login_failed, login_placeholder};

/// The app-level TUI shell. It gates the authenticated terminal session on login state.
pub struct RootTuiView {
    terminal_session: ViewHandle<TuiTerminalSessionView>,
}

impl RootTuiView {
    pub(crate) fn new(terminal_session: ViewHandle<TuiTerminalSessionView>) -> Self {
        Self { terminal_session }
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
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => vec![self.terminal_session.id()],
            TuiLoginPhase::AwaitingLogin { .. } | TuiLoginPhase::Failed { .. } => Vec::new(),
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => Box::new(TuiChildView::new(&self.terminal_session)),
            TuiLoginPhase::AwaitingLogin {
                verification_uri,
                user_code,
            } => login_placeholder(verification_uri.as_deref(), user_code.as_deref()),
            TuiLoginPhase::Failed { message } => login_failed(message.as_str()),
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}
