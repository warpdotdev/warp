//! [`RootTuiView`]: the login-gated root view of the `warp-tui` front-end.

use warp::tui_export::TerminalSurfaceInit;
use warp::{TuiLoginModel, TuiLoginPhase};
use warpui_core::elements::tui::{TuiChildView, TuiElement};
use warpui_core::{
    keymap, AppContext, Entity, EntityId, SingletonEntity, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

use crate::terminal_session_view::TuiTerminalSessionView;
use crate::ui::{login_failed, login_placeholder, terminal_starting};

/// The app-level TUI shell. It gates the authenticated terminal session on login state.
pub struct RootTuiView {
    terminal_session: Option<ViewHandle<TuiTerminalSessionView>>,
}

impl RootTuiView {
    pub(crate) fn new() -> Self {
        Self {
            terminal_session: None,
        }
    }
    /// Creates the terminal child view once login has completed.
    pub(crate) fn create_terminal_session(
        &mut self,
        surface_init: TerminalSurfaceInit,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<TuiTerminalSessionView> {
        let terminal_session =
            ctx.add_typed_action_tui_view(|ctx| TuiTerminalSessionView::new(surface_init, ctx));
        self.terminal_session = Some(terminal_session.clone());
        ctx.notify();
        terminal_session
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
        // The TUI runtime uses this for child focus and event routing.
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => self
                .terminal_session
                .as_ref()
                .map(|terminal_session| vec![terminal_session.id()])
                .unwrap_or_default(),
            TuiLoginPhase::AwaitingLogin { .. } | TuiLoginPhase::Failed { .. } => Vec::new(),
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => {
                if let Some(terminal_session) = &self.terminal_session {
                    Box::new(TuiChildView::new(terminal_session))
                } else {
                    terminal_starting()
                }
            }
            TuiLoginPhase::AwaitingLogin {
                verification_uri,
                user_code,
            } => login_placeholder(verification_uri.as_deref(), user_code.as_deref()),
            TuiLoginPhase::Failed { message } => login_failed(message.as_str()),
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        // Propagate focus context into the input view so keystrokes reach it.
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}
