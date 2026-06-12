use super::{CommandExecutor, CommandOutput, ExecuteCommandOptions};
use crate::safe_warn;
use crate::terminal::shell::{Shell, ShellType};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use command::r#async::Command;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Sandboxing technology to use for isolating commands.
#[derive(Debug, Clone)]
pub enum SandboxTechnology {
    /// Use Firejail for lightweight sandboxing (Unix-like systems).
    Firejail,
    /// Use Docker for container-based sandboxing.
    Docker,
    /// Use Windows Sandbox for Windows-based sandboxing.
    WindowsSandbox,
    /// Use macOS Sandbox for macOS-based sandboxing.
    MacOSSandbox,
}

/// Configuration options for the sandbox environment.
#[derive(Debug, Clone)]
pub struct SandboxOptions {
    /// Sandboxing technology to use.
    pub technology: SandboxTechnology,
    /// Enable network access in the sandbox.
    pub enable_network: bool,
    /// Directories to bind mount into the sandbox.
    pub bind_mounts: Vec<(PathBuf, PathBuf)>,
    /// CPU limits for the sandbox (in percentage).
    pub cpu_limit: Option<u32>,
    /// Memory limits for the sandbox (in MB).
    pub memory_limit: Option<u32>,
    /// Environment variables to set in the sandbox.
    pub environment_variables: HashMap<String, String>,
}

impl Default for SandboxOptions {
    fn default() -> Self {
        Self {
            technology: SandboxTechnology::Firejail,
            enable_network: false,
            bind_mounts: Vec::new(),
            cpu_limit: None,
            memory_limit: None,
            environment_variables: HashMap::new(),
        }
    }
}

/// `CommandExecutor` implementation that executes commands in a sandboxed environment.
/// This is used to run untrusted or potentially harmful commands in an isolated environment.
#[derive(Debug)]
pub struct SandboxCommandExecutor {
    local_shell_path: Option<PathBuf>,
    shell_type: ShellType,
    options: SandboxOptions,
}

impl SandboxCommandExecutor {
    pub fn new(
        local_shell_path: Option<PathBuf>,
        shell_type: ShellType,
        options: SandboxOptions,
    ) -> Self {
        Self {
            local_shell_path,
            shell_type,
            options,
        }
    }

    /// Get the sandbox command based on the configured technology.
    fn get_sandbox_command(&self) -> Result<Command, anyhow::Error> {
        match self.options.technology {
            SandboxTechnology::Firejail => self.get_firejail_command(),
            SandboxTechnology::Docker => self.get_docker_command(),
            SandboxTechnology::WindowsSandbox => self.get_windows_sandbox_command(),
            SandboxTechnology::MacOSSandbox => self.get_macos_sandbox_command(),
        }
    }

    /// Get the Firejail command for lightweight sandboxing.
    fn get_firejail_command(&self) -> Result<Command, anyhow::Error> {
        #[cfg(unix)]
        {
            // Check if Firejail is installed
            let firejail_check = std::process::Command::new("which")
                .arg("firejail")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;

            if !firejail_check.success() {
                return Err(anyhow!(
                    "Firejail is not installed. Please install Firejail to use the sandbox feature."
                ));
            }

            let mut command = Command::new("firejail");
            command.arg("--noprofile");
            command.arg("--private");

            // Configure network access
            if self.options.enable_network {
                command.arg("--net=eth0"); // Enable network access
            } else {
                command.arg("--net=none"); // Disable network access by default
            }

            // Configure bind mounts
            for (host_path, sandbox_path) in &self.options.bind_mounts {
                command
                    .arg("--bind")
                    .arg(host_path)
                    .arg(sandbox_path);
            }

            // Configure CPU limits
            if let Some(cpu_limit) = self.options.cpu_limit {
                command.arg("--cpu").arg(cpu_limit.to_string());
            }

            // Configure memory limits
            if let Some(memory_limit) = self.options.memory_limit {
                command.arg("--rlimit-as").arg(format!("{}M", memory_limit));
            }

            // Configure environment variables
            for (key, value) in &self.options.environment_variables {
                command.env(key, value);
            }

            Ok(command)
        }

        #[cfg(not(unix))]
        {
            Err(anyhow!("Firejail is only supported on Unix-like systems"))
        }
    }

    /// Get the Docker command for container-based sandboxing.
    fn get_docker_command(&self) -> Result<Command, anyhow::Error> {
        // Check if Docker is installed
        let docker_check = std::process::Command::new("which")
            .arg("docker")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;

        if !docker_check.success() {
            return Err(anyhow!(
                "Docker is not installed. Please install Docker to use the sandbox feature."
            ));
        }

        let mut command = Command::new("docker");
        command.arg("run").arg("--rm");

        // Configure network access
        if !self.options.enable_network {
            command.arg("--network").arg("none"); // Disable network access
        }

        // Configure bind mounts
        for (host_path, sandbox_path) in &self.options.bind_mounts {
            command
                .arg("-v")
                .arg(format!("{}:{}", host_path.display(), sandbox_path.display()));
        }

        // Configure CPU limits
        if let Some(cpu_limit) = self.options.cpu_limit {
            command.arg("--cpus").arg((cpu_limit as f64 / 100.0).to_string());
        }

        // Configure memory limits
        if let Some(memory_limit) = self.options.memory_limit {
            command.arg("--memory").arg(format!("{}m", memory_limit));
        }

        // Configure environment variables
        for (key, value) in &self.options.environment_variables {
            command.arg("-e").arg(format!("{}={}", key, value));
        }

        // Use a lightweight Alpine image
        command.arg("alpine");

        Ok(command)
    }

    /// Get the Windows Sandbox command for Windows-based sandboxing.
    fn get_windows_sandbox_command(&self) -> Result<Command, anyhow::Error> {
        #[cfg(windows)]
        {
            // Placeholder for Windows Sandbox implementation
            Err(anyhow!("Windows Sandbox is not yet supported"))
        }

        #[cfg(not(windows))]
        {
            Err(anyhow!("Windows Sandbox is only supported on Windows"))
        }
    }

    /// Get the macOS Sandbox command for macOS-based sandboxing.
    fn get_macos_sandbox_command(&self) -> Result<Command, anyhow::Error> {
        #[cfg(target_os = "macos")]
        {
            // Placeholder for macOS Sandbox implementation
            Err(anyhow!("macOS Sandbox is not yet supported"))
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(anyhow!("macOS Sandbox is only supported on macOS"))
        }
    }

    async fn execute_sandboxed_command(
        &self,
        command: &str,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        // Emit telemetry event for sandbox command execution
        log::info!("Executing command in sandbox: {}", command);

        // Get the sandbox command based on the configured technology
        let mut sandbox_command = self.get_sandbox_command()?;

        // Add the shell command to execute
        let shell_config_flag = match self.shell_type {
            ShellType::Zsh => "-f",
            ShellType::Bash => "--norc",
            ShellType::Fish => "--no-config",
            ShellType::PowerShell => "-NoProfile",
        };

        let shell_path = self.local_shell_path
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or_else(|| self.shell_type.name());

        sandbox_command.arg(shell_path);
        sandbox_command.arg(shell_config_flag);
        sandbox_command.arg("-c");
        sandbox_command.arg(command);

        // Merge environment variables from the options and the input
        let mut merged_env_vars = self.options.environment_variables.clone();
        if let Some(env_vars) = environment_variables {
            for (key, value) in env_vars {
                merged_env_vars.insert(key, value);
            }
        }

        // Set environment variables
        if !merged_env_vars.is_empty() {
            sandbox_command.envs(&merged_env_vars);
        }

        // Set the current directory
        if let Some(current_directory_path) = current_directory_path {
            sandbox_command.current_dir(current_directory_path);
        }

        // Execute the command
        let child = sandbox_command
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let output = child
            .output()
            .await
            .map(|output| output.into())
            .map_err(|e| {
                safe_warn!(
                    safe: ("error executing sandboxed command"),
                    full: ("error executing command {:?} with error {:?}", command, e)
                );
                anyhow!(e)
            });

        output
    }
}

#[async_trait]
impl CommandExecutor for SandboxCommandExecutor {
    async fn execute_command(
        &self,
        command: &str,
        _shell: &Shell,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        self.execute_sandboxed_command(
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
        // Sandboxed commands are isolated and can be safely cancelled
    }
}
