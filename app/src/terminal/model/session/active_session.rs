use std::path::{Path, PathBuf};
use std::sync::Arc;

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

use super::{Session, SessionType, Sessions};
use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::terminal::ShellLaunchData;
use crate::terminal::model::session::SessionsEvent;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::shell::ShellType;

pub struct ActiveSession {
    model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
    sessions: ModelHandle<Sessions>,

    /// The current working directory of the terminal session.
    current_working_directory: Option<String>,
}

impl ActiveSession {
    pub fn new(
        sessions: ModelHandle<Sessions>,
        model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&model_event_dispatcher, move |me, _, event, ctx| {
            let new_pwd = match event {
                ModelEvent::BlockMetadataReceived(e) => Some(
                    e.block_metadata
                        .current_working_directory()
                        .map(|cwd| cwd.to_owned()),
                ),
                ModelEvent::BlockWorkingDirectoryUpdated(e) => Some(
                    e.block_metadata
                        .current_working_directory()
                        .map(|cwd| cwd.to_owned()),
                ),
                _ => None,
            };
            if let Some(new_pwd) = new_pwd
                && me.current_working_directory != new_pwd
            {
                me.current_working_directory = new_pwd;
                ctx.emit(ActiveSessionEvent::UpdatedPwd);
            }
        });

        ctx.subscribe_to_model(&sessions, |me, _, event, ctx| {
            if let SessionsEvent::SessionBootstrapped(bootstrap_event) = event
                && Some(bootstrap_event.session_id)
                    == me.model_event_dispatcher.as_ref(ctx).active_session_id()
            {
                ctx.emit(ActiveSessionEvent::Bootstrapped);
            }
        });

        Self {
            sessions,
            model_event_dispatcher,
            current_working_directory: None,
        }
    }

    pub fn session(&self, app: &AppContext) -> Option<Arc<Session>> {
        self.model_event_dispatcher
            .as_ref(app)
            .active_session_id()
            .and_then(|session_id| self.sessions.as_ref(app).get(session_id))
    }

    pub fn session_type(&self, app: &AppContext) -> Option<SessionType> {
        self.session(app).map(|session| session.session_type())
    }

    pub fn shell_type(&self, app: &AppContext) -> Option<ShellType> {
        self.session(app)
            .as_ref()
            .map(|session| session.shell().shell_type())
    }

    pub fn shell_launch_data(&self, app: &AppContext) -> Option<ShellLaunchData> {
        self.session(app)
            .as_ref()
            .and_then(|session| session.launch_data().cloned())
    }

    pub fn current_working_directory(&self) -> Option<&String> {
        self.current_working_directory.as_ref()
    }

    /// Returns a session-aware path for `path`.
    ///
    /// Local session paths are canonicalized to match git-detected repository paths on
    /// case-insensitive filesystems. Remote session paths are standardized and tagged with
    /// the connected host ID.
    pub fn location_for_path(&self, path: &str, app: &AppContext) -> Option<LocalOrRemotePath> {
        match self.session_type(app) {
            Some(SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            }) => StandardizedPath::try_new(path)
                .ok()
                .map(|path| LocalOrRemotePath::Remote(RemotePath::new(host_id, path))),
            Some(SessionType::WarpifiedRemote { host_id: None }) => None,
            Some(SessionType::Local) | None => {
                let path =
                    dunce::canonicalize(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path));
                Some(LocalOrRemotePath::Local(path))
            }
        }
    }

    pub fn current_working_directory_location(
        &self,
        app: &AppContext,
    ) -> Option<LocalOrRemotePath> {
        let cwd = self.current_working_directory()?;
        self.location_for_path(cwd.as_str(), app)
    }

    /// Returns the `WarpAiExecutionContext` for the active session.
    pub fn ai_execution_environment(&self, app: &AppContext) -> Option<WarpAiExecutionContext> {
        self.session(app).as_ref().map(WarpAiExecutionContext::new)
    }
}

pub enum ActiveSessionEvent {
    /// The active session's working directory changed.
    UpdatedPwd,
    /// The active session finished bootstrapping.
    Bootstrapped,
}

impl Entity for ActiveSession {
    type Event = ActiveSessionEvent;
}
