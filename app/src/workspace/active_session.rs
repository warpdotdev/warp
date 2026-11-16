use std::collections::HashMap;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity, WindowId};

use crate::terminal::model::session::Session;

/// The active terminal session in each window. The active session of a window is the current
/// session of the most-recently-focused terminal pane of the active tab of the window's workspace.
///
/// #### When to use `ActiveSession`
/// Generally, if a more specific session is available, it should be preferred. For example, when
/// opening a Markdown file from a file link in a block's output, that block's session should be
/// the basis. However, sometimes there is no contextual session (such as when opening a file
/// in Warp from Finder, or when starting from a cloud object). In that case, the `ActiveSession`
/// might be used, but it's often still better to be context-independent.
#[derive(Default)]
pub struct ActiveSession {
    window_sessions: HashMap<WindowId, WindowActiveSession>,
}

/// Active session information for an individual window.
#[derive(Default)]
struct WindowActiveSession {
    /// The [`Session`] model for the active session. This is a weak reference so that it doesn't
    /// prevent cleaning up the session when it closes, in case no other session is activated.
    session: Option<Weak<Session>>,
    /// The active session's working directory (local or remote).
    working_directory: Option<LocalOrRemotePath>,
    /// The [`EntityId`]` for the [`TerminalView`] for the active session, if there is one.
    terminal_view_id: Option<EntityId>,
}

impl ActiveSession {
    /// The workspace's active session, if there is one.
    pub fn session(&self, window_id: WindowId) -> Option<Arc<Session>> {
        self.window_sessions
            .get(&window_id)?
            .session
            .as_ref()?
            .upgrade()
    }

    pub fn terminal_view_id(&self, window_id: WindowId) -> Option<EntityId> {
        self.window_sessions.get(&window_id)?.terminal_view_id
    }

    /// The current working directory of the active session, if it's local.
    pub fn path_if_local(&self, window_id: WindowId) -> Option<&Path> {
        self.window_sessions
            .get(&window_id)?
            .working_directory
            .as_ref()
            .and_then(|wd| wd.to_local_path())
    }

    /// The current working directory of the active session (local or remote).
    pub fn working_directory(&self, window_id: WindowId) -> Option<&LocalOrRemotePath> {
        self.window_sessions
            .get(&window_id)?
            .working_directory
            .as_ref()
    }

    /// Set the current session, for use in tests.
    #[cfg(test)]
    pub fn set_session_for_test(
        &mut self,
        window_id: WindowId,
        session: Arc<Session>,
        path_if_local: Option<impl Into<PathBuf>>,
        terminal_view_id: Option<EntityId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_session_state(
            window_id,
            Some(session),
            path_if_local.map(|p| LocalOrRemotePath::Local(p.into())),
            terminal_view_id,
            ctx,
        );
    }

    pub(super) fn set_session_state(
        &mut self,
        window_id: WindowId,
        session: Option<Arc<Session>>,
        working_directory: Option<LocalOrRemotePath>,
        terminal_view_id: Option<EntityId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let window_state = self.window_sessions.entry(window_id).or_default();

        let session = session.map(|session| Arc::downgrade(&session));
        if window_state.session.is_some() != session.is_some() {
            window_state.session = session;
            ctx.notify();
        } else if let Some((prev_session, next_session)) =
            window_state.session.as_ref().zip(session)
        {
            // Session IDs can't necessarily be compared across terminal panes, so check if the backing
            // allocation is the same. We can do this because each `Session` is a singleton.
            if !Weak::ptr_eq(prev_session, &next_session) {
                window_state.session = Some(next_session);
                ctx.notify();
            }
        }

        if window_state.working_directory != working_directory {
            window_state.working_directory = working_directory;
            ctx.notify();
        }

        if window_state.terminal_view_id != terminal_view_id {
            window_state.terminal_view_id = terminal_view_id;
            ctx.notify();
        }
    }

    pub(super) fn close_workspace(&mut self, window_id: WindowId) {
        self.window_sessions.remove(&window_id);
    }
}

impl Entity for ActiveSession {
    type Event = ();
}

impl SingletonEntity for ActiveSession {}
