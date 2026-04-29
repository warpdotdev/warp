use crate::auth::auth_state::AuthStateProvider;
use crate::remote_server::auth_context::server_api_auth_context;
use instant::Instant;
use remote_server::auth::RemoteServerAuthContext;
use settings::Setting;
use std::path::PathBuf;
use std::sync::Arc;
use warp_core::SessionId;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle};

use crate::terminal::warpify::settings::SshExtensionInstallMode;

use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::remote_server::ssh_transport::SshTransport;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::session::{IsLegacySSHSession, SessionInfo};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::warpify::settings::WarpifySettings;
use crate::{send_telemetry_from_ctx, TelemetryEvent};
use remote_server::setup::RemotePlatform;

use super::pty_controller::{EventLoopSender, PtyController};

/// Per-SSH-init state machine. Encoding the state as an enum makes invalid
/// transitions unrepresentable and ensures the `SessionInfo` stash cannot be
/// accessed after it has been consumed.
///
/// Every active state carries `setup_start` so that the total setup duration
/// can be measured when the flow reaches `SessionConnected`.
enum SshInitState {
    Idle,
    /// Stash held, `check_binary` in flight.
    AwaitingCheck {
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
    },
    /// Stash held, choice block showing.
    AwaitingUserChoice {
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
    },
    /// Stash held, `install_binary` in flight.
    AwaitingInstall {
        session_id: SessionId,
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
    },
    /// Stash held, `connect_session` in flight. Bootstrap is flushed only
    /// once `SessionConnected` arrives (or on connection failure).
    AwaitingConnect {
        session_id: SessionId,
        session_info: SessionInfo,
        setup_start: Instant,
    },
}

/// Per-pane orchestrator that defers the bootstrap script write for SSH sessions,
/// checks for the remote-server binary, and presents a two-option choice block when the binary is missing.
///
/// Uses a [`WeakModelHandle`] back to [`PtyController`] to avoid preventing
/// `PtyController` from being deallocated.
pub struct RemoteServerController<T: EventLoopSender> {
    pty_controller: WeakModelHandle<PtyController<T>>,
    model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
    auth_context: Arc<RemoteServerAuthContext>,
    state: SshInitState,
    /// Whether the binary was installed during this setup flow.
    did_install: bool,
    /// Detected remote platform from the binary check phase, used for telemetry.
    remote_platform: Option<RemotePlatform>,
}

impl<T: EventLoopSender> Entity for RemoteServerController<T> {
    type Event = ();
}

impl<T: EventLoopSender> RemoteServerController<T> {
    pub fn new(
        pty_controller: WeakModelHandle<PtyController<T>>,
        model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let auth_context = Arc::new(server_api_auth_context(
            AuthStateProvider::as_ref(ctx).get().clone(),
            ServerApiProvider::as_ref(ctx).get_auth_client(),
        ));
        ctx.subscribe_to_model(&model_event_dispatcher, |me, event, ctx| {
            if let ModelEvent::SshInitShell {
                pending_session_info,
            } = event
            {
                me.on_ssh_init_shell_requested(pending_session_info.as_ref().clone(), ctx);
            }
        });

        let mgr = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&mgr, |me, event, ctx| match event {
            RemoteServerManagerEvent::BinaryCheckComplete {
                session_id,
                result,
                remote_platform,
            } => {
                me.remote_platform = remote_platform.clone();
                me.on_binary_check_complete(*session_id, result.clone(), ctx);
            }
            RemoteServerManagerEvent::BinaryInstallComplete { session_id, result } => {
                me.on_binary_install_complete(*session_id, result.clone(), ctx);
            }
            RemoteServerManagerEvent::SessionConnected { session_id, .. } => {
                me.on_session_connected(*session_id, ctx);
            }
            RemoteServerManagerEvent::SessionConnectionFailed { session_id, .. } => {
                me.on_session_connection_failed(*session_id, ctx);
            }
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionDisconnected { .. }
            | RemoteServerManagerEvent::SessionReconnected { .. }
            | RemoteServerManagerEvent::SessionDeregistered { .. }
            | RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::HostDisconnected { .. }
            | RemoteServerManagerEvent::NavigatedToDirectory { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. } => {}
        });

        Self {
            pty_controller,
            model_event_dispatcher,
            auth_context,
            state: SshInitState::Idle,
            did_install: false,
            remote_platform: None,
        }
    }

    /// Extracts the `SessionInfo` from the stash and writes the bootstrap
    /// script to the PTY via `PtyController::initialize_shell`.
    fn flush_stashed_bootstrap(&mut self, session_info: SessionInfo, ctx: &mut ModelContext<Self>) {
        if let Some(pty) = self.pty_controller.upgrade(ctx) {
            pty.update(ctx, |pty, ctx| {
                pty.initialize_shell(&session_info, ctx);
            });
        } else {
            log::warn!("PtyController dropped before bootstrap could be flushed");
        }
    }

    /// Idle -> AwaitingCheck
    fn on_ssh_init_shell_requested(&mut self, info: SessionInfo, ctx: &mut ModelContext<Self>) {
        let IsLegacySSHSession::Yes { socket_path } = &info.is_legacy_ssh_session else {
            return;
        };
        let session_id = info.session_id;
        let socket_path = socket_path.clone();
        debug_assert!(matches!(self.state, SshInitState::Idle));
        match std::mem::replace(&mut self.state, SshInitState::Idle) {
            SshInitState::Idle => {}
            SshInitState::AwaitingCheck {
                session_info: old_info,
                ..
            }
            | SshInitState::AwaitingUserChoice {
                session_info: old_info,
                ..
            }
            | SshInitState::AwaitingInstall {
                session_info: old_info,
                ..
            }
            | SshInitState::AwaitingConnect {
                session_info: old_info,
                ..
            } => {
                self.flush_stashed_bootstrap(old_info, ctx);
            }
        }
        let transport = SshTransport::new(socket_path, self.auth_context.clone());
        self.did_install = false;
        self.remote_platform = None;
        self.state = SshInitState::AwaitingCheck {
            session_info: info,
            transport: transport.clone(),
            setup_start: Instant::now(),
        };
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.check_binary(session_id, transport, ctx);
        });
    }

    fn on_binary_check_complete(
        &mut self,
        session_id: SessionId,
        result: Result<bool, String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingCheck {
            ref session_info, ..
        } = self.state
        else {
            return;
        };
        if session_info.session_id != session_id {
            return;
        }

        let SshInitState::AwaitingCheck {
            session_info,
            transport,
            setup_start,
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingCheck above");
        };

        match result {
            Ok(true) => {
                let socket_path = transport.socket_path().clone();
                self.state = SshInitState::AwaitingConnect {
                    session_id,
                    session_info,
                    setup_start,
                };
                self.connect_session_for_current_identity(session_id, socket_path, ctx);
            }
            Ok(false) => {
                let install_mode = *WarpifySettings::as_ref(ctx)
                    .ssh_extension_install_mode
                    .value();
                match install_mode {
                    SshExtensionInstallMode::AlwaysAsk => {
                        self.state = SshInitState::AwaitingUserChoice {
                            session_info,
                            transport,
                            setup_start,
                        };
                        self.model_event_dispatcher.update(ctx, |d, ctx| {
                            d.request_remote_server_block(session_id, ctx);
                        });
                    }
                    SshExtensionInstallMode::AlwaysInstall => {
                        self.did_install = true;
                        self.state = SshInitState::AwaitingInstall {
                            session_id,
                            session_info,
                            transport: transport.clone(),
                            setup_start,
                        };
                        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                            mgr.install_binary(session_id, transport, ctx);
                        });
                    }
                    SshExtensionInstallMode::NeverInstall => {
                        self.flush_stashed_bootstrap(session_info, ctx);
                    }
                }
            }
            Err(err) => {
                log::error!("Binary check failed for {session_id:?}: {err}");
                self.flush_stashed_bootstrap(session_info, ctx);
            }
        }
    }

    pub fn handle_ssh_remote_server_install(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingUserChoice { .. } = self.state else {
            log::warn!("Install clicked but state is not AwaitingUserChoice for {session_id:?}");
            return;
        };

        let SshInitState::AwaitingUserChoice {
            session_info,
            transport,
            setup_start,
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingUserChoice above");
        };

        self.did_install = true;
        self.state = SshInitState::AwaitingInstall {
            session_id,
            session_info,
            transport: transport.clone(),
            setup_start,
        };
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.install_binary(session_id, transport, ctx);
        });
    }

    /// Called when the remote server session is connected. Flushes the
    /// stashed bootstrap (so the session initializes with a live client)
    /// and emits the `RemoteServerSetupDuration` telemetry event.
    fn on_session_connected(&mut self, session_id: SessionId, ctx: &mut ModelContext<Self>) {
        let SshInitState::AwaitingConnect {
            session_id: expected,
            ..
        } = &self.state
        else {
            return;
        };
        if *expected != session_id {
            return;
        }

        let SshInitState::AwaitingConnect {
            session_info,
            setup_start,
            ..
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingConnect above");
        };

        // Flush the stashed bootstrap now that the server is connected.
        // `client_for_session` will return `Some` when the session
        // subsequently initializes, so it picks `RemoteServerCommandExecutor`.
        self.flush_stashed_bootstrap(session_info, ctx);

        let duration_ms = Instant::now()
            .duration_since(setup_start)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let (remote_os, remote_arch) = self
            .remote_platform
            .as_ref()
            .map(|p| {
                (
                    Some(p.os.as_str().to_owned()),
                    Some(p.arch.as_str().to_owned()),
                )
            })
            .unwrap_or((None, None));
        send_telemetry_from_ctx!(
            TelemetryEvent::RemoteServerSetupDuration {
                duration_ms,
                installed_binary: self.did_install,
                remote_os,
                remote_arch,
            },
            ctx
        );
    }

    /// Called when the remote server connection failed. Flushes the stashed
    /// bootstrap so the SSH session is not permanently blocked.
    fn on_session_connection_failed(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingConnect {
            session_id: expected,
            ..
        } = &self.state
        else {
            return;
        };
        if *expected != session_id {
            return;
        }

        let SshInitState::AwaitingConnect { session_info, .. } =
            std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingConnect above");
        };
        log::warn!(
            "Remote server connection failed for session {session_id:?}; \
             flushing bootstrap to unblock SSH session"
        );
        self.flush_stashed_bootstrap(session_info, ctx);
    }

    pub fn handle_ssh_remote_server_skip(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingUserChoice { session_info, .. } =
            std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            log::warn!("Skip clicked but state is not AwaitingUserChoice for {session_id:?}");
            return;
        };
        self.flush_stashed_bootstrap(session_info, ctx);
    }

    fn on_binary_install_complete(
        &mut self,
        session_id: SessionId,
        result: Result<(), String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let SshInitState::AwaitingInstall {
            session_id: expected,
            ..
        } = &self.state
        else {
            return;
        };
        if *expected != session_id {
            return;
        }

        let SshInitState::AwaitingInstall {
            session_info,
            transport,
            setup_start,
            ..
        } = std::mem::replace(&mut self.state, SshInitState::Idle)
        else {
            unreachable!("just matched AwaitingInstall above");
        };
        match result {
            Ok(()) => {
                let socket_path = transport.socket_path().clone();
                self.state = SshInitState::AwaitingConnect {
                    session_id,
                    session_info,
                    setup_start,
                };
                self.connect_session_for_current_identity(session_id, socket_path, ctx);
            }
            Err(err) => {
                log::error!("Binary install failed for {session_id:?}: {err}");
                self.flush_stashed_bootstrap(session_info, ctx);
            }
        }
    }

    fn connect_session_for_current_identity(
        &mut self,
        session_id: SessionId,
        socket_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) {
        let transport = SshTransport::new(socket_path, self.auth_context.clone());
        let auth_context = self.auth_context.clone();
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.connect_session(session_id, transport, auth_context, ctx);
        });
    }
}
