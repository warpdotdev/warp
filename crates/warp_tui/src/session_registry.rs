//! [`TuiSessions`]: registry and foreground selection for live TUI sessions.
//!
//! Every session is a full [`TuiTerminalSessionView`] backed by a retained
//! terminal manager. The container owns session lifetime and focus; the root
//! view renders and routes input only to the focused session.
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use pathfinder_geometry::vector::Vector2F;
use warp::tui_export::{
    AIConversationId, BannerState, BlocklistAIHistoryModel, IsSharedSessionCreator,
    LocalTtyTerminalManager, ServerConversationToken, TerminalManagerTrait, TerminalSurfaceResult,
};
use warpui::SingletonEntity;
use warpui_core::runtime::TuiDriverHandle;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle, WindowId};

use crate::orchestration_model::{
    MaterializedLocalOzChildSession, TuiOrchestrationEvent, TuiOrchestrationModel,
};
use crate::resume::TuiExitSummaryHandle;
use crate::terminal_session_view::{TuiTerminalSessionEvent, TuiTerminalSessionView};
use crate::transcript_view::TRANSCRIPT_BLOCK_SPACING;

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
    /// A session was removed from the container.
    SessionRemoved(TuiSessionId),
    /// The focused session changed to this id.
    FocusChanged(TuiSessionId),
}

/// Owns all live TUI sessions and the focused-session selection.
pub(crate) struct TuiSessions {
    /// TUI-specific process driver. Its handle restores terminal mode on
    /// drop, so the app-lifetime session singleton must retain it.
    _driver: Option<TuiDriverHandle>,
    keyboard_enhancement_supported: bool,
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
    /// Creates and registers a full local terminal session.
    pub(crate) fn create_local_terminal_session(
        sessions: &ModelHandle<Self>,
        window_id: WindowId,
        focus: bool,
        startup_directory: Option<PathBuf>,
        ctx: &mut AppContext,
    ) -> (TuiSessionId, ViewHandle<TuiTerminalSessionView>) {
        let (exit_summary, keyboard_enhancement_supported) = sessions.read(ctx, |sessions, _| {
            (
                sessions.exit_summary.clone(),
                sessions.keyboard_enhancement_supported,
            )
        });
        // The manager uses this internal model for unsupported-shell state; the
        // TUI does not render a separate banner surface.
        let banner = ctx.add_model(|_| BannerState::default());
        let manager = LocalTtyTerminalManager::<TuiTerminalSessionView>::create_tui_model(
            startup_directory,
            HashMap::<OsString, OsString>::from_iter(std::env::vars_os()),
            IsSharedSessionCreator::No,
            None,
            banner.clone(),
            Vector2F::new(120., 24.),
            None,
            None,
            TRANSCRIPT_BLOCK_SPACING,
            ctx,
            move |surface_init, ctx| {
                let surface = ctx.add_typed_action_tui_view(window_id, |ctx| {
                    TuiTerminalSessionView::new(
                        surface_init,
                        exit_summary,
                        keyboard_enhancement_supported,
                        ctx,
                    )
                });
                TerminalSurfaceResult {
                    surface,
                    post_wire: move |_manager: &mut LocalTtyTerminalManager<
                        TuiTerminalSessionView,
                    >,
                                     _surface: &ViewHandle<TuiTerminalSessionView>,
                                     _ctx: &mut AppContext| {},
                }
            },
        );

        let surface = manager.surface.clone();
        let session_id =
            Self::register_session(sessions, manager.surface, manager.manager, focus, ctx);
        (session_id, surface)
    }
    /// Wires a session view to orchestration before registering it.
    pub(crate) fn register_session(
        sessions: &ModelHandle<Self>,
        view: ViewHandle<TuiTerminalSessionView>,
        manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
        focus: bool,
        ctx: &mut AppContext,
    ) -> TuiSessionId {
        let id = TuiSessionId(view.id());
        if ctx.has_singleton_model::<TuiOrchestrationModel>() {
            let orchestration = TuiOrchestrationModel::handle(ctx);
            ctx.subscribe_to_view(&view, move |_, event, ctx| match event {
                TuiTerminalSessionEvent::StartAgentConversation {
                    request,
                    working_directory,
                } => {
                    orchestration.update(ctx, |orchestration, ctx| {
                        orchestration.dispatch_create_agent(
                            id,
                            (**request).clone(),
                            working_directory.clone(),
                            ctx,
                        );
                    });
                }
                TuiTerminalSessionEvent::CleanupFailedChildLaunch { conversation_id } => {
                    orchestration.update(ctx, |orchestration, ctx| {
                        orchestration.cleanup_failed_child(conversation_id, ctx);
                    });
                }
                TuiTerminalSessionEvent::ExecuteCommand(_)
                | TuiTerminalSessionEvent::InterruptPty
                | TuiTerminalSessionEvent::WriteAgentInput { .. }
                | TuiTerminalSessionEvent::WriteUserInput(_)
                | TuiTerminalSessionEvent::Resize(_) => {}
            });
        }
        sessions.update(ctx, |sessions, ctx| {
            debug_assert!(
                sessions.session(id).is_none(),
                "a session must not be registered twice"
            );
            sessions.sessions.push(TuiSession {
                id,
                view,
                _manager: manager,
            });
            if focus {
                sessions.focus_session(id, ctx);
            }
            ctx.notify();
            id
        })
    }

    /// Subscribes the session owner to orchestration lifecycle requests.
    pub(crate) fn wire_orchestration(
        sessions: &ModelHandle<Self>,
        orchestration: &ModelHandle<TuiOrchestrationModel>,
        ctx: &mut AppContext,
    ) {
        let sessions_for_model_updates = sessions.clone();
        ctx.observe_model(orchestration, move |_, ctx| {
            let focused_view = sessions_for_model_updates
                .as_ref(ctx)
                .focused_session()
                .map(|session| session.view().clone());
            if let Some(focused_view) = focused_view {
                focused_view.update(ctx, |view, ctx| {
                    view.refresh_orchestration_tab_state(ctx);
                });
            }
        });

        let sessions_for_focus_updates = sessions.clone();
        ctx.subscribe_to_model(sessions, move |_, event, ctx| {
            let TuiSessionsEvent::FocusChanged(session_id) = event else {
                return;
            };
            let focused_view = sessions_for_focus_updates
                .as_ref(ctx)
                .session(*session_id)
                .map(|session| session.view().clone());
            if let Some(focused_view) = focused_view {
                focused_view.update(ctx, |view, ctx| {
                    view.refresh_orchestration_tab_state(ctx);
                });
            }
        });
        let sessions = sessions.clone();
        let orchestration_for_events = orchestration.clone();
        ctx.subscribe_to_model(orchestration, move |_, event, ctx| match event {
            TuiOrchestrationEvent::CreateLocalOzChildSession {
                parent_session_id,
                request,
                model_id,
                working_directory,
                task_id,
                conversation_name,
            } => {
                let window_id = sessions
                    .as_ref(ctx)
                    .session(*parent_session_id)
                    .expect("the dispatching parent session must remain registered")
                    .view()
                    .window_id(ctx);
                let (session_id, session_view) = Self::create_local_terminal_session(
                    &sessions,
                    window_id,
                    false,
                    working_directory.clone(),
                    ctx,
                );
                orchestration_for_events.update(ctx, |orchestration, ctx| {
                    orchestration.register_local_oz_child_session(
                        MaterializedLocalOzChildSession {
                            parent_session_id: *parent_session_id,
                            session_id,
                            session_view,
                            request: (**request).clone(),
                            model_id: model_id.clone(),
                            task_id: *task_id,
                            conversation_name: conversation_name.clone(),
                        },
                        ctx,
                    );
                });
            }
            TuiOrchestrationEvent::RemoveChildSession(session_id) => {
                sessions.update(ctx, |sessions, ctx| {
                    sessions.remove_session(*session_id, ctx);
                });
            }
        });
    }

    /// Creates the app's session container.
    pub(crate) fn new(
        driver: TuiDriverHandle,
        exit_summary: TuiExitSummaryHandle,
        resume_token: Option<ServerConversationToken>,
    ) -> Self {
        let keyboard_enhancement_supported = driver.keyboard_enhancement_supported();
        Self {
            _driver: Some(driver),
            keyboard_enhancement_supported,
            exit_summary,
            sessions: Vec::new(),
            focused_session_id: None,
            resume_token,
        }
    }

    /// Creates a driverless container for unit tests.
    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self {
            _driver: None,
            keyboard_enhancement_supported: false,
            exit_summary: TuiExitSummaryHandle::default(),
            sessions: Vec::new(),
            focused_session_id: None,
            resume_token: None,
        }
    }

    /// Removes a session. When the focused session is removed, focus falls
    /// back to the most recently added remaining session, if any.
    pub(crate) fn remove_session(&mut self, id: TuiSessionId, ctx: &mut ModelContext<Self>) {
        let before = self.sessions.len();
        self.sessions.retain(|session| session.id != id);
        if self.sessions.len() == before {
            return;
        }
        if ctx.has_singleton_model::<TuiOrchestrationModel>() {
            TuiOrchestrationModel::handle(ctx).update(ctx, |orchestration, ctx| {
                orchestration.handle_session_removed(id, ctx);
            });
        }
        ctx.emit(TuiSessionsEvent::SessionRemoved(id));
        if self.focused_session_id == Some(id) {
            self.focused_session_id = None;
            if let Some(fallback) = self.sessions.last().map(|session| session.id) {
                self.focus_session(fallback, ctx);
            }
        }
        ctx.notify();
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

    /// Builds the loaded conversation-to-session index used by one topology snapshot.
    pub(crate) fn session_ids_by_conversation(
        &self,
        history: &BlocklistAIHistoryModel,
    ) -> HashMap<AIConversationId, TuiSessionId> {
        self.sessions
            .iter()
            .flat_map(|session| {
                history
                    .all_live_conversations_for_terminal_surface(session.id.surface_id())
                    .map(move |conversation| (conversation.id(), session.id))
            })
            .collect()
    }

    /// Whether no session has been registered.
    pub(crate) fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Consumes the startup resume token.
    pub(crate) fn take_resume_token(&mut self) -> Option<ServerConversationToken> {
        self.resume_token.take()
    }
}

#[cfg(test)]
#[path = "session_registry_tests.rs"]
mod tests;
