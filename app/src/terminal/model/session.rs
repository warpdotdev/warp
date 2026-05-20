pub mod active_session;
pub mod command_executor;

use async_channel::Sender;
pub use command_executor::*;

use anyhow::Result;
use futures::future::{BoxFuture, Shared};
use futures::FutureExt;
use instant::Instant;
use once_cell::sync::OnceCell;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::{Debug, Display, Formatter};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use typed_path::{TypedPath, TypedPathBuf, WindowsPath};
use warp_util::path::{
    convert_msys2_to_windows_native_path, convert_wsl_to_windows_host_path, msys2_exe_to_root,
    ShellFamily,
};

use version_compare::Version;
use warp_completer::completer::{
    CommandExitStatus, CommandOutput, PathSeparators, TopLevelCommandCaseSensitivity,
};
use warpui::{platform::OperatingSystem, Entity, ModelContext, SingletonEntity};

#[cfg(feature = "local_tty")]
use crate::features::FeatureFlag;
#[cfg(feature = "local_tty")]
use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::server::telemetry::{BootstrappingInfo, TelemetryEvent};
use crate::terminal::event::ExecutedExecutorCommandEvent;
use crate::terminal::ShellHost;
use crate::terminal::ShellLaunchData;
#[cfg(feature = "local_tty")]
use command_executor::remote_server_executor::RemoteServerCommandExecutor;
use parking_lot::{Mutex, RwLock};

use crate::terminal::shell::{Shell, ShellType};
use crate::terminal::warpify::SubshellSource;
use crate::terminal::History;

use super::ansi::{BootstrappedValue, InitShellValue, SSHValue};
use super::terminal_model::{HistoryEntry, SubshellInitializationInfo};
use crate::terminal::event::RemoteServerSetupState;

#[derive(thiserror::Error, Debug)]
pub enum ReadHistoryContentsError {
    #[cfg(windows)]
    #[error("Couldn't get path to history file")]
    HistoryFilePathError,

    #[cfg(windows)]
    #[error("Error running PowerShell commands to read history file: {0}")]
    PowerShellError(anyhow::Error),

    #[cfg(windows)]
    #[error("Error running PowerShell commands and reading from filesystem to read history file. PowerShell error: {powershell_error}, filesystem error: {async_fs_error}")]
    PowerShellAndAsyncFsError {
        powershell_error: anyhow::Error,
        async_fs_error: std::io::Error,
    },

    #[error("Error reading history file from filesystem: {0}")]
    AsyncFsError(std::io::Error),
}

// SessionId is defined in warp_core and re-exported here for backward compatibility.
pub use warp_core::SessionId;

/// Information about the sessions within a given terminal pane/top-level
/// shell.
///
/// This stores multiple sessions as each bootstrapped subshell is a separate
/// session (whether it's a true subshell or an SSH session).
#[derive(Debug)]
pub struct Sessions {
    /// The start time for pending sessions, keyed by the session's
    /// unique ID.
    pending_session_start_times: HashMap<SessionId, Instant>,

    /// The set of known sessions, keyed by the session's unique ID.
    sessions: HashMap<SessionId, Arc<Session>>,

    /// The sending side of a channel used by in-band command executors.
    executor_command_tx: Sender<ExecutorCommandEvent>,

    /// The sending side of channels used to distribute the results of
    /// in-band command execution, keyed by session ID.
    in_band_command_output_tx_map: HashMap<SessionId, Sender<ExecutedExecutorCommandEvent>>,

    /// An executor to use for all spawned sessions.
    ///
    /// This is only intended to be used in tests, which may want to use
    /// various mock executor types in order to test and assert on behaviors.
    executor_for_all_sessions: Option<Arc<dyn CommandExecutor>>,

    /// Select environment variables and their values.
    env_vars: HashMap<SessionId, HashMap<String, String>>,

    /// Tracks the remote server setup state for SSH sessions that have the
    /// `SshRemoteServer` feature flag enabled. Keyed by the pending session ID.
    remote_server_setup_states: HashMap<SessionId, RemoteServerSetupState>,
}

#[derive(Clone, Debug)]
pub struct SessionBootstrappedEvent {
    pub session_id: SessionId,
    pub spawning_command: String,
    pub shell: Shell,
    pub subshell_info: Option<SubshellInitializationInfo>,
    pub session_type: BootstrapSessionType,
}

/// Set of events produced the [`Sessions`] model.
#[derive(Clone, Debug)]
pub enum SessionsEvent {
    /// The session was initialized. This does not indicate that the session has bootstrapped, but
    /// only that we're aware of the beginning of a session that we will attempt to bootstrap.
    SessionInitialized { session_id: SessionId },
    /// A new session was successfully bootstrapped.
    SessionBootstrapped(Box<SessionBootstrappedEvent>),
    /// The environment variables were updated.
    EnvironmentVariablesUpdated { session_id: SessionId },
}

impl Entity for Sessions {
    type Event = SessionsEvent;
}

impl Sessions {
    pub fn new(
        executor_command_tx: Sender<ExecutorCommandEvent>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Track the connected host_id on the `Session` type so downstream
        // code can distinguish hosts. The `RemoteServerCommandExecutor`
        // client itself is baked in at session construction time
        // (see `new_command_executor_for_local_tty_session`) so we no
        // longer need to wire it here on connect/disconnect.
        #[cfg(feature = "local_tty")]
        if FeatureFlag::SshRemoteServer.is_enabled() {
            let mgr = RemoteServerManager::handle(ctx);
            ctx.subscribe_to_model(&mgr, |sessions, event, ctx| match event {
                RemoteServerManagerEvent::SessionConnected {
                    session_id: sid,
                    host_id,
                } => {
                    if let Some(session) = sessions.sessions.get(sid) {
                        session.set_remote_host_id(Some(host_id.clone()));
                    }
                }
                RemoteServerManagerEvent::SessionDisconnected {
                    session_id: sid, ..
                } => {
                    if let Some(session) = sessions.sessions.get(sid) {
                        session.set_remote_host_id(None);
                    }
                }
                RemoteServerManagerEvent::SetupStateChanged { session_id, state } => {
                    sessions.set_remote_server_setup_state(*session_id, state.clone());
                    ctx.notify();
                }
                RemoteServerManagerEvent::BufferUpdated { .. }
                | RemoteServerManagerEvent::BufferConflictDetected { .. } => {
                    // Handled directly by GlobalBufferModel's subscription.
                }
                RemoteServerManagerEvent::SessionConnecting { .. }
                | RemoteServerManagerEvent::SessionDeregistered { .. }
                | RemoteServerManagerEvent::SessionConnectionFailed { .. }
                | RemoteServerManagerEvent::HostConnected { .. }
                | RemoteServerManagerEvent::HostDisconnected { .. }
                | RemoteServerManagerEvent::NavigatedToDirectory { .. }
                | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
                | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
                | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
                | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
                | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
                | RemoteServerManagerEvent::BinaryCheckComplete { .. }
                | RemoteServerManagerEvent::BinaryInstallComplete { .. }
                | RemoteServerManagerEvent::ClientRequestFailed { .. }
                | RemoteServerManagerEvent::ServerMessageDecodingError { .. }
                | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
                | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
                | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
                | RemoteServerManagerEvent::GetBranchesResponse { .. } => {}
                RemoteServerManagerEvent::SessionReconnected {
                    session_id: sid,
                    client,
                    ..
                } => {
                    if let Some(session) = sessions.sessions.get(sid) {
                        let new_executor =
                            Arc::new(RemoteServerCommandExecutor::new(*sid, client.clone()));
                        session.set_command_executor(new_executor);
                        log::info!("Swapped command executor for session {sid:?} after reconnect");
                    }
                }
            });
        }
        #[cfg(not(feature = "local_tty"))]
        let _ = ctx;

        Self {
            pending_session_start_times: Default::default(),
            sessions: Default::default(),
            executor_command_tx,
            in_band_command_output_tx_map: Default::default(),
            executor_for_all_sessions: None,
            env_vars: Default::default(),
            remote_server_setup_states: Default::default(),
        }
    }

    pub fn is_any_session_remote(&self) -> bool {
        self.sessions.values().any(|session| !session.is_local())
    }

    pub fn is_session_remote(&self, session_id: SessionId) -> bool {
        self.sessions
            .get(&session_id)
            .map(|session| !session.is_local())
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        let (executor_command_tx, _executor_command_rx) = async_channel::unbounded();
        Self {
            pending_session_start_times: Default::default(),
            sessions: Default::default(),
            executor_command_tx,
            in_band_command_output_tx_map: Default::default(),
            executor_for_all_sessions: None,
            env_vars: Default::default(),
            remote_server_setup_states: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn with_command_executor(mut self, executor: Arc<dyn CommandExecutor>) -> Self {
        self.executor_for_all_sessions = Some(executor);
        self
    }

    pub fn set_env_vars_for_session(
        &mut self,
        session_id: SessionId,
        env_vars: HashMap<String, String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let old_vars = self.env_vars.insert(session_id, env_vars);
        let new_vars = self.env_vars.get(&session_id);

        // We compute a list to see which env vars actually changed. If there were no
        // actual changes in env var, we do not fire an event
        let did_change = match (old_vars, new_vars) {
            (None, None) => false,
            (None, Some(new_vars)) => !new_vars.is_empty(),
            (Some(old_vars), None) => !old_vars.is_empty(),
            (Some(old_vars), Some(new_vars)) => old_vars != *new_vars,
        };
        if did_change {
            ctx.emit(SessionsEvent::EnvironmentVariablesUpdated { session_id })
        }
    }

    pub fn get_env_vars_for_session(
        &self,
        session_id: SessionId,
    ) -> Option<HashMap<String, String>> {
        self.env_vars.get(&session_id).cloned()
    }

    /// Updates the remote server setup state for the given session.
    pub fn set_remote_server_setup_state(
        &mut self,
        session_id: SessionId,
        state: RemoteServerSetupState,
    ) {
        self.remote_server_setup_states.insert(session_id, state);
    }

    /// Returns the current remote server setup state for the given session, if any.
    pub fn remote_server_setup_state(
        &self,
        session_id: SessionId,
    ) -> Option<&RemoteServerSetupState> {
        self.remote_server_setup_states.get(&session_id)
    }

    pub fn register_pending_session(
        &mut self,
        session_info: &SessionInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pending_session_start_times
            .insert(session_info.session_id, Instant::now());
        ctx.emit(SessionsEvent::SessionInitialized {
            session_id: session_info.session_id,
        })
    }

    pub fn initialize_bootstrapped_session(
        &mut self,
        session_info: SessionInfo,
        spawning_command: String,
        restored_block_commands: Vec<HistoryEntry>,
        rcfiles_duration_seconds: Option<f64>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Remove the session from the list of pending sessions.
        let pending_session_start_time = self
            .pending_session_start_times
            .remove(&session_info.session_id);

        let session_id = session_info.session_id;

        let (in_band_command_output_tx, in_band_command_output_rx) = async_channel::unbounded();
        let command_executor = if let Some(executor) = &self.executor_for_all_sessions {
            // Explicitly drop the receiver to match the other branch, where
            // it gets consumed through the function call.
            let _ = in_band_command_output_rx;
            executor.clone()
        } else {
            let parent_session_info = session_info
                .spawning_session_id
                .as_ref()
                .and_then(|session_id| self.sessions.get(session_id))
                .map(|session| &session.info);
            command_executor::new_command_executor_for_session(
                &session_info,
                &self.executor_command_tx,
                in_band_command_output_rx,
                parent_session_info,
                ctx,
            )
        };
        if !in_band_command_output_tx.is_closed() {
            // If the receiving side of the channel was stored somewhere (and
            // not just dropped), store the sending side of the channel so we
            // can proxy output to it.
            self.in_band_command_output_tx_map
                .insert(session_info.session_id, in_band_command_output_tx);
        }

        let session = Session::new(session_info.clone(), command_executor);

        log::info!("Shell is bootstrapped with session_id {:?}", session.id());
        log::debug!("Session details: {session:?}");

        let session = Arc::new(session);
        self.sessions.insert(session.id(), session.clone());

        // For warpified-remote sessions, pick up the current host_id from
        // the manager so session.remote_host_id() is populated without
        // waiting for the next SessionConnected event. The
        // RemoteServerCommandExecutor already has its client baked in, so
        // nothing else needs to be wired here.
        #[cfg(feature = "local_tty")]
        if FeatureFlag::SshRemoteServer.is_enabled()
            && matches!(
                session_info.session_type,
                BootstrapSessionType::WarpifiedRemote
            )
        {
            if let Some(host_id) = RemoteServerManager::as_ref(ctx).host_id_for_session(session_id)
            {
                session.set_remote_host_id(Some(host_id.clone()));
            }
        }

        let bootstrap_duration_seconds =
            pending_session_start_time.map(|start| start.elapsed().as_secs_f64());
        let warp_attributed_bootstrap_duration_seconds =
            match (bootstrap_duration_seconds, rcfiles_duration_seconds) {
                (Some(total), Some(rcfiles)) => Some(total - rcfiles),
                _ => None,
            };
        let was_triggered_by_rc_file = session
            .subshell_info()
            .clone()
            .map(|info| info.was_triggered_by_rc_file_snippet)
            .unwrap_or(false);

        crate::send_telemetry_from_ctx!(
            TelemetryEvent::BootstrappingSucceeded(BootstrappingInfo {
                shell: session.shell().shell_type().name(),
                shell_version: session.shell().version().clone(),
                is_ssh: session.is_legacy_ssh_session(),
                was_triggered_by_rc_file,
                is_subshell: session.subshell_info().is_some(),
                is_wsl: session.is_wsl(),
                bootstrap_duration_seconds,
                rcfiles_duration_seconds,
                warp_attributed_bootstrap_duration_seconds,
                is_msys2: session.is_msys2(),
                terminal_session_id: Some(session.id()),
            }),
            ctx
        );

        History::handle(ctx).update(ctx, |history, ctx| {
            let session_id = session.id();
            let shell_host = ShellHost::from_session(session.as_ref());

            history.init_session(session, ctx);

            let this_host_commands: Vec<_> = restored_block_commands
                .iter()
                .filter(|item| {
                    item.shell_host
                        .as_ref()
                        .is_none_or(|host| *host == shell_host)
                })
                .cloned()
                .collect();

            // Append the restored block commands at the end of history.
            history.append_restored_commands(session_id, this_host_commands);
        });

        ctx.emit(SessionsEvent::SessionBootstrapped(Box::new(
            SessionBootstrappedEvent {
                session_id,
                spawning_command,
                shell: session_info.shell,
                subshell_info: session_info.subshell_info,
                session_type: session_info.session_type,
            },
        )))
    }

    pub fn get(&self, session_id: SessionId) -> Option<Arc<Session>> {
        self.sessions.get(&session_id).map(Clone::clone)
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Returns whether we're aware of the existence of any sessions, whether
    /// they are pending or fully bootstrapped.
    pub fn has_pending_or_bootstrapped_session(&self) -> bool {
        !self.pending_session_start_times.is_empty() || !self.sessions.is_empty()
    }

    /// Returns whether the given `session_id` is tracked by this [`Sessions`]
    /// model, either as a pending session (registered via [`Self::register_pending_session`])
    /// or a fully bootstrapped one.
    pub fn tracks_session(&self, session_id: SessionId) -> bool {
        self.sessions.contains_key(&session_id)
            || self.pending_session_start_times.contains_key(&session_id)
    }

    /// Returns a map of the spawning commands for all subshell sessions, keyed the session's `SessionId`.
    pub fn spawning_command_for_subshell_sessions(&self) -> HashMap<SessionId, SubshellSource> {
        self.sessions
            .iter()
            .filter_map(|(id, session)| {
                session.subshell_info().as_ref().map(|info| {
                    (
                        *id,
                        if let Some(env_var_collection_name) = &info.env_var_collection_name {
                            SubshellSource::EnvVarCollection(env_var_collection_name.clone())
                        } else {
                            SubshellSource::Command(info.spawning_command.clone())
                        },
                    )
                })
            })
            .collect()
    }

    /// Handles an [`ExecutedExecutorCommandEvent`] by forwarding the event to
    /// the session's [`InBandCommandExecutor`].
    pub fn handle_executed_command_event(
        &mut self,
        session_id: SessionId,
        event: ExecutedExecutorCommandEvent,
    ) {
        if let Some(in_band_command_output_tx) = self.in_band_command_output_tx_map.get(&session_id)
        {
            if let Err(e) = in_band_command_output_tx.try_send(event) {
                log::error!(
                    "Failed to send ExecutedExecutorCommandEvent to InBandCommandExecutor: {e:?}"
                );
            }
        }
    }

    /// Registers a session in the map for the purpose of testing.
    ///
    /// Prefer using [`Self::initialize_bootstrapped_session`] to properly register
    /// a session (e.g. emit the appropriate events).
    #[cfg(test)]
    pub fn register_session_for_test(&mut self, session: SessionInfo) {
        use command_executor::testing::TestCommandExecutor;
        self.sessions.insert(
            session.session_id,
            Arc::new(Session::new(
                session,
                Arc::new(TestCommandExecutor::default()),
            )),
        );
    }
}

impl From<SessionType> for command_corrections::SessionType {
    fn from(session_type: SessionType) -> Self {
        match session_type {
            SessionType::WarpifiedRemote { .. } => command_corrections::SessionType::Remote,
            SessionType::Local => command_corrections::SessionType::Local,
        }
    }
}

impl From<&SessionType> for command_corrections::SessionType {
    fn from(session_type: &SessionType) -> Self {
        match session_type {
            SessionType::WarpifiedRemote { .. } => command_corrections::SessionType::Remote,
            SessionType::Local => command_corrections::SessionType::Local,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsLegacySSHSession {
    Yes { socket_path: PathBuf },
    No,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostInfo {
    // TODO(CORE-2219): This should be an enum instead of a string
    pub os_category: Option<String>,
    pub linux_distribution: Option<String>,
}

impl HostInfo {
    // TODO(CORE-2219): Once we have a struct instead of a string type,
    // we should instead implement this as either
    //   From<StructName> for command_corrections::PlatformType
    //   TryFrom<StructName for command_corrections::PlatformType
    pub fn platform_type(&self) -> command_corrections::PlatformType {
        use command_corrections::PlatformType::*;

        let Some(category) = &self.os_category else {
            return Posix;
        };
        if category == "Windows" {
            Windows
        } else {
            Posix
        }
    }
}

/// Session information sent from the shell to the Rust app after bootstrap.
///
/// This is an intermediate abstraction between the [`BootstrappedValue`] read from the pty at
/// session bootstrap and the [`Session`] model object managed by the Rust app. At session bootstrap
/// time, the `TerminalModel` constructs this from the session `BootstrappedValue` and emits it as
/// part of the `BootstrappedEvent` payload.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: SessionId,
    pub shell: Shell,
    pub launch_data: Option<ShellLaunchData>,
    pub histfile: Option<String>,
    pub user: String,
    pub hostname: String,
    pub subshell_info: Option<SubshellInitializationInfo>,
    pub path: Option<String>,
    pub environment_variable_names: HashSet<SmolStr>,
    pub aliases: HashMap<SmolStr, String>,
    pub abbreviations: HashMap<SmolStr, String>,
    // A Vec is sufficient here because function_names are guaranteed to be unique.
    pub function_names: HashSet<SmolStr>,
    pub builtins: HashSet<SmolStr>,
    pub keywords: Vec<SmolStr>,
    pub is_legacy_ssh_session: IsLegacySSHSession,
    pub home_dir: Option<String>,
    pub cdpath: Option<String>,
    pub editor: Option<String>,
    pub session_type: BootstrapSessionType,
    pub host_info: HostInfo,
    pub tmux_control_mode: bool,
    pub wsl_name: Option<String>,
    /// If this is a subshell or remote session, e.g. ssh, store the parent session ID here.
    pub spawning_session_id: Option<SessionId>,
}

impl SessionInfo {
    /// Returns a partially populated `SessionInfo` constructed from data contained in the given
    /// arguments. Most notably, this is created after the InitShell DCS hook is received, which
    /// contains the SessionId.
    ///
    /// This should be called to cache data from the InitShell value after the InitShell DCS for a
    /// newly spawned session is received.
    pub fn create_pending(
        shell_type: ShellType,
        init_shell_value: InitShellValue,
        subshell_info: Option<SubshellInitializationInfo>,
        launch_data: Option<ShellLaunchData>,
        legacy_ssh_session: Option<SSHValue>,
        is_warpified_ssh_session: bool,
        active_block_session_id: Option<SessionId>,
    ) -> Self {
        let is_legacy_ssh_session = match legacy_ssh_session {
            Some(ssh_value) => IsLegacySSHSession::Yes {
                socket_path: ssh_value.socket_path,
            },
            None => IsLegacySSHSession::No,
        };

        if launch_data.is_none() && is_legacy_ssh_session == IsLegacySSHSession::No {
            log::warn!("pending_local_shell_path was None for a local session");
        }

        // Compare the hostname of the session bootstrap payload with the hostname of the machine
        // to determine if this is a local or remote session.
        let session_type = Self::determine_session_type(
            &init_shell_value,
            is_warpified_ssh_session
                || matches!(&is_legacy_ssh_session, IsLegacySSHSession::Yes { .. }),
        );

        let spawning_session_id = if matches!(session_type, BootstrapSessionType::WarpifiedRemote)
            || subshell_info.is_some()
        {
            active_block_session_id
        } else {
            None
        };

        SessionInfo {
            session_id: init_shell_value.session_id,
            shell: Shell::new(shell_type, None, None, Default::default(), None),
            launch_data,
            user: init_shell_value.user,
            hostname: init_shell_value.hostname,
            session_type,
            subshell_info,
            is_legacy_ssh_session,
            environment_variable_names: Default::default(),
            path: None,
            home_dir: None,
            cdpath: None,
            editor: None,
            histfile: None,
            aliases: Default::default(),
            abbreviations: Default::default(),
            function_names: Default::default(),
            builtins: Default::default(),
            keywords: Default::default(),
            host_info: Default::default(),
            tmux_control_mode: false,
            wsl_name: init_shell_value.wsl_name,
            spawning_session_id,
        }
    }

    #[cfg(not(feature = "remote_tty"))]
    fn determine_session_type(
        init_shell_value: &InitShellValue,
        is_warpified_ssh_session: bool,
    ) -> BootstrapSessionType {
        match get_local_hostname() {
            Ok(local_hostname) => {
                // Ensures subshells are treated as local
                if local_hostname == init_shell_value.hostname &&
                // Ensures `ssh localhost` is treated as remote
                !is_warpified_ssh_session
                {
                    BootstrapSessionType::Local
                } else {
                    BootstrapSessionType::WarpifiedRemote
                }
            }
            Err(e) => {
                crate::report_error!(e);
                BootstrapSessionType::Local
            }
        }
    }

    #[cfg(feature = "remote_tty")]
    fn determine_session_type(
        _init_shell_value: &InitShellValue,
        _is_warpified_ssh_session: bool,
    ) -> BootstrapSessionType {
        // When the `remote_tty` feature is enabled--the session is always considered remote.
        BootstrapSessionType::WarpifiedRemote
    }

    /// Returns a fully populated [`SessionInfo`] containing data derived from the given
    /// `bootstrapped_value`. SessionId, user, and hostname are carried over from `self` since
    /// these are populated in [`Self::create_pending()`].
    ///
    /// This should be called on the pending `SessionInfo` after the session is bootstrapped and
    /// used to create the canonical `Session` object for the newly bootstrapped session.
    pub fn merge_from_bootstrapped_value(
        mut self,
        bootstrapped_value: BootstrappedValue,
        tmux_control_mode: bool,
    ) -> Self {
        // Determine the value from the bootstrap message, falling back to the cached shell type
        // (from the `InitShell` payload) if unable to parse.
        let shell_type = match ShellType::from_name(bootstrapped_value.shell.as_str()) {
            Some(value) => {
                if value != self.shell.shell_type() {
                    log::error!("Received ShellType {:?} in BootstrappedValue that conflicts with pending ShellType {:?}", value, self.shell.shell_type());
                }
                value
            }
            None => self.shell.shell_type(),
        };

        let home_dir = bootstrapped_value.home_dir;

        let aliases = bootstrapped_value
            .aliases
            .map(|alias_output| shell_type.aliases(alias_output.as_str()));

        let abbreviations = bootstrapped_value
            .abbreviations
            .map(|abbr_output| shell_type.abbreviations(abbr_output.as_str()));

        let function_names = bootstrapped_value
            .function_names
            .map(|function_names_output| function_names_output.lines().map(Into::into).collect());

        let builtins = bootstrapped_value
            .builtins
            .map(|builtins_output| builtins_output.lines().map(Into::into).collect());

        let keywords = bootstrapped_value
            .keywords
            .map(|keywords_output| keywords_output.lines().map(Into::into).collect());

        let env_var_names = bootstrapped_value.env_var_names.map(|names| {
            // In zsh the output of `echo ${(k)parameters[(R)*export*]}` is a single line separated
            // by spaces, whereas `compgen -e` in Bash and `set --names` in Fish put each env var on
            // a separate line.
            let split = match &self.shell.shell_type() {
                ShellType::Zsh | ShellType::PowerShell => names.split(' '),
                ShellType::Bash | ShellType::Fish => names.split('\n'),
            };
            split.map(Into::into).collect::<HashSet<_>>()
        });

        let options = if bootstrapped_value
            .vi_mode_enabled
            .is_some_and(|vi_mode| vi_mode.eq("1"))
        {
            let mut opts = bootstrapped_value.shell_options.unwrap_or_default();
            opts.insert("vi_mode".into());
            Some(opts)
        } else {
            bootstrapped_value.shell_options
        };

        SessionInfo {
            session_id: self.session_id,
            shell: Shell::new(
                shell_type,
                bootstrapped_value.shell_version,
                options,
                bootstrapped_value.shell_plugins.unwrap_or_default(),
                bootstrapped_value.shell_path,
            ),
            launch_data: self.launch_data.take(),
            histfile: bootstrapped_value.histfile,
            user: self.user,
            hostname: self.hostname,
            session_type: self.session_type,
            path: bootstrapped_value.path,
            environment_variable_names: env_var_names.unwrap_or_default(),
            aliases: aliases.unwrap_or_default(),
            abbreviations: abbreviations.unwrap_or_default(),
            function_names: function_names.unwrap_or_default(),
            builtins: builtins.unwrap_or_default(),
            keywords: keywords.unwrap_or_default(),
            home_dir,
            cdpath: bootstrapped_value.cdpath,
            editor: bootstrapped_value.editor,
            is_legacy_ssh_session: self.is_legacy_ssh_session,
            subshell_info: self.subshell_info.take(),
            host_info: HostInfo {
                os_category: bootstrapped_value.os_category,
                linux_distribution: bootstrapped_value.linux_distribution,
            },
            tmux_control_mode,
            wsl_name: bootstrapped_value.wsl_name,
            spawning_session_id: self.spawning_session_id,
        }
    }

    /// Returns the name of the WSL distribution, or `None` if this session is not a WSL session.
    fn wsl_name(&self) -> Option<&str> {
        self.wsl_name
            .as_deref()
            .or(self
                .launch_data
                .as_ref()
                .and_then(|launch_data| match launch_data {
                    ShellLaunchData::WSL { distro } => Some(distro.as_str()),
                    _ => None,
                }))
    }

    /// If the path is for a session inside some emulation layer, like a VM for WSL, convert a
    /// paths from inside the session into something the native host can use. Otherwise, leave the
    /// path as-is.
    pub fn maybe_convert_to_native_path(&self, path: &TypedPath) -> anyhow::Result<PathBuf> {
        if let Some(distro) = self.wsl_name() {
            return Ok(convert_wsl_to_windows_host_path(path, distro)?);
        }
        if let Some(ShellLaunchData::MSYS2 {
            executable_path, ..
        }) = &self.launch_data
        {
            return Ok(convert_msys2_to_windows_native_path(
                path,
                &msys2_exe_to_root(WindowsPath::new(
                    executable_path.as_os_str().as_encoded_bytes(),
                )),
            )?);
        }
        PathBuf::try_from(path.to_path_buf())
            .map_err(|path| anyhow::anyhow!("Unable to convert path: {path:?}"))
    }
}

/// The session type determined at bootstrap time.
///
/// Unlike [`SessionType`], this does not carry a `host_id` because that
/// information is not available until the remote-server handshake completes,
/// which happens *after* the session is bootstrapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootstrapSessionType {
    /// The session host is the same host where Warp is running.
    Local,

    /// The session host is a different host from where Warp is running.
    WarpifiedRemote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionType {
    /// The session host is the same host where Warp is running.
    Local,

    /// The session host is a different host from where Warp is running.
    /// Note that we only know this for sure when we Warpify a block.
    ///
    /// `host_id` is `Some` when the remote server feature flag is enabled and
    /// `RemoteServerManager` has completed the connection handshake. It is
    /// `None` when the feature flag is off or the connection hasn't been
    /// established yet.
    WarpifiedRemote { host_id: Option<warp_core::HostId> },
}

impl From<BootstrapSessionType> for SessionType {
    fn from(bst: BootstrapSessionType) -> Self {
        match bst {
            BootstrapSessionType::Local => SessionType::Local,
            BootstrapSessionType::WarpifiedRemote => SessionType::WarpifiedRemote { host_id: None },
        }
    }
}

/// Represents session state and context, mostly populated and constructed at session bootstrap
/// time.
///
/// This object exposes methods for querying the session context (e.g. history, aliases, executable
/// names) as well as command execution, in which an arbitrary shell command may be executed in a
/// context similar to that of the session (e.g. with the same environment variable definitions,
/// $PATH, working directory, etc).
#[derive(Debug)]
pub struct Session {
    info: SessionInfo,
    external_commands: Arc<OnceCell<HashSet<SmolStr>>>,
    /// The command executor for this session. Behind a `RwLock` so it can be
    /// swapped after a remote server reconnect (via `set_command_executor`).
    command_executor: RwLock<Arc<dyn CommandExecutor>>,
    load_external_commands_future: OnceCell<Shared<BoxFuture<'static, ()>>>,
    command_case_sensitivity: TopLevelCommandCaseSensitivity,
    /// The authoritative session type, initially derived from the
    /// [`BootstrapSessionType`] in `SessionInfo` and updated by [`Sessions`]
    /// when `RemoteServerManager` reports a connected session (to fill in the
    /// `host_id`). Interior mutability allows updating through `Arc<Session>`.
    session_type: Mutex<SessionType>,
}

impl Session {
    pub fn new(session_info: SessionInfo, command_executor: Arc<dyn CommandExecutor>) -> Self {
        if let Some(version) = &session_info.shell.version() {
            log::info!("Parsed shell version string: {:?}", Version::from(version));
        }
        let command_case_sensitivity = session_info
            .host_info
            .os_category
            .as_deref()
            .map(TopLevelCommandCaseSensitivity::from_os_category)
            .unwrap_or_else(|| OperatingSystem::get().into());

        let session_type = SessionType::from(session_info.session_type.clone());
        Self {
            info: session_info,
            external_commands: Arc::new(OnceCell::new()),
            command_executor: RwLock::new(command_executor),
            load_external_commands_future: Default::default(),
            command_case_sensitivity,
            session_type: Mutex::new(session_type),
        }
    }

    pub fn id(&self) -> SessionId {
        self.info.session_id
    }

    pub fn user(&self) -> &str {
        self.info.user.as_str()
    }

    pub fn hostname(&self) -> &str {
        self.info.hostname.as_str()
    }

    pub fn session_type(&self) -> SessionType {
        self.session_type.lock().clone()
    }

    /// Updates the `host_id` on a `WarpifiedRemote` session type after the
    /// remote server handshake completes (or clears it on disconnect).
    pub fn set_remote_host_id(&self, host_id: Option<warp_core::HostId>) {
        let mut st = self.session_type.lock();
        if let SessionType::WarpifiedRemote { host_id: ref mut h } = *st {
            *h = host_id;
        }
    }

    pub fn shell_family(&self) -> ShellFamily {
        self.shell().shell_type().into()
    }

    pub fn launch_data(&self) -> Option<&ShellLaunchData> {
        self.info.launch_data.as_ref()
    }

    pub fn maybe_convert_to_native_path(&self, path: &TypedPath) -> anyhow::Result<PathBuf> {
        self.info.maybe_convert_to_native_path(path)
    }

    pub fn path_separators(&self) -> PathSeparators {
        match self.shell().shell_type() {
            ShellType::Zsh | ShellType::Bash | ShellType::Fish => PathSeparators::for_unix(),
            ShellType::PowerShell => PathSeparators::for_os(),
        }
    }

    pub fn home_dir(&self) -> Option<&str> {
        if cfg!(test) {
            return warp_util::path::TEST_SESSION_HOME_DIR.as_deref();
        }

        self.info.home_dir.as_deref()
    }

    pub fn editor(&self) -> Option<&str> {
        self.info.editor.as_deref()
    }

    pub fn cdpath(&self) -> Option<&str> {
        self.info.cdpath.as_deref()
    }

    pub fn host_info(&self) -> HostInfo {
        self.info.host_info.clone()
    }

    pub fn is_legacy_ssh_session(&self) -> bool {
        matches!(
            self.info.is_legacy_ssh_session,
            IsLegacySSHSession::Yes { .. }
        )
    }

    pub fn is_subshell_or_ssh(&self) -> bool {
        matches!(self.session_type(), SessionType::WarpifiedRemote { .. })
            || self.is_legacy_ssh_session()
            || self.subshell_info().is_some()
    }

    pub fn is_wsl(&self) -> bool {
        self.info.wsl_name().is_some()
    }

    pub fn wsl_distro_name(&self) -> Option<&str> {
        self.info.wsl_name()
    }

    pub fn is_msys2(&self) -> bool {
        matches!(self.launch_data(), Some(ShellLaunchData::MSYS2 { .. }))
    }

    /// Returns the function that converts a Windows-native path into this session's native
    /// representation, or `None` when no conversion is appropriate.
    pub fn windows_path_converter(&self) -> Option<fn(&str) -> String> {
        if self.is_wsl() {
            Some(warp_util::path::convert_windows_path_to_wsl)
        } else if self.is_msys2() {
            Some(warp_util::path::convert_windows_path_to_msys2)
        } else {
            None
        }
    }

    pub fn alias_names(&self) -> impl Iterator<Item = &str> {
        self.info.aliases.keys().map(Deref::deref)
    }

    pub fn builtin_names(&self) -> impl Iterator<Item = &str> {
        self.info.builtins.iter().map(Deref::deref)
    }

    pub fn function_names(&self) -> impl Iterator<Item = &str> {
        self.info.function_names.iter().map(Deref::deref)
    }

    pub fn executable_names(&self) -> impl Iterator<Item = &str> {
        self.external_commands
            .get()
            .into_iter()
            .flatten()
            .map(Deref::deref)
    }

    pub fn aliases(&self) -> &HashMap<SmolStr, String> {
        &self.info.aliases
    }

    pub fn alias_value(&self, name: &str) -> Option<&str> {
        self.info.aliases.get(name).map(Deref::deref)
    }

    pub fn abbreviations(&self) -> &HashMap<SmolStr, String> {
        &self.info.abbreviations
    }

    pub fn abbreviation_value(&self, name: &str) -> Option<&str> {
        self.info.abbreviations.get(name).map(Deref::deref)
    }

    pub fn functions(&self) -> &HashSet<SmolStr> {
        &self.info.function_names
    }

    pub fn builtins(&self) -> &HashSet<SmolStr> {
        &self.info.builtins
    }

    pub fn subshell_info(&self) -> &Option<SubshellInitializationInfo> {
        &self.info.subshell_info
    }

    /// Replaces the command executor for this session. Used after a remote
    /// server reconnect to swap in a new `RemoteServerCommandExecutor`
    /// backed by the reconnected client.
    pub fn set_command_executor(&self, executor: Arc<dyn CommandExecutor>) {
        *self.command_executor.write() = executor;
    }

    /// Returns true if the session is employing in-band command execution to run generators.
    pub fn is_using_in_band_command_execution(&self) -> bool {
        self.command_executor
            .read()
            .as_ref()
            .as_any()
            .downcast_ref::<InBandCommandExecutor>()
            .is_some()
    }

    /// Returns `true` if already attempted to load external commands for the `Session`.
    pub fn has_attempted_to_load_external_commands(&self) -> bool {
        self.load_external_commands_future.get().is_some()
    }

    /// Returns `true` if external commands have finished loading for this `Session`.
    pub fn has_loaded_external_commands(&self) -> bool {
        self.external_commands.get().is_some()
    }

    /// Asynchronously loads the external commands.
    ///
    /// If this is called while a previous call to `load_external_commands` is
    /// still running, this will not resolve until the previous call does.  This
    /// means it is safe to call `load_external_commands` multiple times, and
    /// that when a call resolves, there is a guarantee that external commands
    /// have been loaded.
    ///
    /// Note that we load executables post-bootstrap because we don't need an interactive, login shell
    /// to get them (unlike aliases, functions and env-vars). All we need is the user's $PATH var,
    /// which we have access to at this point.
    pub async fn load_external_commands(&self) {
        let (load_future, receiver) = (async {
            let shell = self.info.shell.clone();
            let external_commands = self.external_commands.clone();
            let shell_command_to_get_executables =
                shell.shell_type().shell_command_to_get_executables();
            let env_vars = self
                .info
                .path
                .as_deref()
                .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));

            let result = self
                .execute_command(
                    shell_command_to_get_executables,
                    None,
                    env_vars,
                    ExecuteCommandOptions::default(),
                )
                .await;

            let is_msys2 =
                self.info.launch_data.as_ref().is_some_and(|launch_data| {
                    matches!(launch_data, ShellLaunchData::MSYS2 { .. })
                });
            // We gather the external Windows-specific commands by using PowerShell because
            // Git Bash's `compgen` is slow at gathering these. The Git Bash-specific commands
            // like ls.exe are retrieved above.
            let mut new_commands = if is_msys2 {
                let env_vars = self
                    .info
                    .path
                    .as_deref()
                    .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));
                let executor = self.command_executor.read().clone();
                let windows_results = executor
                    .execute_command(
                        ShellType::PowerShell.shell_command_to_get_executables(),
                        &Shell::new(ShellType::PowerShell, None, None, Default::default(), None),
                        None,
                        env_vars,
                        ExecuteCommandOptions::default(),
                    )
                    .await;
                HashSet::from_iter(
                    ShellType::PowerShell
                        .executables_from_shell_command_output(
                            windows_results,
                            false, /* is_msys2 */
                        )
                        .into_iter(),
                )
            } else {
                HashSet::new()
            };
            new_commands.extend(
                shell
                    .shell_type()
                    .executables_from_shell_command_output(result, is_msys2)
                    .into_iter(),
            );
            if external_commands.set(new_commands).is_err() {
                log::warn!("External commands should only be loaded once per session.");
            }
        })
        .remote_handle();

        match self
            .load_external_commands_future
            .try_insert(receiver.boxed().shared())
        {
            Ok(_) => load_future.await,
            Err((existing_receiver, _)) => existing_receiver.clone().await,
        };
    }

    /// All of the top-level commands within this session. This includes executables on the
    /// user's PATH and aliases.
    pub fn top_level_commands(&self) -> impl Iterator<Item = &str> {
        self.external_commands
            .get()
            .into_iter()
            .flatten()
            .chain(&self.info.function_names)
            .chain(self.info.aliases.keys())
            .chain(self.info.abbreviations.keys())
            .chain(&self.info.builtins)
            .chain(&self.info.keywords)
            .map(Deref::deref)
    }

    pub fn path(&self) -> &Option<String> {
        &self.info.path
    }

    pub fn histfile(&self) -> &Option<String> {
        &self.info.histfile
    }

    pub fn shell(&self) -> &Shell {
        &self.info.shell
    }

    pub fn is_local(&self) -> bool {
        self.session_type() == SessionType::Local
    }

    async fn read_history_for_local_session(&self, is_kaspersky_running: bool) -> Vec<String> {
        let histfile = &self.info.histfile;
        let shell_type = &self.info.shell.shell_type();
        let history_files = histfile.as_ref().map_or_else(
            || shell_type.history_files(),
            |histfile| vec![histfile.to_string()],
        );

        for history_file in history_files {
            let typed_path = TypedPath::from(history_file.as_str());
            let Ok(history_file) = self.maybe_convert_to_native_path(&typed_path) else {
                continue;
            };
            if history_file.exists() {
                log::info!(
                    "Loading history from file {} for shell {}",
                    history_file.display(),
                    shell_type.name()
                );

                let contents = match Self::read_history_contents(
                    history_file.as_path(),
                    *shell_type,
                    is_kaspersky_running,
                )
                .await
                {
                    Ok(contents) => contents,
                    Err(e) => {
                        log::error!("Failed to read history contents for file: {e:?}");
                        continue;
                    }
                };

                let history = shell_type.parse_history(&contents);
                return history;
            }
        }
        log::info!(
            "No history file found for shell {}, starting with empty history",
            shell_type.name()
        );
        Vec::new()
    }

    #[cfg_attr(not(windows), allow(unused_variables))]
    async fn read_history_contents(
        history_file: &Path,
        shell_type: ShellType,
        is_kaspersky_running: bool,
    ) -> Result<Vec<u8>, ReadHistoryContentsError> {
        #[cfg(windows)]
        if shell_type == ShellType::PowerShell {
            return Self::read_powershell_history_contents(history_file, is_kaspersky_running)
                .await;
        }

        async_fs::read(history_file)
            .await
            .map_err(ReadHistoryContentsError::AsyncFsError)
    }

    /// Read the PowerShell history contents by running a PowerShell command and reading the output.
    ///
    /// This is a workaround as reading the history file using [`async_fs::read`] on Windows is a
    /// trigger for certain antivirus software (Kaspersky).
    #[cfg(windows)]
    async fn read_powershell_history_contents(
        history_file: &Path,
        is_kaspersky_running: bool,
    ) -> Result<Vec<u8>, ReadHistoryContentsError> {
        let Some(history_file_path) = history_file.as_os_str().to_str() else {
            return Err(ReadHistoryContentsError::HistoryFilePathError);
        };

        // Try reading the history file using PowerShell commands first.
        let powershell_error = match Self::read_history_via_powershell(history_file_path).await {
            Ok(result) => return Ok(result),
            Err(e) => e,
        };

        // If Kaspersky is running, early return since we can't use [`async_fs`] to read the history
        // file.
        if is_kaspersky_running {
            return Err(ReadHistoryContentsError::PowerShellError(powershell_error));
        }

        // Otherwise, fall back to using [`async_fs`] to read the history file.
        match async_fs::read(history_file).await {
            Ok(contents) => {
                // Report this error so we have some data on whether this method of running
                // PowerShell commands is reliable. If this turns out to be noisy, we can remove
                // this log line.
                log::warn!(
                    "Failed to read history using PowerShell commands: {powershell_error:?}"
                );
                #[cfg(feature = "crash_reporting")]
                sentry::with_scope(
                    |scope| {
                        let mut context = std::collections::BTreeMap::new();
                        context.insert(
                            "powershell_error".to_string(),
                            format!("{powershell_error:?}").into(),
                        );
                        scope.set_context(
                            "powershell_history",
                            sentry::protocol::Context::Other(context),
                        );
                    },
                    || {
                        sentry::capture_message(
                            "Failed to read history using PowerShell commands",
                            sentry::Level::Error,
                        )
                    },
                );
                Ok(contents)
            }
            Err(e) => Err(ReadHistoryContentsError::PowerShellAndAsyncFsError {
                powershell_error,
                async_fs_error: e,
            }),
        }
    }

    #[cfg(windows)]
    async fn read_history_via_powershell(history_file_path: &str) -> Result<Vec<u8>> {
        let Some(powershell_command) = crate::util::windows::any_powershell_path() else {
            return Err(anyhow::anyhow!(
                "Failed to find powershell executable to read history"
            ));
        };

        let read_result = command::r#async::Command::new(powershell_command)
            .arg("-NoProfile")
            .arg("-NoLogo")
            .arg("-Command")
            .arg(format!(
                "[System.IO.File]::ReadAllText('{history_file_path}')"
            ))
            .output()
            .await;
        match read_result {
            Ok(output) if output.status.success() => Ok(output.stdout),
            Ok(output) => Err(anyhow::anyhow!(
                "Command to read history file failed with stderr: {:#}",
                String::from_utf8_lossy(&output.stderr)
            )),
            Err(e) => Err(anyhow::anyhow!(
                "Failed to execute command to read history file: {:#}",
                e
            )),
        }
    }

    async fn read_history_for_remote_session(&self) -> Vec<String> {
        let histfile = &self.info.histfile;
        let shell_type = self.info.shell.shell_type();

        let history_files = histfile.as_ref().map_or_else(
            || shell_type.history_files(),
            |histfile| vec![histfile.to_string()],
        );

        for history_file in history_files {
            if let Some(command_history) = self.read_history_from_file(history_file.as_str()).await
            {
                return command_history;
            }
        }

        log::info!(
            "No history file found for shell {}, starting with empty history",
            shell_type.name()
        );
        Vec::new()
    }

    async fn read_history_from_file(&self, history_file: &str) -> Option<Vec<String>> {
        let env_vars = self
            .info
            .path
            .as_deref()
            .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));

        let output_in_bytes = self
            .execute_command(
                format!("cat {history_file}").as_str(),
                None,
                env_vars,
                ExecuteCommandOptions::default(),
            )
            .await
            .ok()?;

        match output_in_bytes.status {
            CommandExitStatus::Success => {
                log::info!("Successfully parsed history file");
                Some(
                    self.info
                        .shell
                        .shell_type()
                        .parse_history(output_in_bytes.output()),
                )
            }
            CommandExitStatus::Failure => {
                log::error!("Failed to parse history file from file");
                None
            }
        }
    }

    pub async fn read_history(&self, is_kaspersky_running: bool) -> Vec<String> {
        match self.info.session_type {
            BootstrapSessionType::Local => {
                self.read_history_for_local_session(is_kaspersky_running)
                    .await
            }
            BootstrapSessionType::WarpifiedRemote => self.read_history_for_remote_session().await,
        }
    }

    pub fn environment_variable_names(&self) -> &HashSet<SmolStr> {
        &self.info.environment_variable_names
    }

    #[cfg(feature = "integration_tests")]
    pub fn external_commands(&self) -> Arc<OnceCell<HashSet<SmolStr>>> {
        self.external_commands.clone()
    }

    /// Returns a reference to the session's command executor for integration
    /// test assertions (e.g. to verify `RemoteServerCommandExecutor` is wired).
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn command_executor(&self) -> Arc<dyn CommandExecutor> {
        self.command_executor.read().clone()
    }

    pub async fn execute_command(
        &self,
        command: &str,
        current_dir_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        // Clone the Arc out of the lock so we don't hold the read guard
        // across the await point.
        let executor = self.command_executor.read().clone();
        executor
            .execute_command(
                command,
                &self.info.shell,
                current_dir_path,
                environment_variables,
                execute_command_options,
            )
            .await
    }

    /// Whether the backing executor for the session supports execution of commands in parallel.
    pub fn supports_parallel_command_execution(&self) -> bool {
        self.command_executor
            .read()
            .supports_parallel_command_execution()
    }

    pub fn cancel_active_commands(&self) {
        self.command_executor.read().cancel_active_commands();
    }

    pub async fn git_branches_for_command_corrections(&self, working_dir: &str) -> Vec<String> {
        let env_vars = self
            .info
            .path
            .as_deref()
            .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));

        let output = self
            .execute_command(
                "git --no-optional-locks branch --no-color",
                Some(working_dir),
                env_vars,
                ExecuteCommandOptions::default(),
            )
            .await;

        match output {
            Ok(command_output) if command_output.status == CommandExitStatus::Success => {
                let Ok(output_string) = command_output.to_string() else {
                    log::warn!(
                        "the output for git_branches_for_command_corrections was unparseable"
                    );
                    return vec![];
                };
                let res = output_string
                    .lines()
                    .map(|s| s.trim().to_string())
                    .collect();
                res
            }
            _ => {
                log::warn!("failed to get git_branches_for_command_corrections");
                vec![]
            }
        }
    }

    pub fn command_case_sensitivity(&self) -> TopLevelCommandCaseSensitivity {
        self.command_case_sensitivity
    }

    /// Converts the given directory into a [`typed_path::TypedPathBuf`].
    pub fn convert_directory_to_typed_path_buf(&self, pwd: String) -> TypedPathBuf {
        // We need to determine whether this session requires windows file paths
        // or unix file paths. This needs to be resilient to warpified ssh. Some examples:
        // - bash on mac ---> unix
        // - powershell on linux ---> unix
        // - powershell on windows ---> windows
        // - wsl on windows ---> unix
        // - warpified zsh --> unix

        // If the host architecture is unix, we can infer unix file paths. This would break
        // if we supported warpifying a powershell-on-windows SSH session.
        if cfg!(unix) {
            return TypedPathBuf::from_unix(pwd);
        }

        // We assume that we're on Windows.
        match self.shell_family() {
            // Cases: WSL, MSYS2, warpified bash
            ShellFamily::Posix => TypedPathBuf::from_unix(pwd),
            // Cases: powershell sessions
            ShellFamily::PowerShell => TypedPathBuf::from_windows(pwd),
        }
    }
}

impl Display for Session {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "shell: {}, histfile: {:?}, user: {}, host_machine: {:?}",
            self.info.shell.shell_type().name(),
            self.info.histfile,
            self.info.user,
            self.info.hostname,
        )
    }
}

/// Returns the hostname for the local machine where Warp is running.
pub fn get_local_hostname() -> Result<String> {
    cfg_if::cfg_if! {
        if #[cfg(not(target_family = "wasm"))] {
            use gethostname::gethostname;

            gethostname()
                .into_string()
                .map_err(|os_string| {
                    anyhow::anyhow!("Failed to convert local hostname OsString {os_string:?} into String.")
                })
        } else {
            anyhow::bail!("Cannot get machine hostname from wasm")
        }
    }
}

#[cfg(test)]
pub mod testing {
    use super::{command_executor::testing::TestCommandExecutor, *};

    /// Builder methods for constructing `SessionInfo` in tests.
    impl SessionInfo {
        pub fn new_for_test() -> Self {
            let path = std::env::var_os("PATH").unwrap().into_string().ok();

            #[cfg(unix)]
            let shell_type = ShellType::Bash;
            #[cfg(windows)]
            let shell_type = ShellType::PowerShell;

            Self {
                session_id: SessionId::from(0),
                shell: Shell::new(shell_type, None, None, Default::default(), None),
                launch_data: None,
                histfile: None,
                user: "local:user".to_owned(),
                hostname: "local:host".to_owned(),
                session_type: BootstrapSessionType::Local,
                subshell_info: None,
                path,
                editor: None,
                environment_variable_names: HashSet::new(),
                aliases: HashMap::new(),
                abbreviations: HashMap::new(),
                function_names: HashSet::new(),
                builtins: HashSet::new(),
                keywords: Vec::new(),
                is_legacy_ssh_session: IsLegacySSHSession::No,
                home_dir: None,
                cdpath: None,
                host_info: Default::default(),
                tmux_control_mode: false,
                wsl_name: None,
                spawning_session_id: None,
            }
        }

        pub fn with_aliases(mut self, aliases: HashMap<SmolStr, String>) -> Self {
            self.aliases = aliases;
            self
        }

        pub fn with_abbreviations(mut self, abbreviations: HashMap<SmolStr, String>) -> Self {
            self.abbreviations = abbreviations;
            self
        }

        pub fn with_builtins(mut self, builtins: HashSet<SmolStr>) -> Self {
            self.builtins = builtins;
            self
        }

        pub fn with_function_names(mut self, function_names: HashSet<SmolStr>) -> Self {
            self.function_names = function_names;
            self
        }

        pub fn with_histfile(mut self, histfile: Option<String>) -> Self {
            self.histfile = histfile;
            self
        }

        pub fn with_user(mut self, user: String) -> Self {
            self.user = user;
            self
        }

        pub fn with_session_type(mut self, session_type: BootstrapSessionType) -> Self {
            self.session_type = session_type;
            self
        }

        pub fn with_hostname(mut self, hostname: String) -> Self {
            self.hostname = hostname;
            self
        }

        pub fn with_home_dir(mut self, home_dir: String) -> Self {
            self.home_dir = Some(home_dir);
            self
        }

        pub fn with_cdpath(mut self, cdpath: String) -> Self {
            self.cdpath = Some(cdpath);
            self
        }

        pub fn with_id(mut self, id: impl Into<SessionId>) -> Self {
            self.session_id = id.into();
            self
        }

        pub fn with_ssh_socket_path(mut self, socket_path: PathBuf) -> Self {
            if let BootstrapSessionType::Local = self.session_type {
                self.session_type = BootstrapSessionType::WarpifiedRemote;
            }
            self.is_legacy_ssh_session = IsLegacySSHSession::Yes { socket_path };
            self
        }

        pub fn with_keywords(mut self, keywords: Vec<SmolStr>) -> Self {
            self.keywords = keywords;
            self
        }

        pub fn with_path(mut self, path: Option<String>) -> Self {
            self.path = path;
            self
        }

        pub fn with_environment_variable_names(
            mut self,
            environment_variable_names: HashSet<SmolStr>,
        ) -> Self {
            self.environment_variable_names = environment_variable_names;
            self
        }

        pub fn with_shell_type(mut self, shell_type: ShellType) -> Self {
            self.shell = Shell::new(
                shell_type,
                self.shell.version().clone(),
                self.shell.options().clone(),
                self.shell.plugins().clone(),
                self.shell.shell_path().clone(),
            );
            self
        }

        pub fn with_shell_options(mut self, shell_options: HashSet<String>) -> Self {
            self.shell = Shell::new(
                self.shell.shell_type(),
                self.shell.version().clone(),
                Some(shell_options),
                self.shell.plugins().clone(),
                self.shell.shell_path().clone(),
            );
            self
        }
    }

    impl Session {
        pub fn test() -> Self {
            let info = SessionInfo::new_for_test();
            let session_type = SessionType::from(info.session_type.clone());
            Self {
                info,
                external_commands: Default::default(),
                command_executor: RwLock::new(Arc::new(TestCommandExecutor::default())),
                load_external_commands_future: Default::default(),
                command_case_sensitivity: TopLevelCommandCaseSensitivity::CaseSensitive,
                session_type: Mutex::new(session_type),
            }
        }

        pub fn test_remote() -> Self {
            let info = SessionInfo::new_for_test()
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_shell_type(ShellType::Bash); // We only support UNIX-based remote sessions.
            let session_type = SessionType::from(info.session_type.clone());
            Self {
                info,
                external_commands: Default::default(),
                command_executor: RwLock::new(Arc::new(TestCommandExecutor::default())),
                load_external_commands_future: Default::default(),
                command_case_sensitivity: TopLevelCommandCaseSensitivity::CaseSensitive,
                session_type: Mutex::new(session_type),
            }
        }

        pub fn set_shell_options(&mut self, options: Option<HashSet<String>>) {
            self.info.shell = Shell::new(
                self.info.shell.shell_type(),
                self.info.shell.version().clone(),
                options,
                self.info.shell.plugins().clone(),
                self.info.shell.shell_path().clone(),
            );
        }

        pub fn set_external_commands(&self, commands: impl IntoIterator<Item = impl AsRef<str>>) {
            if self
                .external_commands
                .set(external_commands_with_values(commands))
                .is_err()
            {
                log::warn!("Ignored call to set_external_commands, as external commands had already been set!");
            };
        }

        pub fn set_environment_variables(
            &mut self,
            env_vars: impl IntoIterator<Item = impl Into<SmolStr>>,
        ) {
            self.info.environment_variable_names =
                HashSet::from_iter(env_vars.into_iter().map(Into::into));
        }

        pub fn with_shell_launch_data(mut self, launch_data: ShellLaunchData) -> Self {
            self.info.launch_data = Some(launch_data);
            self
        }
    }

    fn external_commands_with_values(
        values: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> HashSet<SmolStr> {
        values
            .into_iter()
            .map(|item| item.as_ref().into())
            .collect()
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod test;
