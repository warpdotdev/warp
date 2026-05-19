use anyhow::{anyhow, Result};
#[cfg(feature = "local_tty")]
use futures::future::{BoxFuture, FutureExt};
use std::{collections::HashMap, path::PathBuf, process::Stdio};

#[cfg(feature = "local_tty")]
use super::model::session::LocalCommandExecutor;
use super::{local_tty::shell::ShellStarter, shell::ShellType};
use crate::terminal::available_shells::AvailableShells;
use crate::terminal::local_tty::shell::ShellStarterSourceOrWslName;
#[cfg(feature = "local_tty")]
use command::r#async::Command;
use warpui::{Entity, ModelContext, SingletonEntity};

#[derive(Debug)]
pub enum LocalShellState {
    /// The shell for a session has been loaded.
    Loaded(LocalShell),
    /// The shell state for a local session has not been loaded.
    /// Loading shell information for WSL is extremely expensive and can take upwards of 10s to
    /// load.
    NotLoaded,
}

/// State of the interactive shell environment capture.
/// This is used for LSP operations that need the full interactive PATH.
#[cfg(feature = "local_tty")]
#[derive(Debug, Default)]
pub enum InteractiveEnvState {
    /// Interactive PATH has not been requested yet.
    #[default]
    NotRequested,
    /// Interactive PATH capture is in progress.
    /// Stores senders that will be notified when capture completes.
    Pending {
        waiters: Vec<async_channel::Sender<Option<String>>>,
    },
    /// Interactive PATH capture completed.
    Ready(Option<String>),
}

#[derive(Debug)]
pub struct LocalShell {
    shell_type: ShellType,
    /// Defines the path of the shell binary, i.e. /bin/zsh
    shell_path: PathBuf,
    /// The PATH sourced from the user's shell (non-interactive, fast)
    path_env_var: Option<String>,
    /// The PATH sourced from an interactive login shell (lazy, for LSP)
    #[cfg(feature = "local_tty")]
    interactive_env_state: InteractiveEnvState,
}

impl LocalShell {
    pub fn get_shell_type(&self) -> ShellType {
        self.shell_type
    }

    pub fn get_shell_path(&self) -> &PathBuf {
        &self.shell_path
    }

    pub fn get_path_env_var(&self) -> &Option<String> {
        &self.path_env_var
    }
}

#[derive(Debug, Clone)]
pub enum LocalShellStateEvent {}

/// Local shell is a singleton model registered on application startup
/// which asynchronously collects information about the user's default
/// shell/corresponding path environment variable. It's useful in
/// executing commands locally without having to initiate a session.
///
/// Its usage pattern can be seen in app/src/external_secrets/mod.rs,
/// where the shell_path is fetched from a caller view (in this case,
/// fetch_secrets in app/src/env_vars/env_var_collection.rs) via a
/// LocalShell handle, and is passed into the external secret manager
/// interface. The interface then dispatches commands to execute via
/// execute_command in this file.
#[cfg(feature = "local_tty")]
impl LocalShellState {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let preferred_shell = AvailableShells::handle(ctx)
            .read(ctx, |shells, ctx| shells.get_user_preferred_shell(ctx));
        let shell_starter_source_or_wsl_name = match ShellStarter::init(preferred_shell) {
            Some(shell_starter_source_or_wsl_name) => shell_starter_source_or_wsl_name,
            None => return LocalShellState::NotLoaded,
        };

        let shell_starter = match shell_starter_source_or_wsl_name {
            ShellStarterSourceOrWslName::Source(starter_source) => match starter_source.into() {
                ShellStarter::Direct(starter) | ShellStarter::MSYS2(starter) => starter,
                ShellStarter::DockerSandbox(docker_starter) => docker_starter.direct,
                ShellStarter::Wsl(_) => return LocalShellState::NotLoaded,
            },
            // TODO(CORE-3020): Implement WSL for the Local Shell model.
            ShellStarterSourceOrWslName::WSLName { .. } => return LocalShellState::NotLoaded,
        };

        let shell_path = shell_starter.logical_shell_path().to_owned();
        let shell_type = shell_starter.shell_type();
        let clone = shell_path.clone();

        let command = match shell_type {
            ShellType::Bash | ShellType::Zsh => "echo $PATH",
            ShellType::Fish => "env | grep PATH",
            ShellType::Nushell => "$env.PATH | str join (char esep)",
            ShellType::PowerShell => "echo $Env:PATH",
        };

        // Execute a command to fetch the user's PATH var, and store it
        // in the associated field
        ctx.spawn(
            async move { execute_command(shell_type, clone, None, command).await },
            |me, res, _| match me {
                LocalShellState::Loaded(local_shell_state) => {
                    // Trim to remove trailing newline from `echo $PATH` output
                    local_shell_state.path_env_var = res.ok().map(|s| s.trim().to_string());
                }
                LocalShellState::NotLoaded => {
                    log::warn!("Tried to execute a command on LocalShell that wasn't loaded")
                }
            },
        );

        Self::Loaded(LocalShell {
            shell_type,
            shell_path,
            path_env_var: None,
            interactive_env_state: InteractiveEnvState::NotRequested,
        })
    }

    pub fn local_shell_info(&self) -> Option<&LocalShell> {
        match self {
            LocalShellState::Loaded(shell_state) => Some(shell_state),
            LocalShellState::NotLoaded => None,
        }
    }

    /// Returns a future that will yield the PATH from an interactive login shell.
    /// This lazily triggers the capture on first call and caches the result.
    /// Use this for LSP operations that need the full interactive environment.
    pub fn get_interactive_path_env_var(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, Option<String>> {
        let LocalShellState::Loaded(local_shell) = self else {
            // Not loaded - return immediately with None
            return futures::future::ready(None).boxed();
        };

        match &mut local_shell.interactive_env_state {
            InteractiveEnvState::Ready(path) => {
                // Already captured - return immediately with cached value
                futures::future::ready(path.clone()).boxed()
            }
            InteractiveEnvState::Pending { waiters } => {
                // Capture in progress - add a new waiter
                let (tx, rx) = async_channel::bounded(1);
                waiters.push(tx);
                async move { rx.recv().await.ok().flatten() }.boxed()
            }
            InteractiveEnvState::NotRequested => {
                // First request - kick off the capture
                let (tx, rx) = async_channel::bounded(1);
                local_shell.interactive_env_state =
                    InteractiveEnvState::Pending { waiters: vec![tx] };

                let shell_type = local_shell.shell_type;
                let shell_path = local_shell.shell_path.clone();

                ctx.spawn(
                    async move { capture_interactive_shell_env(shell_type, shell_path).await },
                    move |me, result, _ctx| {
                        let path = result.ok();

                        // Notify all waiting receivers
                        if let LocalShellState::Loaded(local_shell) = me {
                            if let InteractiveEnvState::Pending { waiters } = std::mem::replace(
                                &mut local_shell.interactive_env_state,
                                InteractiveEnvState::Ready(path.clone()),
                            ) {
                                for waiter in waiters {
                                    let _ = waiter.try_send(path.clone());
                                }
                            }
                        }
                    },
                );

                async move { rx.recv().await.ok().flatten() }.boxed()
            }
        }
    }
}

#[cfg(feature = "local_tty")]
pub async fn execute_command(
    shell_type: ShellType,
    shell_path: PathBuf,
    path_env_var: Option<String>,
    command: &str,
) -> Result<String> {
    let local_command_executor = LocalCommandExecutor::new(Some(shell_path), shell_type);

    // Build environment variables map.
    // Always include HOME to ensure the shell can expand ~ in rc files - this is critical
    // when Warp is launched via launchd (Finder, Dock) with a minimal environment.
    let mut env_vars = HashMap::new();
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(|s| s.to_string())) {
        env_vars.insert("HOME".to_owned(), home);
    }
    if let Some(path) = path_env_var {
        env_vars.insert("PATH".to_owned(), path);
    }
    let env_vars = if env_vars.is_empty() {
        None
    } else {
        Some(env_vars)
    };

    match local_command_executor
        .execute_local_command_in_login_shell(command, None, env_vars)
        .await
    {
        Ok(result) => {
            let res = result.to_string()?;
            if result.success() {
                Ok(res)
            } else {
                Err(anyhow!(res))
            }
        }
        Err(_) => Err(anyhow!("Error while parsing result")),
    }
}

/// Captures the PATH environment variable from an interactive login shell.
/// This uses setsid() to start a new session (fully detaching from the terminal)
/// and stdin(null) to prevent interactive prompts from blocking.
#[cfg(feature = "local_tty")]
async fn capture_interactive_shell_env(
    shell_type: ShellType,
    shell_path: PathBuf,
) -> Result<String> {
    let command_str = match shell_type {
        ShellType::Bash | ShellType::Zsh => "echo $PATH",
        ShellType::Fish => "string join : $PATH",
        ShellType::Nushell => "$env.PATH | str join (char esep)",
        ShellType::PowerShell => "echo $Env:PATH",
    };

    // With the `-i` flag, shells may try to set themselves as the foreground process for their
    // controlling terminal with `tcsetpgrp` [1]. If the Warp process itself tries to read from
    // stdin (for example, some Oz CLI commands have interactive inputs), it may get suspended with
    // a `SIGTTIN` or `SIGTTOU` signal.
    //
    // To prevent this, we run the child in a new session with no controlling terminal by using
    // `setsid` [2].
    //
    // [1]: https://pubs.opengroup.org/onlinepubs/007904975/functions/tcsetpgrp.html
    // [2]: https://man7.org/linux/man-pages/man2/setsid.2.html
    #[cfg(unix)]
    let mut command = Command::new_with_session(&shell_path);
    #[cfg(not(unix))]
    let mut command = Command::new(&shell_path);

    // Add shell-specific flags for interactive login shell
    match shell_type {
        ShellType::Bash | ShellType::Zsh => {
            // -i: interactive (sources .zshrc/.bashrc)
            // -l: login shell (sources .zprofile/.bash_profile)
            command.args(["-i", "-l", "-c", command_str]);
        }
        ShellType::Fish => {
            command.args(["-i", "-l", "-c", command_str]);
        }
        ShellType::Nushell => {
            command.args(["--login", "--commands", command_str]);
        }
        ShellType::PowerShell => {
            // Note: we intentionally omit `-Login` here. PowerShell 5.1
            // (`powershell.exe`) does not support it, and on Windows the
            // PATH is managed via system/user environment variables rather
            // than login profile scripts, so `-Login` has no practical
            // effect even on PowerShell 7.
            command.args(["-Command", command_str]);
        }
    }

    // Set HOME to ensure shell can expand ~ in rc files
    if let Some(home) = dirs::home_dir() {
        command.env("HOME", home);
    }

    // stdin(null) prevents any prompts from blocking
    let output: std::process::Output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("Failed to spawn interactive shell: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[LSP PATH] Interactive shell capture had non-zero exit: {}",
            stderr
        );
        // Still try to use stdout even if exit was non-zero
        // (some shells exit non-zero but still produce valid output)
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = stdout.trim().to_string();

    if path.is_empty() {
        Err(anyhow!("Interactive shell returned empty PATH"))
    } else {
        Ok(path)
    }
}

impl Entity for LocalShellState {
    type Event = LocalShellStateEvent;
}

impl SingletonEntity for LocalShellState {}
