mod in_band_command_executor;
#[cfg(feature = "local_tty")]
mod local_command_executor;
#[cfg(feature = "local_tty")]
mod msys2_command_executor;
mod tmux_executor;
#[cfg(feature = "local_tty")]
mod wsl_command_executor;
use std::collections::HashMap;
mod noop_command_executor;
#[cfg(feature = "local_tty")]
mod remote_command_executor;
#[cfg(feature = "local_tty")]
pub(crate) mod remote_server_executor;
mod shared;

use std::{any::Any, fmt::Debug, sync::Arc};

use anyhow::Result;
use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use warp_completer::completer::CommandOutput;
use warpui::ModelContext;

use crate::terminal::{
    event::ExecutedExecutorCommandEvent, model::session::Sessions, shell::Shell,
};

use super::SessionInfo;

pub use in_band_command_executor::{
    is_in_band_command, InBandCommand, InBandCommandCancelledEvent, InBandCommandExecutor,
    InBandCommandOutputReceiver,
};
#[cfg(feature = "local_tty")]
pub use local_command_executor::LocalCommandExecutor;
pub use noop_command_executor::NoOpCommandExecutor;
#[cfg(feature = "local_tty")]
pub use remote_command_executor::RemoteCommandExecutor;
pub use shared::{shell_escape_single_quotes, ExecutorCommandEvent};

#[derive(Copy, Clone, Debug)]
pub struct ExecuteCommandOptions {
    /// Whether the command must be run in the same shell as the currently running [`Session`].
    ///
    /// If false, it's an implementation detail which shell the command is run within.
    /// i.e. For unix shells, the command may be run in an `sh` shell instead of `bash` or `zsh`.
    /// On Windows, commands may be run through `cmd.exe`.
    ///
    /// ## Platform Support
    /// This field is currently only respected on Windows.
    pub run_command_in_same_shell_as_session: bool,
}

impl Default for ExecuteCommandOptions {
    fn default() -> Self {
        Self {
            run_command_in_same_shell_as_session: true,
        }
    }
}

/// Trait to be implemented by structs that execute command in context that emulates or actually is
/// identical to the active terminal session's context. `CommandExecutor` is commonly used to
/// execute generator commands to power completions, syntax highlighting, and autosuggestions.
#[async_trait]
pub trait CommandExecutor: Send + Sync + Debug {
    /// Executes the given command from the given `current_directory_path` with $PATH set to
    /// `path_env_var` and with the given environment variables.
    async fn execute_command(
        &self,
        command: &str,
        shell: &Shell,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput>;

    /// Cancels in-progress commands.
    ///
    /// Some implementations may not need to explicitly cancel command execution, so a default
    /// implementation is given.
    fn cancel_active_commands(&self) {}

    /// Allows us to downcast an executor to check what command execution method is used by a
    /// session.
    fn as_any(&self) -> &dyn Any;

    /// Whether the backing executor for the session supports execution of commands in parallel.
    fn supports_parallel_command_execution(&self) -> bool;
}

#[allow(unused_variables)]
pub fn new_command_executor_for_session(
    session_info: &SessionInfo,
    executor_command_tx: &Sender<ExecutorCommandEvent>,
    in_band_command_output_rx: Receiver<ExecutedExecutorCommandEvent>,
    parent_session_info: Option<&SessionInfo>,
    ctx: &mut ModelContext<Sessions>,
) -> Arc<dyn CommandExecutor> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "local_tty")] {
            new_command_executor_for_local_tty_session(session_info, executor_command_tx, in_band_command_output_rx, parent_session_info, ctx)
        } else if  #[cfg(feature = "remote_tty")]{
            new_command_executor_for_network_backed_pty(executor_command_tx, in_band_command_output_rx, ctx)
        } else {
            Arc::new(NoOpCommandExecutor::default())
        }
    }
}

/// Constructs a new command executor when there is a network-backed PTY (indicated by the
/// `remote_tty` feature).
#[cfg(feature = "remote_tty")]
#[cfg_attr(feature = "remote_tty", allow(dead_code))]
fn new_command_executor_for_network_backed_pty(
    executor_command_tx: &Sender<ExecutorCommandEvent>,
    in_band_command_output_rx: Receiver<ExecutedExecutorCommandEvent>,
    ctx: &mut ModelContext<Sessions>,
) -> Arc<dyn CommandExecutor> {
    log::info!("creating an in-band command executor!");
    let (in_band_command_cancelled_tx, in_band_command_cancelled_rx) = async_channel::unbounded();
    let executor = Arc::new(InBandCommandExecutor::new(
        executor_command_tx.clone(),
        in_band_command_cancelled_tx.clone(),
    ));
    let executor_clone = executor.clone();
    ctx.spawn_stream_local(
        in_band_command_output_rx,
        move |_, event, _| executor_clone.handle_executed_command_event(event),
        |_, _| {}, /* on_done */
    );
    let executor_clone = executor.clone();
    ctx.spawn_stream_local(
        in_band_command_cancelled_rx,
        move |_, event, _| executor_clone.handle_cancelled_in_band_command_event(event),
        |_, _| {}, /* on_done */
    );
    executor
}

#[cfg(feature = "local_tty")]
fn new_command_executor_for_local_tty_session(
    session_info: &SessionInfo,
    executor_command_tx: &Sender<ExecutorCommandEvent>,
    in_band_command_output_rx: Receiver<ExecutedExecutorCommandEvent>,
    parent_session_info: Option<&SessionInfo>,
    ctx: &mut ModelContext<Sessions>,
) -> Arc<dyn CommandExecutor> {
    use msys2_command_executor::MSYS2CommandExecutor;
    use remote_server_executor::RemoteServerCommandExecutor;
    use settings::Setting as _;
    use tmux_executor::TmuxCommandExecutor;
    use warpui::SingletonEntity as _;
    use wsl_command_executor::WslCommandExecutor;

    use crate::{
        features::FeatureFlag,
        remote_server::manager::RemoteServerManager,
        settings::DebugSettings,
        terminal::{
            available_shells::AvailableShells,
            model::session::{BootstrapSessionType, ShellLaunchData},
            shell::ShellType,
        },
    };

    use super::IsLegacySSHSession;

    // When the remote server feature flag is enabled and the session is a
    // legacy SSH session, use the remote server executor *if* the manager
    // already has a live `Connected` client for this session.
    //
    // By construction this branch is only reached after
    // `ModelEventDispatcher::complete_bootstrapped_session` has gated on
    // both `Bootstrapped` and the remote-server setup result
    // (`RemoteServerReady` / `RemoteServerFailed` / skipped). So
    // `client_for_session` returning `Some` corresponds to a successful
    // setup and `None` corresponds to the skip / failure paths, where we
    // fall through to the existing ControlMaster-based
    // `RemoteCommandExecutor` below. This preserves the fallback behavior
    // described in specs/APP-3797.
    if FeatureFlag::SshRemoteServer.is_enabled() {
        if let IsLegacySSHSession::Yes { .. } = &session_info.is_legacy_ssh_session {
            let session_id = session_info.session_id;
            let maybe_client = RemoteServerManager::handle(ctx)
                .read(ctx, |mgr, _| mgr.client_for_session(session_id).cloned());
            if let Some(client) = maybe_client {
                log::info!("creating a remote server executor for session {session_id:?}");
                return Arc::new(RemoteServerCommandExecutor::new(session_id, client));
            }
            log::info!(
                "SshRemoteServer flag on but no connected client for session {session_id:?}; \
                 falling back to ControlMaster executor"
            );
        }
    }

    if FeatureFlag::SSHTmuxWrapper.is_enabled()
        && session_info.tmux_control_mode
        // We don't allow nested tmux warpification, so if our parent session is already warified using
        // tmux then we shouldn't.
        && !parent_session_info.is_some_and(|s| s.tmux_control_mode)
    {
        log::info!("creating a tmux executor!");
        let executor = Arc::new(TmuxCommandExecutor::new(executor_command_tx.clone()));
        let executor_clone = executor.clone();
        ctx.spawn_stream_local(
            in_band_command_output_rx,
            move |_, event, _| executor_clone.handle_executed_command_event(event),
            |_, _| {}, /* on_done */
        );
        return executor;
    }

    let debug_settings = DebugSettings::as_ref(ctx);
    let are_in_band_generators_for_all_sessions_enabled_debug_setting = debug_settings
        .are_in_band_generators_for_all_sessions_enabled
        .value();
    let should_force_disable_in_band_generators =
        debug_settings.force_disable_in_band_generators.value();

    let is_legacy_ssh_session = matches!(
        &session_info.is_legacy_ssh_session,
        IsLegacySSHSession::Yes { .. }
    );

    let shell_needs_in_band_executor = session_info.shell.force_in_band_command_executor();
    // Docker sandbox sessions run commands inside the container; the host-side
    // LocalCommandExecutor has no way to reach into the sandbox, so generators
    // must go through the session's in-band executor (which rides on the live
    // PTY that is already attached to the container).
    //
    // TODO(advait): For production this should be a dedicated
    // `SandboxCommandExecutor` that runs generators via
    // `sbx exec warp-sandbox-<id> -- sh -c "<cmd>"` (analogous to
    // how `LocalCommandExecutor` spawns fresh subprocesses for the
    // host-shell path). That would avoid serializing generators
    // through the user's live PTY (slower, blocked by long-running
    // foreground commands, and has to dodge line-editor state) and
    // instead run them as fresh, parallelizable processes inside the
    // container — the same semantics host shells get today.
    let launch_data_needs_in_band_executor = matches!(
        session_info.launch_data,
        Some(ShellLaunchData::DockerSandbox { .. })
    );
    let force_use_in_band_generators = shell_needs_in_band_executor
        || launch_data_needs_in_band_executor
        || *are_in_band_generators_for_all_sessions_enabled_debug_setting;

    match &session_info.session_type {
        BootstrapSessionType::Local if !force_use_in_band_generators => {
            let shell_type = session_info.shell.shell_type();

            log::info!("creating a local executor!");
            match &session_info.launch_data {
                Some(ShellLaunchData::Executable {
                    executable_path, ..
                }) => Arc::new(LocalCommandExecutor::new(
                    Some(executable_path.to_owned()),
                    shell_type,
                )),
                // Docker sandbox sessions should already be routed to the
                // in-band executor via `launch_data_needs_in_band_executor`
                // above, so this arm is only reached if that routing drifts
                // (e.g. a feature flag/debug setting disables it). Rather
                // than panic in user-facing code, log loudly and fall back
                // to a no-op executor so the sandbox session still runs;
                // generators won't work but the PTY stays healthy.
                Some(ShellLaunchData::DockerSandbox { .. }) => {
                    debug_assert!(
                        false,
                        "Docker sandbox sessions should be routed through the in-band executor"
                    );
                    log::error!(
                        "Docker sandbox session reached the local-executor branch; \
                         falling back to a no-op command executor. \
                         `launch_data_needs_in_band_executor` routing may have drifted."
                    );
                    Arc::new(NoOpCommandExecutor::new())
                }
                Some(ShellLaunchData::MSYS2 {
                    executable_path, ..
                }) => {
                    let windows_native_shell_path =
                        AvailableShells::handle(ctx).read(ctx, |shells, _ctx| {
                            shells
                                .find_known_shell_by_type(ShellType::PowerShell)
                                .and_then(|powershell| powershell.get_valid_shell_path_and_type())
                                .and_then(|shell_launch_data| {
                                    if let ShellLaunchData::Executable { executable_path, .. } = shell_launch_data {
                                        Some(executable_path)
                                    } else {
                                        log::warn!("Found available shell for windows-native shell but could not get executable path");
                                        None
                                    }
                                })
                        });
                    Arc::new(MSYS2CommandExecutor::new(
                        windows_native_shell_path,
                        executable_path.to_owned(),
                    ))
                }
                Some(ShellLaunchData::WSL { distro }) => {
                    Arc::new(WslCommandExecutor::new(distro.to_owned(), shell_type))
                }
                None => {
                    if let Some(wsl_name) = session_info.wsl_name() {
                        Arc::new(WslCommandExecutor::new(wsl_name.to_owned(), shell_type))
                    } else {
                        Arc::new(LocalCommandExecutor::new(None, shell_type))
                    }
                }
            }
        }
        BootstrapSessionType::WarpifiedRemote
            if is_legacy_ssh_session
                && !FeatureFlag::InBandGeneratorsForSSH.is_enabled()
                && !force_use_in_band_generators =>
        {
            if let IsLegacySSHSession::Yes { socket_path } = &session_info.is_legacy_ssh_session {
                let wsl_distro = parent_session_info
                    .and_then(|session| session.wsl_name())
                    .map(ToOwned::to_owned);
                log::info!("creating a legacy ssh executor!");
                Arc::new(RemoteCommandExecutor::new(socket_path.clone(), wsl_distro))
            } else {
                unreachable!("Unreachable because of match! above. Unfortunately if let guards in rust are still experimental.")
            }
        }
        _ => {
            if *should_force_disable_in_band_generators {
                // The user has manually disabled in-band generators via command
                // modifying 'user defaults', so pass a no-op command executor.
                //
                // This code path exists as a fail-safe for disabling in-band
                // generators if some unforeseen severe issue surfaces during or
                // shortly after subshells launch. The setting that triggers this
                // codepath is only accessible via a user defaults command that a Warp
                // engineer would have given to the user via some first-hand
                // correspondence (e.g. GitHub issues).
                log::info!("creating a no-op executor!");
                Arc::new(NoOpCommandExecutor::new())
            } else {
                log::info!("creating an in-band command executor!");
                let (in_band_command_cancelled_tx, in_band_command_cancelled_rx) =
                    async_channel::unbounded();
                let executor = Arc::new(InBandCommandExecutor::new(
                    executor_command_tx.clone(),
                    in_band_command_cancelled_tx.clone(),
                ));
                let executor_clone = executor.clone();
                ctx.spawn_stream_local(
                    in_band_command_output_rx,
                    move |_, event, _| executor_clone.handle_executed_command_event(event),
                    |_, _| {}, /* on_done */
                );
                let executor_clone = executor.clone();
                ctx.spawn_stream_local(
                    in_band_command_cancelled_rx,
                    move |_, event, _| executor_clone.handle_cancelled_in_band_command_event(event),
                    |_, _| {}, /* on_done */
                );
                executor
            }
        }
    }
}

#[cfg(test)]
pub mod testing {
    use crate::terminal::shell::ShellType;

    use anyhow::anyhow;
    use command::r#async::Command;
    use warp_completer::completer::CommandOutput;

    use super::*;

    /// Implementation of `CommandExecutor` for use in tests. This implementation simply executes
    /// the given command in a bash subprocess.
    #[derive(Debug, Default)]
    pub struct TestCommandExecutor {}

    #[async_trait]
    impl CommandExecutor for TestCommandExecutor {
        async fn execute_command(
            &self,
            command: &str,
            shell: &Shell,
            current_directory_path: Option<&str>,
            environment_variables: Option<HashMap<String, String>>,
            _execute_command_options: ExecuteCommandOptions,
        ) -> Result<CommandOutput> {
            let mut command_process = Command::new(match shell.shell_type() {
                ShellType::PowerShell => "pwsh",
                _ => "bash",
            });

            // Set environment variables, including $PATH.
            if let Some(environment_variables) = environment_variables {
                command_process.envs(&environment_variables);
            }

            // Set the current dir, if any.
            if let Some(current_directory_path) = current_directory_path {
                command_process.current_dir(current_directory_path);
            }

            command_process
                .arg("-c")
                .arg(command)
                .output()
                .await
                .map(|output| output.into())
                .map_err(|e| anyhow!(e))
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn supports_parallel_command_execution(&self) -> bool {
            false
        }
    }
}
