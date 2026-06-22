use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use command::r#async::Command;
use itertools::Itertools as _;

use super::{CommandExecutor, CommandOutput, ExecuteCommandOptions};
use crate::env_vars::{serialize_variables_for_shell, EnvVarValue};
use crate::safe_warn;
use crate::terminal::shell::{Shell, ShellType};

/// `CommandExecutor` implementation that executes the given `command` in a WSL instance via the
/// `wsl.exe` executable.
#[derive(Debug)]
pub struct WslCommandExecutor {
    distro_name: String,
    shell_type: ShellType,
}

impl WslCommandExecutor {
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn new(distro_name: String, shell_type: ShellType) -> Self {
        Self {
            distro_name,
            shell_type,
        }
    }

    pub async fn execute_local_command(
        &self,
        command: &str,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
    ) -> Result<CommandOutput> {
        let shell_config_flag = match self.shell_type {
            ShellType::Zsh => "-f",
            ShellType::Bash => "--norc",
            ShellType::Fish => "--no-config",
            ShellType::PowerShell => "-NoProfile",
        };

        let mut command_process = Command::new("wsl");

        command_process.arg("--distribution").arg(&self.distro_name);

        if let Some(dir) = current_directory_path {
            command_process.arg("--cd");
            command_process.arg(dir);
        }

        let mut command_with_env = Cow::Borrowed(command);
        if let Some(mut env_vars) = environment_variables {
            if let Some(mut path_var) = env_vars.remove("PATH") {
                // Unfortunately, bash's `compgen` is extremely slow with PATH contains a bunch of
                // entries pointing to the Windows host. On my system, it takes around 2 minutes to
                // complete! We special-case this to filter out all paths beginning with `/mnt`.
                // This means we won't correctly highlight executables on the Windows host. We're
                // assuming WSL users will mostly be calling the executables inside the WSL guest.
                if self.shell_type == ShellType::Bash && command.contains("compgen") {
                    path_var = path_var
                        .split(':')
                        .filter(|path| !path.starts_with("/mnt"))
                        .join(":");
                }

                // Furthermore, we must pass the PATH by serializing it and adding a literal
                // assignment to the command itself. This is b/c Windows attempts to do a
                // conversion when propagating PATH from Windows to WSL, which cannot be disabled.
                // This conversion fails in this case b/c we collected the value of PATH from a
                // bootstrapped WSL session and it's _already_ converted. Conversion failures
                // result in truncation.
                let env_vars_str = serialize_variables_for_shell(
                    [("PATH", &EnvVarValue::Constant(path_var))],
                    self.shell_type,
                );
                command_with_env = Cow::Owned(format!(r#"{env_vars_str}; {command}"#));
            }

            // The rest of the env vars can be passed more "normally", though they need to be
            // allowlisted by assigning WSLENV.
            command_process.envs(&env_vars);
            command_process.env(
                "WSLENV",
                env_vars.keys().map(|k| format!("{k}/u")).join(":"),
            );
        }

        command_process
            .arg("--exec")
            .arg(self.shell_type.name())
            .arg(shell_config_flag)
            .arg("-c")
            .arg(&*command_with_env)
            // The purpose of the executor is to produce output. If the child
            // has been dropped, there's no way to get the output anymore,
            // so there's no need for the process itself to stick around.
            .kill_on_drop(true)
            .output()
            .await
            .map(|output| output.into())
            .map_err(|e| {
                safe_warn!(
                    safe: ("error executing local command"),
                    full: ("error executing command {:?} with error {:?}", command, e)
                );
                anyhow!(e)
            })
    }
}

#[async_trait]
impl CommandExecutor for WslCommandExecutor {
    async fn execute_command(
        &self,
        command: &str,
        _shell: &Shell,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        _execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        self.execute_local_command(command, current_directory_path, environment_variables)
            .await
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        true
    }
}
