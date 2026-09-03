use std::path::PathBuf;
use std::sync::Arc;

use remote_server::setup::{PreinstallCheckResult, PreinstallStatus, UnsupportedReason};
use remote_server::transport::Error;
use warp_core::SessionId;
use warpui::{SingletonEntity, ViewContext};

use super::operation;
use crate::auth::auth_state::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::remote_server::auth_context::server_api_auth_context;
use crate::remote_server::dev_container_transport::DevContainerTransport;
use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::server::server_api::ServerApiProvider;
use crate::settings::PrivacySettings;
use crate::terminal::view::TerminalView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DevContainerRemoteSetupPhase {
    Checking,
    Installing,
    Connecting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryCheckDecision {
    Connect,
    Install,
    Unsupported,
    Failed,
}

fn unsupported_container_message(preinstall_check: Option<&PreinstallCheckResult>) -> String {
    match preinstall_check.map(|check| &check.status) {
        Some(PreinstallStatus::Unsupported {
            reason: UnsupportedReason::NonGlibc { name },
        }) => {
            format!("This container's C library ({name}) is not supported by Warp's remote server.")
        }
        Some(PreinstallStatus::Unsupported {
            reason: UnsupportedReason::GlibcTooOld { detected, required },
        }) => format!(
            "This container's glibc {detected} is older than Warp's remote server requires \
             ({required})."
        ),
        Some(PreinstallStatus::Unsupported {
            reason: UnsupportedReason::UnsupportedOs { os },
        }) => format!("This container's OS ({os}) is not supported by Warp's remote server."),
        Some(PreinstallStatus::Unsupported {
            reason: UnsupportedReason::UnsupportedArch { arch },
        }) => format!(
            "This container's architecture ({arch}) is not supported by Warp's remote server."
        ),
        Some(PreinstallStatus::Supported | PreinstallStatus::Unknown) | None => {
            "This container is not supported by Warp's remote server.".to_owned()
        }
    }
}

fn binary_check_decision(
    result: &Result<bool, Arc<Error>>,
    preinstall_check: Option<&PreinstallCheckResult>,
) -> BinaryCheckDecision {
    if matches!(
        preinstall_check,
        Some(PreinstallCheckResult {
            status: PreinstallStatus::Unsupported { .. },
            ..
        })
    ) {
        return BinaryCheckDecision::Unsupported;
    }
    match result {
        Ok(true) => BinaryCheckDecision::Connect,
        Ok(false) => BinaryCheckDecision::Install,
        Err(_) => BinaryCheckDecision::Failed,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionConnectedDecision {
    ReplaceBuildPane,
    DeregisterStale,
    Ignore,
}

fn session_connected_decision(
    setup_session_id: SessionId,
    event_session_id: SessionId,
    is_current_attempt: bool,
) -> SessionConnectedDecision {
    if setup_session_id != event_session_id {
        SessionConnectedDecision::Ignore
    } else if is_current_attempt {
        SessionConnectedDecision::ReplaceBuildPane
    } else {
        SessionConnectedDecision::DeregisterStale
    }
}

pub(crate) struct DevContainerRemoteSetup {
    session_id: SessionId,
    transport: DevContainerTransport,
    phase: DevContainerRemoteSetupPhase,
    workspace_folder: PathBuf,
    docker_path: PathBuf,
    container_id: String,
    remote_user: Option<String>,
    remote_workspace_folder: String,
    sandbox_id: String,
    operation_id: uuid::Uuid,
    attempt_id: u64,
}

impl TerminalView {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_dev_container_remote_server(
        &mut self,
        workspace_folder: PathBuf,
        docker_path: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
        session_id: SessionId,
        operation_id: uuid::Uuid,
        attempt_id: u64,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::LocalDevContainer.is_enabled() {
            self.fail_dev_container_remote_setup(
                "Dev Container remote server is disabled.".to_owned(),
                ctx,
            );
            return;
        }

        let auth_context = Arc::new(server_api_auth_context(
            AuthStateProvider::as_ref(ctx).get().clone(),
            ServerApiProvider::as_ref(ctx).get_auth_client(),
            PrivacySettings::handle(ctx)
                .as_ref(ctx)
                .is_crash_reporting_enabled,
        ));
        let transport = DevContainerTransport::new(
            docker_path.clone(),
            container_id.clone(),
            remote_user.clone(),
            remote_workspace_folder.clone(),
            auth_context,
        );
        self.dev_container_remote_setup = Some(DevContainerRemoteSetup {
            session_id,
            transport: transport.clone(),
            phase: DevContainerRemoteSetupPhase::Checking,
            workspace_folder,
            docker_path,
            container_id,
            remote_user,
            remote_workspace_folder,
            sandbox_id,
            operation_id,
            attempt_id,
        });
        if let Some(operation) = &self.dev_container_build {
            operation.update(ctx, |operation, ctx| {
                operation.set_remote_server_session_id(Some(session_id), ctx);
            });
        }
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.check_binary(session_id, transport, ctx);
        });
    }

    pub(crate) fn pending_dev_container_remote_session_id(&self) -> Option<SessionId> {
        self.dev_container_remote_setup
            .as_ref()
            .map(|setup| setup.session_id)
    }

    pub(crate) fn handle_dev_container_remote_server_event(
        &mut self,
        event: &RemoteServerManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(setup) = self.dev_container_remote_setup.as_ref() else {
            return false;
        };
        let Some(event_session_id) = event.session_id() else {
            return false;
        };
        if event_session_id != setup.session_id {
            return false;
        }
        if !self.is_current_dev_container_attempt(setup.operation_id, setup.attempt_id, ctx) {
            self.deregister_dev_container_remote_setup(ctx);
            return true;
        }

        match event {
            RemoteServerManagerEvent::BinaryCheckComplete {
                session_id,
                result,
                preinstall_check,
                ..
            } => {
                self.on_dev_container_binary_check_complete(
                    *session_id,
                    result.clone(),
                    preinstall_check.clone(),
                    ctx,
                );
                true
            }
            RemoteServerManagerEvent::BinaryInstallComplete {
                session_id, result, ..
            } => {
                self.on_dev_container_binary_install_complete(*session_id, result.clone(), ctx);
                true
            }
            RemoteServerManagerEvent::SessionConnected { session_id, .. } => {
                self.on_dev_container_session_connected(*session_id, ctx);
                true
            }
            RemoteServerManagerEvent::SessionConnectionFailed {
                session_id, error, ..
            } => {
                self.on_dev_container_session_connection_failed(*session_id, error.clone(), ctx);
                true
            }
            _ => false,
        }
    }

    fn on_dev_container_binary_check_complete(
        &mut self,
        session_id: SessionId,
        result: Result<bool, Arc<Error>>,
        preinstall_check: Option<PreinstallCheckResult>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(setup) = self.dev_container_remote_setup.as_mut() else {
            return;
        };
        if setup.session_id != session_id || setup.phase != DevContainerRemoteSetupPhase::Checking {
            return;
        }
        match binary_check_decision(&result, preinstall_check.as_ref()) {
            BinaryCheckDecision::Unsupported => self.fail_dev_container_remote_setup(
                unsupported_container_message(preinstall_check.as_ref()),
                ctx,
            ),
            BinaryCheckDecision::Connect => self.connect_dev_container_remote_server(ctx),
            BinaryCheckDecision::Install => {
                let Some(setup) = self.dev_container_remote_setup.as_mut() else {
                    return;
                };
                setup.phase = DevContainerRemoteSetupPhase::Installing;
                let transport = setup.transport.clone();
                let session_id = setup.session_id;
                RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                    mgr.install_binary(session_id, transport, false, ctx);
                });
            }
            BinaryCheckDecision::Failed => {
                let message = match result {
                    Err(err) => {
                        format!("Failed to verify the Warp remote server in the container: {err}")
                    }
                    Ok(_) => "Failed to verify the Warp remote server in the container.".to_owned(),
                };
                self.fail_dev_container_remote_setup(message, ctx);
            }
        }
    }

    fn on_dev_container_binary_install_complete(
        &mut self,
        session_id: SessionId,
        result: Result<(), Arc<Error>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(setup) = self.dev_container_remote_setup.as_ref() else {
            return;
        };
        if setup.session_id != session_id || setup.phase != DevContainerRemoteSetupPhase::Installing
        {
            return;
        }
        match result {
            Ok(()) => self.connect_dev_container_remote_server(ctx),
            Err(err) => self.fail_dev_container_remote_setup(
                format!("Failed to install the Warp remote server in the container: {err}"),
                ctx,
            ),
        }
    }

    fn connect_dev_container_remote_server(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(setup) = self.dev_container_remote_setup.as_mut() else {
            return;
        };
        setup.phase = DevContainerRemoteSetupPhase::Connecting;
        let auth_context = Arc::new(server_api_auth_context(
            AuthStateProvider::as_ref(ctx).get().clone(),
            ServerApiProvider::as_ref(ctx).get_auth_client(),
            PrivacySettings::handle(ctx)
                .as_ref(ctx)
                .is_crash_reporting_enabled,
        ));
        let transport = DevContainerTransport::new(
            setup.docker_path.clone(),
            setup.container_id.clone(),
            setup.remote_user.clone(),
            setup.remote_workspace_folder.clone(),
            auth_context.clone(),
        );
        setup.transport = transport.clone();
        let session_id = setup.session_id;
        let label = setup
            .remote_user
            .as_deref()
            .map(|user| format!("{user}@devcontainer"))
            .unwrap_or_else(|| "devcontainer".to_owned());
        RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
            mgr.connect_session(session_id, transport, auth_context, Some(label), ctx);
        });
    }

    fn on_dev_container_session_connected(
        &mut self,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(setup) = self.dev_container_remote_setup.take() else {
            return;
        };
        let is_current_attempt =
            self.is_current_dev_container_attempt(setup.operation_id, setup.attempt_id, ctx);
        match session_connected_decision(setup.session_id, session_id, is_current_attempt) {
            SessionConnectedDecision::Ignore => {
                self.dev_container_remote_setup = Some(setup);
                return;
            }
            SessionConnectedDecision::DeregisterStale => {
                RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                    mgr.deregister_session(session_id, ctx);
                });
                return;
            }
            SessionConnectedDecision::ReplaceBuildPane => {}
        }
        if let Some(operation) = self.dev_container_build.clone() {
            operation.update(ctx, |operation, ctx| {
                operation.set_phase(operation::DevContainerBuildPhase::Attach, ctx);
                operation.complete(ctx);
            });
            ctx.emit(super::super::Event::ReplaceDevContainerBuildPane {
                workspace_folder: setup.workspace_folder,
                docker_path: setup.docker_path,
                container_id: setup.container_id,
                remote_user: setup.remote_user,
                remote_workspace_folder: setup.remote_workspace_folder,
                sandbox_id: setup.sandbox_id,
                session_id: setup.session_id,
            });
            return;
        }
        self.create_and_push_dev_container(
            setup.workspace_folder,
            setup.docker_path,
            setup.container_id,
            setup.remote_user,
            setup.remote_workspace_folder,
            setup.sandbox_id,
            setup.session_id,
            ctx,
        );
    }

    fn on_dev_container_session_connection_failed(
        &mut self,
        session_id: SessionId,
        error: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(setup) = self.dev_container_remote_setup.as_ref() else {
            return;
        };
        if setup.session_id != session_id {
            return;
        }
        self.fail_dev_container_remote_setup(
            format!("Failed to start the Warp remote server in the container: {error}"),
            ctx,
        );
    }

    pub(super) fn deregister_dev_container_remote_setup(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(setup) = self.dev_container_remote_setup.take() {
            RemoteServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                mgr.deregister_session(setup.session_id, ctx);
            });
        }
    }

    fn fail_dev_container_remote_setup(&mut self, message: String, ctx: &mut ViewContext<Self>) {
        self.deregister_dev_container_remote_setup(ctx);
        self.fail_dev_container_build(operation::DevContainerBuildPhase::Staging, message, ctx);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn setup_phases_are_distinct() {
        assert_ne!(
            DevContainerRemoteSetupPhase::Checking,
            DevContainerRemoteSetupPhase::Connecting
        );
    }

    #[test]
    fn binary_check_connects_when_present() {
        assert_eq!(
            binary_check_decision(&Ok(true), None),
            BinaryCheckDecision::Connect
        );
    }

    #[test]
    fn binary_check_installs_when_missing() {
        assert_eq!(
            binary_check_decision(&Ok(false), None),
            BinaryCheckDecision::Install
        );
    }

    #[test]
    fn binary_check_fails_when_check_errors() {
        assert_eq!(
            binary_check_decision(&Err(Arc::new(Error::TimedOut)), None),
            BinaryCheckDecision::Failed
        );
    }

    #[test]
    fn binary_check_fails_closed_when_unsupported() {
        let preinstall = PreinstallCheckResult::unsupported(
            remote_server::setup::UnsupportedReason::UnsupportedOs {
                os: "plan9".to_owned(),
            },
        );
        assert_eq!(
            binary_check_decision(&Ok(true), Some(&preinstall)),
            BinaryCheckDecision::Unsupported
        );
        assert!(
            unsupported_container_message(Some(&preinstall)).contains("plan9"),
            "{}",
            unsupported_container_message(Some(&preinstall))
        );
    }

    #[test]
    fn musl_preinstall_does_not_block_install() {
        let preinstall = PreinstallCheckResult::parse(
            "required_glibc=2.31\nlibc_family=musl\nstatus=supported\n",
        );
        assert_eq!(
            binary_check_decision(&Ok(false), Some(&preinstall)),
            BinaryCheckDecision::Install
        );
        assert_eq!(
            binary_check_decision(&Ok(true), Some(&preinstall)),
            BinaryCheckDecision::Connect
        );
    }

    #[test]
    fn session_connected_replaces_pane_only_for_current_attempt() {
        let session_id = SessionId::from(3);
        assert_eq!(
            session_connected_decision(session_id, session_id, true),
            SessionConnectedDecision::ReplaceBuildPane
        );
        assert_eq!(
            session_connected_decision(session_id, session_id, false),
            SessionConnectedDecision::DeregisterStale
        );
        assert_eq!(
            session_connected_decision(session_id, SessionId::from(4), true),
            SessionConnectedDecision::Ignore
        );
    }
}
