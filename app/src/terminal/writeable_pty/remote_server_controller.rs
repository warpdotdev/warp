use crate::auth::auth_state::AuthStateProvider;
use crate::remote_server::auth_context::server_api_auth_context;
use instant::Instant;
use remote_server::auth::RemoteServerAuthContext;
use std::path::PathBuf;
use std::sync::Arc;
use warp_core::SessionId;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity, WeakModelHandle};

use settings::Setting;

use crate::terminal::warpify::settings::{SshExtensionInstallMode, WarpifySettings};

use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::remote_server::ssh_transport::SshTransport;
use crate::server::server_api::ServerApiProvider;
use crate::settings::PrivacySettings;
use crate::terminal::model::session::{IsLegacySSHSession, SessionInfo};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::{send_telemetry_from_ctx, TelemetryEvent};
use remote_server::setup::{
    PreinstallCheckResult, PreinstallStatus, RemoteLibc, RemotePlatform, UnsupportedReason,
};
use remote_server::transport::Error;

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
    /// `for_update` is `true` when reinstalling over an existing install
    /// (auto-update path) and `false` for a fresh install.
    AwaitingInstall {
        session_id: SessionId,
        session_info: SessionInfo,
        transport: SshTransport,
        setup_start: Instant,
        #[allow(dead_code)]
        for_update: bool,
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
    state: SshInitState,
    /// Whether the binary was installed during this setup flow.
    did_install: bool,
    /// Detected remote platform from the binary check phase, used for telemetry.
    remote_platform: Option<RemotePlatform>,
    /// Outcome of the preinstall check from the binary check phase,
    /// used for telemetry on the supported path.
    preinstall_check: Option<PreinstallCheckResult>,
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
                preinstall_check,
                has_old_binary,
            } => {
                me.remote_platform = remote_platform.clone();
                me.preinstall_check = preinstall_check.clone();
                me.on_binary_check_complete(
                    *session_id,
                    result.clone(),
                    preinstall_check.clone(),
                    *has_old_binary,
                    ctx,
                );
            }
            RemoteServerManagerEvent::BinaryInstallComplete {
                session_id,
                result,
                install_source: _,
            } => {
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
            | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. } => {}
        });

        Self {
            pty_controller,
            model_event_dispatcher,
            state: SshInitState::Idle,
            did_install: false,
            remote_platform: None,
            preinstall_check: None,
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
            log::warn!("Remote server PtyController dropped before bootstrap could be flushed");
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
        let transport = SshTransport::new(socket_path, self.build_auth_context(ctx));
        self.did_install = false;
        self.remote_platform = None;
        self.preinstall_check = None;
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
        result: Result<bool, Arc<Error>>,
        preinstall_check: Option<PreinstallCheckResult>,
        has_old_binary: bool,
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
        if let Some(PreinstallCheckResult {
            status: PreinstallStatus::Unsupported { reason },
            libc,
            ..
        }) = preinstall_check.as_ref()
        {
            send_unsupported_telemetry(self.remote_platform.as_ref(), reason, Some(libc), ctx);
            self.flush_stashed_bootstrap(session_info, ctx);
            return;
        }

        match result {
            Ok(true) => {
                let socket_path = transport.socket_path().clone();
                let connection_label = connection_label_for_session_info(&session_info);
                self.state = SshInitState::AwaitingConnect {
                    session_id,
                    session_info,
                    setup_start,
                };
                self.connect_session_for_current_identity(
                    session_id,
                    socket_path,
                    connection_label,
                    ctx,
                );
            }
            Ok(false) if has_old_binary => {
                // Auto-update: a prior install exists, so skip the modal
                // and reinstall.
                self.did_install = true;
                self.state = SshInitState::AwaitingInstall {
                    session_id,
                    session_info,
                    transport: transport.clone(),
                    setup_start,
                    for_update: true,
                };
                RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                    mgr.install_binary(session_id, transport, true, ctx);
                });
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
                            for_update: false,
                        };
                        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                            mgr.install_binary(session_id, transport, false, ctx);
                        });
                    }
                    SshExtensionInstallMode::NeverInstall => {
                        self.flush_stashed_bootstrap(session_info, ctx);
                    }
                }
            }
            Err(err) => {
                log::warn!("Remote server binary check failed: session={session_id:?} error={err}");
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
            log::warn!(
                "Remote server install requested in unexpected state: session={session_id:?}"
            );
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

        // Reaching this path implies the user explicitly confirmed a
        // fresh install from the modal. Auto-update flows (with an old
        // binary detected) skip the modal entirely and go through
        // `on_binary_check_complete` with `is_update: true`.
        self.did_install = true;
        self.state = SshInitState::AwaitingInstall {
            session_id,
            session_info,
            transport: transport.clone(),
            setup_start,
            for_update: false,
        };
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.install_binary(session_id, transport, false, ctx);
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
        let remote_libc = self
            .preinstall_check
            .as_ref()
            .map(|check| describe_libc(&check.libc));
        send_telemetry_from_ctx!(
            TelemetryEvent::RemoteServerSetupDuration {
                duration_ms,
                installed_binary: self.did_install,
                remote_os,
                remote_arch,
                remote_libc,
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
        log::warn!("Remote server connection failed: session={session_id:?}");
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
            log::warn!("Remote server skip requested in unexpected state: session={session_id:?}");
            return;
        };
        self.flush_stashed_bootstrap(session_info, ctx);
    }

    fn on_binary_install_complete(
        &mut self,
        session_id: SessionId,
        result: Result<(), Arc<Error>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let expected = match &self.state {
            SshInitState::AwaitingInstall { session_id, .. } => *session_id,
            _ => return,
        };
        if expected != session_id {
            return;
        }

        let (session_info, transport, setup_start) =
            match std::mem::replace(&mut self.state, SshInitState::Idle) {
                SshInitState::AwaitingInstall {
                    session_info,
                    transport,
                    setup_start,
                    ..
                } => (session_info, transport, setup_start),
                _ => unreachable!("just matched AwaitingInstall above"),
            };
        match result {
            Ok(()) => {
                let socket_path = transport.socket_path().clone();
                let connection_label = connection_label_for_session_info(&session_info);
                self.state = SshInitState::AwaitingConnect {
                    session_id,
                    session_info,
                    setup_start,
                };
                self.connect_session_for_current_identity(
                    session_id,
                    socket_path,
                    connection_label,
                    ctx,
                );
            }
            Err(err) => {
                log::warn!(
                    "Remote server binary install failed: session={session_id:?} error={err}"
                );
                self.flush_stashed_bootstrap(session_info, ctx);
            }
        }
    }

    /// Builds a fresh [`RemoteServerAuthContext`] that captures the current
    /// crash-reporting preference from [`PrivacySettings`], so each
    /// connection attempt uses the latest value without requiring a
    /// long-lived cache or subscription.
    fn build_auth_context(&self, ctx: &ModelContext<Self>) -> Arc<RemoteServerAuthContext> {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let crash_reporting_enabled = PrivacySettings::handle(ctx)
            .as_ref(ctx)
            .is_crash_reporting_enabled;
        Arc::new(server_api_auth_context(
            auth_state,
            auth_client,
            crash_reporting_enabled,
        ))
    }

    fn connect_session_for_current_identity(
        &mut self,
        session_id: SessionId,
        socket_path: PathBuf,
        connection_label: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let auth_context = self.build_auth_context(ctx);
        let transport = SshTransport::new(socket_path, auth_context.clone());
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.connect_session(
                session_id,
                transport,
                auth_context,
                Some(connection_label),
                ctx,
            );
        });
    }
}

fn connection_label_for_session_info(session_info: &SessionInfo) -> String {
    let host = if session_info.hostname.is_empty() {
        session_info
            .subshell_info
            .as_ref()
            .and_then(|info| info.ssh_connection_info.as_ref())
            .and_then(|ssh| ssh.host.as_deref())
            .map(connection_label_from_ssh_host)
    } else {
        Some(session_info.hostname.clone())
    };

    connection_label_from_user_and_host(&session_info.user, host.as_deref())
}

fn connection_label_from_user_and_host(user: &str, host: Option<&str>) -> String {
    match (user.is_empty(), host.filter(|host| !host.is_empty())) {
        (false, Some(host)) => format!("{user}@{host}"),
        (false, None) => user.to_string(),
        (true, Some(host)) => host.to_string(),
        (true, None) => "Remote host".to_string(),
    }
}

fn connection_label_from_ssh_host(host: &str) -> String {
    host.rsplit_once('@')
        .map_or(host, |(_user, host)| host)
        .to_string()
}
/// Describes a [`RemoteLibc`] as a short string for telemetry.
fn describe_libc(libc: &RemoteLibc) -> String {
    match libc {
        RemoteLibc::Glibc(version) => format!("glibc {version}"),
        RemoteLibc::NonGlibc { name } => name.clone(),
        RemoteLibc::Unknown => "unknown".to_string(),
    }
}

fn send_unsupported_telemetry<T: EventLoopSender>(
    remote_platform: Option<&RemotePlatform>,
    unsupported_reason: &UnsupportedReason,
    detected_libc: Option<&RemoteLibc>,
    ctx: &mut ModelContext<RemoteServerController<T>>,
) {
    let (remote_os, remote_arch) = remote_platform
        .map(|p| {
            (
                Some(p.os.as_str().to_owned()),
                Some(p.arch.as_str().to_owned()),
            )
        })
        .unwrap_or((None, None));
    let detected_libc = detected_libc
        .map(describe_libc)
        .unwrap_or_else(|| "unknown".to_string());
    send_telemetry_from_ctx!(
        TelemetryEvent::RemoteServerHostUnsupported {
            remote_os,
            remote_arch,
            unsupported_reason: unsupported_reason.clone(),
            detected_libc,
        },
        ctx
    );
}

#[cfg(test)]
mod tests {
    use super::{connection_label_from_ssh_host, connection_label_from_user_and_host};

    #[test]
    fn connection_label_from_ssh_host_strips_user_prefix() {
        assert_eq!(
            connection_label_from_ssh_host("moira@moira.devbox.namespace"),
            "moira.devbox.namespace"
        );
        assert_eq!(
            connection_label_from_ssh_host("moira.devbox.namespace"),
            "moira.devbox.namespace"
        );
    }

    #[test]
    fn connection_label_from_user_and_host_matches_udi_format() {
        assert_eq!(
            connection_label_from_user_and_host("kevinyang", Some("ssh-testing")),
            "kevinyang@ssh-testing"
        );
        assert_eq!(
            connection_label_from_user_and_host("kevinyang", None),
            "kevinyang"
        );
        assert_eq!(
            connection_label_from_user_and_host("", Some("ssh-testing")),
            "ssh-testing"
        );
        assert_eq!(connection_label_from_user_and_host("", None), "Remote host");
    }
}
