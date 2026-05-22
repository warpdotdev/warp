use std::any::Any;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use command::r#async::Command;
use itertools::Itertools;
use typed_path::{TypedPath, WindowsPath};
use warp_completer::completer::CommandOutput;
use warp_util::path::{convert_msys2_to_windows_native_path, msys2_exe_to_root};

use super::{CommandExecutor, ExecuteCommandOptions};
use crate::safe_warn;
use crate::terminal::shell::{Shell, ShellType};

const BASH_CONFIG_FLAG: &str = "--norc";
const POWERSHELL_CONFIG_FLAG: &str = "-NoProfile";

#[derive(Debug)]
pub struct MSYS2CommandExecutor {
    windows_native_shell_path: Option<PathBuf>,
    msys2_shell_path: PathBuf,
}

impl MSYS2CommandExecutor {
    pub fn new(windows_native_shell_path: Option<PathBuf>, msys2_shell_path: PathBuf) -> Self {
        Self {
            windows_native_shell_path,
            msys2_shell_path,
        }
    }

    async fn execute_windows_native_command(
        &self,
        command: &str,
        environment_variables: Option<HashMap<String, String>>,
    ) -> Result<CommandOutput> {
        // Currently, we use PowerShell for this.
        let Some(windows_native_shell_path) = self.windows_native_shell_path.as_ref() else {
            return Err(anyhow!("Windows native shell path not found"));
        };
        let mut command_process = Command::new(windows_native_shell_path);
        command_process.arg(POWERSHELL_CONFIG_FLAG);

        if let Some(mut environment_variables) = environment_variables {
            // We need to convert the unix-style paths to Windows paths so that PowerShell can
            // parse it.
            if let Some(path) = environment_variables.remove("PATH") {
                let mut new_path = OsString::new();
                for path in path.split(":").filter_map(|path| {
                    let unix_path = TypedPath::unix(path);
                    convert_msys2_to_windows_native_path(
                        &unix_path,
                        &msys2_exe_to_root(WindowsPath::new(
                            self.msys2_shell_path.as_os_str().as_encoded_bytes(),
                        )),
                    )
                    .ok()
                }) {
                    new_path.push(path);
                    new_path.push(";");
                }
                command_process.env("PATH", new_path);
            }
            command_process.envs(&environment_variables);
        }

        command_process
            .arg("-c")
            .arg(command)
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

    async fn execute_msys2_shell_command(
        &self,
        command: &str,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
    ) -> Result<CommandOutput> {
        let mut command_process = Command::new(&self.msys2_shell_path);
        command_process.arg(BASH_CONFIG_FLAG);

        if let Some(mut environment_variables) = environment_variables {
            // We exclude anything from the Windows filesystem here because it is very slow.
            // Compgen can take over a minute to run locally otherwise. We retrieve the
            // Windows executables by using a Windows-native shell.
            if command.contains("compgen") {
                if let Some(path) = environment_variables.get_mut("PATH") {
                    *path = path
                        .split(":")
                        .filter(|path| !path.starts_with("/c/"))
                        .join(":");
                }
            }
            command_process.envs(environment_variables);
        }

        if let Some(current_directory_path) = current_directory_path {
            let inner_path = TypedPath::unix(current_directory_path);
            let current_directory_path = convert_msys2_to_windows_native_path(
                &inner_path,
                &msys2_exe_to_root(WindowsPath::new(
                    self.msys2_shell_path.as_os_str().as_encoded_bytes(),
                )),
            )?;
            command_process.current_dir(current_directory_path);
        }

        command_process
            .arg("-c")
            .arg(command)
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
impl CommandExecutor for MSYS2CommandExecutor {
    async fn execute_command(
        &self,
        command: &str,
        shell: &Shell,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        _execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        match shell.shell_type() {
            ShellType::PowerShell => {
                self.execute_windows_native_command(command, environment_variables)
                    .await
            }
            shell_type => {
                if self.msys2_shell_path.file_stem().is_some_and(|stem| {
                    ShellType::from_name(stem.to_string_lossy().as_ref()) == Some(shell_type)
                }) {
                    self.execute_msys2_shell_command(
                        command,
                        current_directory_path,
                        environment_variables,
                    )
                    .await
                } else {
                    Err(anyhow!(
                        "MSYS2CommandExecutor tried to execute on a shell that isn't supported: {shell_type:?}"
                    ))
                }
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        true
    }
}
