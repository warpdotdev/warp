use super::{CommandExecutor, CommandOutput, ExecuteCommandOptions};
use crate::safe_warn;
use crate::terminal::shell::{Shell, ShellType};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use command::r#async::Command;
use parking_lot::Mutex;
use std::any::Any;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

#[cfg(unix)]
fn kill_all_processes_in_process_group(pid: u32) -> Result<(), nix::Error> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    // Killing a negative PID kills all processes in this process group
    kill(Pid::from_raw(-(pid as i32)), Signal::SIGKILL)
}

enum CommandBuilder<'a> {
    #[cfg(windows)]
    CmdExe,
    ShellType {
        shell_type: ShellType,
        local_shell_path: Option<&'a Path>,
    },
}

impl CommandBuilder<'_> {
    fn build(self, command_string: &str, shell_config_flag: &str) -> Command {
        match self {
            #[cfg(windows)]
            CommandBuilder::CmdExe => {
                use command::windows::CommandExt as _;
                let mut command = Command::new_with_process_group("cmd.exe");
                command.args(["/Q", "/C"]);
                command.raw_arg(command_string);
                command
            }
            CommandBuilder::ShellType {
                local_shell_path,
                shell_type,
            } => {
                let program_to_execute = local_shell_path
                    .as_ref()
                    .and_then(|p| p.to_str())
                    .unwrap_or_else(|| {
                        log::warn!("local_shell_path was None for a local session");
                        shell_type.name()
                    });
                let mut command = Command::new_with_process_group(program_to_execute);
                command.arg(shell_config_flag);
                command.arg("-c");
                command.arg(command_string);
                command
            }
        }
    }
}

/// `CommandExecutor` implementation that executes the given `command` in a forked subshell process
/// where the current working directory is set to `current_dir_path` and $PATH is set
/// according to environment_variables. This is typically used to run generator commands for local sessions.
#[derive(Debug)]
pub struct LocalCommandExecutor {
    local_shell_path: Option<PathBuf>,
    shell_type: ShellType,

    spawned_children_pids: Arc<Mutex<HashSet<u32>>>,
}

impl LocalCommandExecutor {
    pub fn new(local_shell_path: Option<PathBuf>, shell_type: ShellType) -> Self {
        Self {
            local_shell_path,
            shell_type,
            spawned_children_pids: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn execute_local_command(
        &self,
        command: &str,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        let shell_config_flag = match self.shell_type {
            ShellType::Zsh => "-f",
            ShellType::Bash => "--norc",
            ShellType::Fish => "--no-config",
            ShellType::PowerShell => "-NoProfile",
        };

        self.execute_local_command_internal(
            command,
            current_directory_path,
            environment_variables,
            shell_config_flag,
            execute_command_options,
        )
        .await
    }

    pub async fn execute_local_command_in_login_shell(
        &self,
        command: &str,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
    ) -> Result<CommandOutput> {
        let shell_config_flag = match self.shell_type {
            ShellType::Bash | ShellType::Zsh | ShellType::Fish => "-l",
            ShellType::PowerShell => "-Login",
        };

        self.execute_local_command_internal(
            command,
            current_directory_path,
            environment_variables,
            shell_config_flag,
            ExecuteCommandOptions {
                // We have to run the command in the same shell as the session
                // because we want to run it in a login shell.
                run_command_in_same_shell_as_session: true,
            },
        )
        .await
    }

    #[cfg(unix)]
    fn command_builder(
        &self,
        _execute_command_options: ExecuteCommandOptions,
    ) -> CommandBuilder<'_> {
        CommandBuilder::ShellType {
            shell_type: self.shell_type,
            local_shell_path: self.local_shell_path.as_deref(),
        }
    }

    #[cfg(windows)]
    fn command_builder(
        &self,
        execute_command_options: ExecuteCommandOptions,
    ) -> CommandBuilder<'_> {
        let use_cmd_exe = !execute_command_options.run_command_in_same_shell_as_session
            && self.shell_type == ShellType::PowerShell;
        if use_cmd_exe {
            CommandBuilder::CmdExe
        } else {
            CommandBuilder::ShellType {
                shell_type: self.shell_type,
                local_shell_path: self.local_shell_path.as_deref(),
            }
        }
    }

    async fn execute_local_command_internal(
        &self,
        command: &str,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        // The value of shell_config_flag is appended as an argument
        // indicating the supplied command should be run under some configuration,
        // i.e. in a login shell or without sourcing .rc files
        shell_config_flag: &str,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        let command_builder = self.command_builder(execute_command_options);

        let mut command_process = command_builder.build(command, shell_config_flag);

        // This sets then environment variables, including the PATH var.
        // We need to run the command with the PATH var set because if the
        // user opened Warp through a parent process that didn't have the PATH var set
        // (i.e. outside of a shell, for example opening the app via Finder),
        // the subshell won't inherit the PATH var, but we need the PATH var
        // to reference executables we might run as part of generators.
        // Note: we don't need to quote/escape the PATH and pwd because
        // they're treated as single words.
        if let Some(environment_variables) = environment_variables {
            command_process.envs(&environment_variables);
        }

        // Set the current dir, if any.
        if let Some(current_directory_path) = current_directory_path {
            command_process.current_dir(current_directory_path);
        }

        // The purpose of the executor is to produce output. If the child
        // has been dropped, there's no way to get the output anymore,
        // so there's no need for the process itself to stick around.
        let child = command_process
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let child_pid = child.id();
        self.spawned_children_pids.lock().insert(child_pid);

        let output = child
            .output()
            .await
            .map(|output| output.into())
            .map_err(|e| {
                safe_warn!(
                    safe: ("error executing local command"),
                    full: ("error executing command {:?} with error {:?}", command, e)
                );
                anyhow!(e)
            });

        self.spawned_children_pids.lock().remove(&child_pid);
        output
    }
}

#[async_trait]
impl CommandExecutor for LocalCommandExecutor {
    async fn execute_command(
        &self,
        command: &str,
        _shell: &Shell,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        self.execute_local_command(
            command,
            current_directory_path,
            environment_variables,
            execute_command_options,
        )
        .await
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        true
    }

    fn cancel_active_commands(&self) {
        let spawned_children_pids = std::mem::take(&mut *self.spawned_children_pids.lock());
        for _pid in spawned_children_pids {
            // TODO(roland): handle for windows
            #[cfg(unix)]
            if let Err(e) = kill_all_processes_in_process_group(_pid) {
                match e {
                    // Ignore errors that occur when the process is no longer running,
                    // or if we cannot kill all processes in the process group.  These
                    // are expected to happen occasionally.
                    nix::errno::Errno::ESRCH | nix::errno::Errno::EPERM => {}
                    _ => log::warn!("Failed to kill process {_pid}: {e}"),
                }
            }
        }
    }
}
