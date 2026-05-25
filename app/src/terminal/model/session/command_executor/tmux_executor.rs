use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use anyhow::Result;
use async_channel::{self, Receiver, Sender};
use async_trait::async_trait;
use chrono::DateTime;
use parking_lot::Mutex;
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_core::command::ExitCode;

use super::{CommandExecutor, ExecuteCommandOptions, ExecutorCommandEvent};
use crate::server::datetime_ext::DateTimeExt;
use crate::terminal::event::ExecutedExecutorCommandEvent;
use crate::terminal::model::tmux::commands::TmuxCommand;
use crate::terminal::shell::Shell;

/// A `Session`-scoped executor for commands via tmux.
pub struct TmuxCommandExecutor {
    executor_command_tx: Sender<ExecutorCommandEvent>,
    in_flight_commands: Arc<Mutex<HashMap<String, Sender<CommandOutput>>>>,
}

impl fmt::Debug for TmuxCommandExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TmuxCommandExecutor {{}}")
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
impl TmuxCommandExecutor {
    pub fn new(executor_command_tx: Sender<ExecutorCommandEvent>) -> Self {
        Self {
            executor_command_tx,
            in_flight_commands: Default::default(),
        }
    }

    fn execute_command_internal(
        &self,
        command_id: &str,
        current_directory_path: Option<&str>,
        command: &str,
        _shell: &Shell,
        environment_variables: Option<HashMap<String, String>>,
    ) -> Result<Receiver<CommandOutput>> {
        let (output_channel_tx, output_channel_rx) = async_channel::unbounded::<CommandOutput>();

        self.in_flight_commands
            .lock()
            .insert(command_id.to_string(), output_channel_tx);

        let tmux_command = TmuxCommand::RunInBackgroundWindow {
            current_directory_path: current_directory_path.map(|s| s.to_string()),
            command_id: command_id.to_string(),
            command: command.to_string(),
            environment_variables,
        };

        if let Err(e) = self
            .executor_command_tx
            .try_send(ExecutorCommandEvent::ExecuteTmuxCommand(tmux_command))
        {
            log::warn!("Failed to send TmuxCommand to pty_controller: {e}");
        }

        Ok(output_channel_rx)
    }

    pub fn handle_executed_command_event(&self, event: ExecutedExecutorCommandEvent) {
        if let Some(output_tx) = self.in_flight_commands.lock().get(&event.command_id) {
            if !output_tx.is_closed() {
                // We shouldn't be receiving exit codes that aren't 32 bit signed integers.
                let exit_code = Some(ExitCode::from(event.exit_code as i32));
                let command_output = if event.exit_code == 0 {
                    CommandOutput {
                        stdout: event.output,
                        stderr: vec![],
                        status: CommandExitStatus::Success,
                        exit_code,
                    }
                } else {
                    CommandOutput {
                        stdout: vec![],
                        stderr: event.output,
                        status: CommandExitStatus::Failure,
                        exit_code,
                    }
                };
                if let Err(error) = output_tx.try_send(command_output) {
                    log::error!("Error occurred when sending generator command output: {error}");
                }
            }
        }
    }
}

#[async_trait]
impl CommandExecutor for TmuxCommandExecutor {
    /// Executes `command` while attached to an active tmux control mode session.
    /// Runs the command in a background tmux window.
    async fn execute_command(
        &self,
        command: &str,
        shell: &Shell,
        current_directory_path: Option<&str>,
        environment_variables: Option<HashMap<String, String>>,
        _execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        let command_id = DateTime::now().timestamp_micros().to_string();

        let future = async {
            let output_channel_rx = self.execute_command_internal(
                command_id.as_str(),
                current_directory_path,
                command,
                shell,
                environment_variables,
            )?;
            output_channel_rx.recv().await.map_err(anyhow::Error::from)
        };

        future.await
    }

    fn supports_parallel_command_execution(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
#[path = "tmux_executor_tests.rs"]
mod tests;
