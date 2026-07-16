//! [`TuiSessions`]: registry and foreground selection for live TUI sessions.
//!
//! Every session is a full [`TuiTerminalSessionView`] backed by a retained
//! terminal manager. The container owns session lifetime and focus; the root
//! view renders and routes input only to the focused session.

use warp::tui_export::{ServerConversationToken, TerminalManagerTrait};
use warpui::SingletonEntity;
use warpui_core::runtime::TuiDriverHandle;
use warpui_core::{Entity, EntityId, ModelContext, ModelHandle, ViewHandle, WindowId};

use crate::resume::TuiExitSummaryHandle;
use crate::terminal_session_view::TuiTerminalSessionView;

/// Identifies a TUI terminal session.
///
/// A session and its eagerly-created view have the same lifetime, so the
/// view's entity id is also the terminal surface id used by shared AI models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TuiSessionId(EntityId);

impl TuiSessionId {
    /// The raw entity id used at shared-model boundaries.
    pub(crate) fn surface_id(self) -> EntityId {
        self.0
    }
}

/// A live TUI session: its full view and the manager retaining its PTY.
pub(crate) struct TuiSession {
    id: TuiSessionId,
    view: ViewHandle<TuiTerminalSessionView>,
    /// Retained for the session's lifetime to keep its PTY and event loop alive.
    _manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
}

impl TuiSession {
    /// The session's full terminal view.
    pub(crate) fn view(&self) -> &ViewHandle<TuiTerminalSessionView> {
        &self.view
    }
}

/// Events emitted as the session set or focus changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiSessionsEvent {
    /// A session was registered, possibly in the background.
    SessionAdded(TuiSessionId),
    /// The focused session changed to this id.
    FocusChanged(TuiSessionId),
}

/// Owns all live TUI sessions and the focused-session selection.
pub(crate) struct TuiSessions {
    /// TUI-specific process driver. Its handle restores terminal mode on
    /// drop, so the app-lifetime session singleton must retain it.
    _driver: Option<TuiDriverHandle>,
    keyboard_enhancement_supported: bool,
    window_id: WindowId,
    exit_summary: TuiExitSummaryHandle,
    sessions: Vec<TuiSession>,
    focused_session_id: Option<TuiSessionId>,
    resume_token: Option<ServerConversationToken>,
}

impl Entity for TuiSessions {
    type Event = TuiSessionsEvent;
}

impl SingletonEntity for TuiSessions {}

impl TuiSessions {
    /// Creates the app's session container.
    pub(crate) fn new(
        driver: TuiDriverHandle,
        window_id: WindowId,
        exit_summary: TuiExitSummaryHandle,
        resume_token: Option<ServerConversationToken>,
    ) -> Self {
        let keyboard_enhancement_supported = driver.keyboard_enhancement_supported();
        Self {
            _driver: Some(driver),
            keyboard_enhancement_supported,
            window_id,
            exit_summary,
            sessions: Vec::new(),
            focused_session_id: None,
            resume_token,
        }
    }

    /// Creates a driverless container for unit tests.
    #[cfg(test)]
    pub(crate) fn new_for_test(window_id: WindowId) -> Self {
        Self {
            _driver: None,
            keyboard_enhancement_supported: false,
            window_id,
            exit_summary: TuiExitSummaryHandle::default(),
            sessions: Vec::new(),
            focused_session_id: None,
            resume_token: None,
        }
    }

    /// Registers an eagerly-created session view and optionally focuses it.
    pub(crate) fn add_session(
        &mut self,
        view: ViewHandle<TuiTerminalSessionView>,
        manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
        focus: bool,
        ctx: &mut ModelContext<Self>,
    ) -> TuiSessionId {
        let id = TuiSessionId(view.id());
        debug_assert!(
            self.session(id).is_none(),
            "a session must not be registered twice"
        );
        self.sessions.push(TuiSession {
            id,
            view,
            _manager: manager,
        });
        ctx.emit(TuiSessionsEvent::SessionAdded(id));
        if focus {
            self.focus_session(id, ctx);
        }
        ctx.notify();
        id
    }

    /// Returns the window and exit-summary handle used to create session views.
    pub(crate) fn surface_context(&self) -> (WindowId, TuiExitSummaryHandle, bool) {
        (
            self.window_id,
            self.exit_summary.clone(),
            self.keyboard_enhancement_supported,
        )
    }

    /// Focuses a registered session. Returns whether focus changed.
    pub(crate) fn focus_session(&mut self, id: TuiSessionId, ctx: &mut ModelContext<Self>) -> bool {
        if self.focused_session_id == Some(id) || self.session(id).is_none() {
            return false;
        }
        self.focused_session_id = Some(id);
        let view = self
            .session(id)
            .expect("focused session was validated above")
            .view
            .clone();
        view.update(ctx, |view, ctx| view.activate(ctx));
        ctx.emit(TuiSessionsEvent::FocusChanged(id));
        ctx.notify();
        true
    }

    /// The focused session's id.
    pub(crate) fn focused_session_id(&self) -> Option<TuiSessionId> {
        self.focused_session_id
    }

    /// The focused session.
    pub(crate) fn focused_session(&self) -> Option<&TuiSession> {
        self.focused_session_id.and_then(|id| self.session(id))
    }

    /// Looks up a registered session.
    pub(crate) fn session(&self, id: TuiSessionId) -> Option<&TuiSession> {
        self.sessions.iter().find(|session| session.id == id)
    }

    /// Whether no session has been registered.
    pub(crate) fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Consumes the startup resume token.
    pub(crate) fn take_resume_token(&mut self) -> Option<ServerConversationToken> {
        self.resume_token.take()
    }
}

#[cfg(test)]
#[path = "session_registry_tests.rs"]
mod tests;
