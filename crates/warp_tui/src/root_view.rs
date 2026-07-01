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

/// Whether the authenticated terminal session has been created yet. Mirrors the
/// GUI root view's `AuthOnboardingState` split between the pre-session login gate
/// and the live terminal session.
enum RootTuiState {
    /// Login gate: no terminal session exists yet. The placeholder shown is
    /// chosen from the current [`TuiLoginPhase`].
    Auth,
    /// The authenticated terminal session.
    Terminal(ViewHandle<TuiTerminalSessionView>),
}

/// The app-level TUI shell. It gates the authenticated terminal session on login state.
pub struct RootTuiView {
    state: RootTuiState,
}

impl RootTuiView {
    pub(crate) fn new() -> Self {
        Self {
            state: RootTuiState::Auth,
        }
    }
    /// Creates the terminal child view once login has completed, or returns the
    /// existing one if it was already created. Callers notify the root so it
    /// re-renders from the login placeholder to the terminal session.
    pub(crate) fn create_terminal_session(
        &mut self,
        surface_init: TerminalSurfaceInit,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<TuiTerminalSessionView> {
        if let RootTuiState::Terminal(terminal_session) = &self.state {
            return terminal_session.clone();
        }
        let terminal_session =
            ctx.add_typed_action_tui_view(|ctx| TuiTerminalSessionView::new(surface_init, ctx));
        self.state = RootTuiState::Terminal(terminal_session.clone());
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

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        // The TUI runtime uses this for child focus and event routing; only the
        // live terminal session participates.
        match &self.state {
            RootTuiState::Terminal(terminal_session) => vec![terminal_session.id()],
            RootTuiState::Auth => Vec::new(),
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match &self.state {
            RootTuiState::Terminal(terminal_session) => {
                TuiChildView::new(terminal_session).finish()
            }
            RootTuiState::Auth => match TuiLoginModel::as_ref(ctx).phase() {
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
        // Propagate focus context into the input view so keystrokes reach it.
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}
