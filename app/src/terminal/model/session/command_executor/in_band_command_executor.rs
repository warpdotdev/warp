use std::any::Any;
use std::cmp::min;
use std::collections::HashMap;
use std::sync::Arc;
use std::{collections::VecDeque, fmt};

use anyhow::Result;
use async_channel::{self, Receiver, Sender};
use async_trait::async_trait;
use chrono::DateTime;
use parking_lot::{Mutex, MutexGuard};
use warp_core::command::ExitCode;
use warp_terminal::model::Point;
use warpui::r#async::block_on;

use crate::safe_info;
use crate::server::datetime_ext::DateTimeExt;
use crate::terminal::event::ExecutedExecutorCommandEvent;
use crate::terminal::shell::{Shell, ShellType};
use warp_util::on_cancel::OnCancelFutureExt;

use crate::terminal::model::session::command_executor::{
    shared, CommandExecutor, ExecutorCommandEvent,
};
use crate::terminal::SizeInfo;
use warp_completer::completer::{CommandExitStatus, CommandOutput};

use super::ExecuteCommandOptions;

#[derive(Clone, Debug)]
pub struct InBandCommand {
    pub command: String,
    pub shell_type: ShellType,
    pub command_id: String,
}

pub struct InBandCommandCancelledEvent {
    pub command_id: String,
}
/// Information about a pending or running command, used to both trigger its execution and handle
/// its output.
#[derive(Debug, Clone)]
struct CommandExecutionInfo {
    id: String,
    command: String,
    shell: Shell,
    output_tx: Option<Sender<CommandOutput>>,
}

/// A `Session`-scoped executor for "in-band" commands.
///
/// "In-band" commands are commands that are executed _within_ the active terminal session -- that
/// is, the terminal session that corresponds to the `pty_controller` passed to the executor as a
/// constructor parameter.
///
/// Because commands are executed in the active terminal session, commands are executed serially to
/// avoid competing command outputs from corrupting one another.
///
/// This can be used to run arbitrary commands in the user's active session most commonly to query
/// the session context (e.g. files in a directory, branches in a git repo) to power features like
/// completions and syntax highlighting.
///
/// For more context, see the "In-band generators" TDD: https://docs.google.com/document/d/15GO1p9WHNnDsV2Nb-O-FW4c38QpWDuIyb7wmnKxmOrE/edit?usp=sharing.
pub struct InBandCommandExecutor {
    executor_command_tx: Sender<ExecutorCommandEvent>,
    cancel_command_tx: Sender<InBandCommandCancelledEvent>,
    running_command: Arc<Mutex<Option<CommandExecutionInfo>>>,
    pending_commands: Arc<Mutex<VecDeque<CommandExecutionInfo>>>,
}

/// Output is written as if it's going to a grid, but we store it as a string. This
/// struct manages some cursor state so we can properly handle some cursor mutations
/// that might happen during in-band output.
pub struct InBandCommandOutputReceiver {
    output: String,
    /// Current coordinates of the cursor.
    point: Point,
    /// Whether the cursor needs to wrap before receiving more input.
    /// This is functionally the same as input_needs_wrap on GridStorage::Cursor.
    needs_wrap: bool,
    /// The height of the grid.
    num_rows: usize,
    /// The width of the grid.
    num_cols: usize,
}

/// Tracks the cursor during in-band output so we arbitrary cursor mutations
/// during an in-band command don't corrupt the output.
impl InBandCommandOutputReceiver {
    pub fn new(starting_location: Point, grid_size: &SizeInfo) -> Self {
        Self {
            output: Default::default(),
            point: starting_location,
            needs_wrap: false,
            num_rows: grid_size.rows,
            num_cols: grid_size.columns,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.output
    }

    /// Add a character and advance the cursor.
    pub fn input(&mut self, c: char) {
        if self.needs_wrap {
            self.point.row = min(self.num_rows - 1, self.point.row + 1);
            self.point.col = 0;
        }
        self.output.push(c);
        if self.point.col + 1 >= self.num_cols {
            self.needs_wrap = true;
        } else {
            self.point.col += 1;
            self.needs_wrap = false;
        }
    }

    /// Moves the cursor back to the specified coordinate and updates the string.
    /// Note: this doesn't support forward movements of the cursor!
    pub fn goto(&mut self, row: usize, column: usize) {
        let new_point = Point::new(row, column);
        if new_point <= self.point {
            let mut distance = self.point.distance(self.num_cols, &new_point);
            if self.needs_wrap {
                distance += 1;
            }
            self.point = self.point.wrapping_sub(self.num_cols, distance);
            self.output.truncate(self.output.len() - distance);
        } else {
            log::warn!("Moving cursor forward during in-band output, which is unhandled");
        }
    }

    /// Moves the cursor to the beginning of the following row.
    pub fn carriage_return(&mut self) {
        self.point.row = min(self.num_rows - 1, self.point.row + 1);
        self.point.col = 0;
        self.needs_wrap = false;
    }
}

impl fmt::Debug for InBandCommandExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InBandCommandExecutor {{}}")
    }
}

impl InBandCommandExecutor {
    pub fn new(
        executor_command_tx: Sender<ExecutorCommandEvent>,
        cancel_command_tx: Sender<InBandCommandCancelledEvent>,
    ) -> Self {
        Self {
            executor_command_tx,
            cancel_command_tx,
            running_command: Arc::new(Mutex::new(None)),
            pending_commands: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Cancels a running command.
    ///
    /// Note: this is different from `Self::cancel_command`, in that
    /// it represents an *internal* cancellation where something downstack has cancelled
    /// a command believed to be running. In this case, we remove the command from
    /// execution, but also resolve the command with a CommandResult.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(super) fn handle_cancelled_in_band_command_event(
        &self,
        InBandCommandCancelledEvent { command_id }: InBandCommandCancelledEvent,
    ) {
        {
            // Scope all of this within a block so the `running_command` MutexGuard is not in scope when we call
            // `execute_command_internal` (which also attempts to lock `running_command`).
            let mut lock = self.running_command.lock();
            let Some(cmd) = lock.as_ref() else {
                return;
            };
            if cmd.id != command_id {
                return;
            }
            if let Some(output_tx) = cmd.output_tx.clone() {
                if !output_tx.is_closed() {
                    // TODO: we should consider turning this into a Result::Err
                    if let Err(error) = output_tx.try_send(CommandOutput {
                        stdout: vec![],
                        stderr: vec![],
                        status: CommandExitStatus::Failure,
                        exit_code: None,
                    }) {
                        log::warn!("Error occurred when sending generator command output: {error}");
                    }
                }
            }
            *lock = None;
        }
        self.execute_command_internal();
    }

    /// Parses `event` (which is parsed DCS output from the PTY) into `CommandOutput` and notifies
    /// its channel for the completed command (based on its command ID). This effectively
    /// completes the `Future` returned by `Self::execute_command()` for the command with the
    /// same command ID.
    ///
    /// Afterwards, attempts to execute the next pending command, if there is any.
    pub fn handle_executed_command_event(&self, event: ExecutedExecutorCommandEvent) {
        {
            // Scope all of this within a block so the `running_command` MutexGuard is not in scope when we call
            // `execute_command_internal` (which also attempts to lock `running_command`).
            let mut current_command = self.running_command.lock();
            if let Some(cmd) = current_command.take() {
                if cmd.id == event.command_id {
                    if let Some(output_tx) = cmd.output_tx {
                        if !output_tx.is_closed() {
                            let command_output = if event.exit_code == 0 {
                                CommandOutput {
                                    stdout: event.output,
                                    stderr: vec![],
                                    status: CommandExitStatus::Success,
                                    exit_code: Some(ExitCode::from(event.exit_code as i32)),
                                }
                            } else {
                                CommandOutput {
                                    stdout: vec![],
                                    stderr: event.output,
                                    status: CommandExitStatus::Failure,
                                    exit_code: Some(ExitCode::from(event.exit_code as i32)),
                                }
                            };
                            if let Err(error) = output_tx.try_send(command_output) {
                                log::error!(
                                    "Error occurred when sending generator command output: {error}"
                                );
                            }
                        }
                    }
                } else {
                    log::warn!("Cached in-band command ID {} does not match ID of executed in-band command output {}", cmd.id, &event.command_id);
                    // If the command event that we received is not for the current running command,
                    // we need to restore the command as the currently running command.
                    *current_command = Some(cmd);
                }
            }
        }
        self.execute_command_internal();
    }

    fn cancel_command(&self, command_id: &str) {
        // Notify the `PTYController` that the given command should be cancelled. We use
        // `send_blocking` here because this function is called in an implementation of `Drop`,
        // which must be synchronous. The underlying `executor_command_tx` channel is unbounded, so
        // in practice we'll never actually block if the channel is full.
        if let Err(e) = block_on(self.executor_command_tx.send(
            ExecutorCommandEvent::CancelCommand {
                id: command_id.into(),
            },
        )) {
            log::warn!("Failed to cancel in band command: {e:?}");
        }

        // Clear the running command if it corresponds to the command we want to cancel.
        let cmd_is_running = self
            .running_command
            .lock()
            .iter()
            .any(|item| item.id == command_id);

        if cmd_is_running {
            self.running_command.lock().take();
        }

        // Remove the command from the set of pending commands.
        self.pending_commands
            .lock()
            .retain(|item| item.id != command_id);

        // If the current command was removed, we need to restart
        // the execute_command loop to queue up another command
        if cmd_is_running {
            self.execute_command_internal();
        }
    }

    /// Adds `command` with `command_id` to the pending commands queue, and then attempts to
    /// execute it.
    fn enqueue_command(
        &self,
        command_id: &str,
        command: &str,
        shell: &Shell,
    ) -> Result<Receiver<CommandOutput>> {
        let (output_channel_tx, output_channel_rx) = async_channel::unbounded::<CommandOutput>();
        self.pending_commands
            .lock()
            .push_back(CommandExecutionInfo {
                id: command_id.to_owned(),
                command: command.to_owned(),
                shell: shell.clone(),
                output_tx: Some(output_channel_tx),
            });
        self.execute_command_internal();
        Ok(output_channel_rx)
    }

    /// If no command is currently running, pops a command from the pending_commands queue and
    /// executes it. If a command is running, does nothing.
    ///
    /// Prior to writing the command bytes, writes the "kill buffer bytes" for the appropriate
    /// shell type (which clears the input buffer) to ensure the command is executed as written.
    fn execute_command_internal(&self) {
        let mut running_command = self.running_command.lock();
        if running_command.is_none() {
            let mut pending_commands = self.pending_commands.lock();
            *running_command = pending_commands.pop_front();
            if let Some(CommandExecutionInfo {
                id, command, shell, ..
            }) = running_command.as_ref()
            {
                safe_info!(
                    safe: ("Running in-band command {id}"),
                    full: ("Running in-band command {id}: {command}")
                );

                // Because we wrap the command in single quotes in the string sent to the pty,
                // escape the single quotes in a valid way given the session's shell type.
                let escaped_command =
                    shared::shell_escape_single_quotes(command, shell.shell_type());

                let in_band_command = match shell.shell_type() {
                    ShellType::PowerShell => {
                        format!("Warp-Run-GeneratorCommand {id} '{escaped_command}' -ErrorAction Ignore")
                    }
                    ShellType::Fish => {
                        // Add a leading space for in-band commands in fish, which omits them from
                        // history. Unlike bash and zsh, fish does not have a mechanism for
                        // specifying command patterns to be omitted from history. Ignoring
                        // commands with a leading space is default, non-configurable behavior in
                        // fish.
                        format!(" warp_run_generator_command {id} '{escaped_command}'")
                    }
                    _ => {
                        format!("warp_run_generator_command {id} '{escaped_command}'")
                    }
                };

                if let Err(e) =
                    self.executor_command_tx
                        .try_send(ExecutorCommandEvent::ExecuteCommand {
                            command: InBandCommand {
                                command: in_band_command,
                                shell_type: shell.shell_type(),
                                command_id: id.into(),
                            },
                            cancel_tx: self.cancel_command_tx.clone(),
                        })
                {
                    log::warn!("Failed to send InBandCommand to pty_controller: {e}");
                }
            }
        }
    }

    /// Clears the `CommandExecutionInfo` from the given `running_command` and `pending_commands`.
    ///
    /// This method exists just to unify the cancellation logic in `execute_command_internal()` and
    /// the public implementation of `cancel_active_commands`. `execute_command_internal()` cannot
    /// call `self.cancel_active_commands()` directly; this would cause a deadlock on
    /// `self.running_command`.
    fn cancel_active_commands(
        running_command: &mut MutexGuard<Option<CommandExecutionInfo>>,
        pending_commands: &mut MutexGuard<VecDeque<CommandExecutionInfo>>,
    ) {
        **running_command = None;
        pending_commands.clear();
    }
}

#[async_trait]
impl CommandExecutor for InBandCommandExecutor {
    /// Executes `command` as an 'in-band' command in the active terminal session corresponding to
    /// the `pty_controller` passed to this executor during construction.
    ///
    /// The given `command` is executed in the active session using the
    /// `warp_run_generator_command`/`Warp-Run-GeneratorCommand` shell script API that is declared as
    /// part of Warp's bootstrap script.
    ///
    /// Internally, `command` is added to a queue of commands to be executed serially (this is to
    /// avoid output from multiple commands corrupting one another since the pty is a single
    /// channel). If there are no commands in the queue, `command` is written to the pty immediately.
    async fn execute_command(
        &self,
        command: &str,
        shell: &Shell,
        _current_directory_path: Option<&str>,
        _environment_variables: Option<HashMap<String, String>>,
        _execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        let command_id = DateTime::now().timestamp_micros().to_string();

        // If the future is aborted (via a call to `AbortHandle#abort`) we need to make sure to
        // remove the command from the in-band generator pending command queue to ensure that
        // commands aren't executed when they are eventually popped off the queue. We do this by
        // attaching a custom `on_cancel` hook to the future that cancels execution of the command
        // if the future was cancelled before it was ready.
        let future = async {
            let output_channel_rx = self.enqueue_command(command_id.as_str(), command, shell)?;
            output_channel_rx.recv().await.map_err(anyhow::Error::from)
        }
        .on_cancel(|| self.cancel_command(command_id.as_str()));

        future.await
    }

    /// "Cancels" active in-band commands.
    ///
    /// In reality, this does not cancel command execution (that is, however, actually done on the
    /// shell side in `warp_preexec`). This merely clears the running and pending command IDs from
    /// the executor's, such that subsequently calling `handle_executed_command_event` with
    /// a cleared command ID is a no-op.
    fn cancel_active_commands(&self) {
        Self::cancel_active_commands(
            &mut self.running_command.lock(),
            &mut self.pending_commands.lock(),
        );
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        false
    }
}

/// Returns `true` if `command` is an in-band command string, e.g. a command executed via
/// `InBandCommandExecutor`.
///
/// In-band commands are prefixed with a leading space for Fish, which is done to omit them from
/// fish's command history.  Thus we strip leading whitespace before matching the `command`.
pub fn is_in_band_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    trimmed.starts_with("Warp-Run-GeneratorCommand ")
        || trimmed.starts_with("warp_run_generator_command ")
}

#[cfg(test)]
#[path = "in_band_command_executor_tests.rs"]
mod tests;
