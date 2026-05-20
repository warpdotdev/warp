use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Local};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use futures::{select, FutureExt};
use futures_lite::pin;
use itertools::Itertools;
use parking_lot::FairMutex;
use warp_core::command::ExitCode;
use warp_core::execution_mode::AppExecutionMode;
use warp_util::path::ShellFamily;
use warpui::r#async::{Spawnable, Timer};
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::ai::agent::{
    AIAgentActionId, AIAgentActionType, AIAgentPtyWriteMode, ReadShellCommandOutputResult,
    RequestCommandOutputResult, ShellCommandDelay, ShellCommandError,
    TransferShellCommandControlToUserResult, WriteToLongRunningShellCommandResult,
};
use crate::ai::blocklist::permissions::CommandExecutionPermission;
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::WriteToPtyPermission;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::{
    formatted_terminal_contents_for_input, Block, BlockId, CURSOR_MARKER,
};
use crate::terminal::shell::ShellType;
use crate::{
    ai::agent::AIAgentActionResultType,
    terminal::{
        model::session::active_session::ActiveSession,
        model_events::{ModelEvent, ModelEventDispatcher},
        TerminalModel,
    },
};
use crate::{send_telemetry_from_ctx, TelemetryEvent};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};

pub struct ShellCommandExecutor {
    active_session: ModelHandle<ActiveSession>,
    block_finished_senders: HashMap<BlockSelector, oneshot::Sender<()>>,
    /// Senders used by the `Check now` affordance to force a long-running shell command's
    /// pending poll future to resolve immediately with a fresh snapshot, bypassing the
    /// agent-set timeout.
    force_refresh_senders: HashMap<BlockSelector, oneshot::Sender<()>>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    terminal_view_id: EntityId,
    /// Sender to notify when user hands control back to agent after TransferShellCommandControlToUser.
    control_handback_sender: Option<oneshot::Sender<()>>,
}

impl ShellCommandExecutor {
    pub const MAX_WAIT_DURATION: Duration = Duration::from_secs(2);
    /// Maximum delay we will honor for any agent-requested wait. Applies both  
    /// to finite `ShellCommandDelay::Duration` requests and to  
    /// `ShellCommandDelay::OnCompletion`, which would otherwise wait indefinitely.  
    pub const MAX_AGENT_DELAY_DURATION: Duration = Duration::from_secs(120);

    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(model_event_dispatcher, Self::handle_terminal_model_event);

        Self {
            active_session,
            terminal_model,
            block_finished_senders: HashMap::new(),
            force_refresh_senders: HashMap::new(),
            terminal_view_id,
            control_handback_sender: None,
        }
    }

    fn handle_terminal_model_event(&mut self, event: &ModelEvent, _ctx: &mut ModelContext<Self>) {
        // We wait for precmd for the block _after_ the requested command's block so that
        // downstream checks for current working directory are fresh. The precmd hook is when
        // the shell relays current working directory to warp.
        if let ModelEvent::BlockMetadataReceived(BlockMetadataReceivedEvent { .. }) = event {
            let model = self.terminal_model.lock();
            let block_finished_senders = self.block_finished_senders.drain().collect_vec();
            for (block_selector, block_finished_tx) in block_finished_senders.into_iter() {
                if let Some(block) = block_selector.get_block(&model) {
                    if block.is_command_finished() {
                        if let Err(e) = block_finished_tx.send(()) {
                            log::warn!(
                                "Failed to notify block completion for running requested command: {e:?}"
                            )
                        }
                    } else {
                        self.block_finished_senders
                            .insert(block_selector, block_finished_tx);
                    }
                }
            }
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let blocklist_permissions = BlocklistAIPermissions::as_ref(ctx);
        match &input.action.action {
            AIAgentActionType::RequestCommandOutput {
                command,
                is_read_only,
                is_risky,
                ..
            } => {
                let Some(escape_char) = self
                    .active_session
                    .as_ref(ctx)
                    .shell_type(ctx)
                    .map(|s| ShellFamily::from(s).escape_char())
                else {
                    return false;
                };
                let autoexecution_permission = blocklist_permissions.can_autoexecute_command(
                    &input.conversation_id,
                    command,
                    escape_char,
                    is_read_only.unwrap_or(false),
                    *is_risky,
                    Some(self.terminal_view_id),
                    ctx,
                );
                if let CommandExecutionPermission::Allowed(reason) = autoexecution_permission {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AutoexecutedAgentModeRequestedCommand { reason },
                        ctx
                    );
                } else if let CommandExecutionPermission::Denied(reason) = autoexecution_permission
                {
                    if AppExecutionMode::as_ref(ctx).is_autonomous() {
                        log::warn!(
                            "Command denied during autonomous execution, reason: {reason:?}"
                        );
                    }
                }
                autoexecution_permission.is_allowed()
            }
            AIAgentActionType::WriteToLongRunningShellCommand { block_id, .. } => {
                let terminal_model = self.terminal_model.lock();
                let block = terminal_model.block_list().block_with_id(block_id);

                if block.is_none_or(|block| block.finished()) {
                    // If the block is already finished, allow auto-execution - the finished output
                    // will be returned.
                    true
                } else {
                    let should_autoexecute = match blocklist_permissions.can_write_to_pty(
                        &input.conversation_id,
                        Some(self.terminal_view_id),
                        ctx,
                    ) {
                        WriteToPtyPermission::AlwaysAllow => true,
                        WriteToPtyPermission::AskOnFirstWrite => terminal_model
                            .block_list()
                            .active_block()
                            .has_agent_written_to_block(),
                        _ => false,
                    };

                    if should_autoexecute {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CLISubagentActionExecuted {
                                conversation_id: input.conversation_id,
                                block_id: block_id.clone(),
                                is_autoexecuted: true,
                            },
                            ctx
                        );
                    }

                    should_autoexecute
                }
            }
            AIAgentActionType::ReadShellCommandOutput { .. } => true,
            AIAgentActionType::TransferShellCommandControlToUser { .. } => false,
            _ => false,
        }
    }

    /// Decorate the command so that we can turn off pager.
    fn turn_off_pager_for_command(&self, command: &String, ctx: &mut ModelContext<Self>) -> String {
        match self.active_session.as_ref(ctx).shell_type(ctx) {
            // If it's a posix shell, we can use parentheses as the grouping character. Add command to
            // avoid cases with aliases.
            Some(ShellType::Zsh) | Some(ShellType::Bash) => format!("({command}) | command cat"),
            // Fish doesn't have grouping characters. We need to use begin; and end; to ensure the command
            // gets evaluated first.
            Some(ShellType::Fish) => format!("begin; {command} ;end | command cat"),
            // For powershell, we use Out-Host to send paged output to the
            // console. Add a backslash to avoid executing an alias.
            Some(ShellType::PowerShell) => format!("({command}) | \\Out-Host"),
            // If we can't determine a shell type, run command as it is.
            None => command.clone(),
        }
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let model = self.terminal_model.lock();

        // Determine the action we want to take based on the input.
        let action_id = input.action.id.clone();

        let command = model
            .block_list()
            .active_block()
            .command_with_secrets_unobfuscated(false)
            .clone();

        let handle = ctx.handle();
        match &input.action.action {
            AIAgentActionType::RequestCommandOutput {
                command,
                uses_pager,
                wait_until_completion,
                ..
            } => {
                if model
                    .block_list()
                    .active_block()
                    .is_active_and_long_running()
                {
                    // If there is an active block, we can't execute another command.
                    return ActionExecution::Sync(AIAgentActionResultType::RequestCommandOutput(
                        RequestCommandOutputResult::CancelledBeforeExecution,
                    ));
                }
                // If the command might use pager and can't be interacted with,
                // we pipe its output to cat so we can prevent activating the altscreen.
                // The parentheses here ensures the command always gets evaluated first.
                let decorated_command =
                    if uses_pager.is_some_and(|uses_pager| uses_pager) && *wait_until_completion {
                        self.turn_off_pager_for_command(command, ctx)
                    } else {
                        command.clone()
                    };
                ctx.emit(ShellCommandExecutorEvent::ExecuteCommand {
                    action_id: action_id.clone(),
                    command: decorated_command,
                });

                let block_selector = BlockSelector::RequestedCommandId(action_id.clone());
                let command = command.clone();
                drop(model);

                ActionExecution::new_async(
                    self.action_result_future(block_selector.clone(), None),
                    move |result, ctx| {
                        // Remove the senders from the maps.
                        if let Some(handle) = handle.upgrade(ctx) {
                            handle.update(ctx, |me, _| {
                                me.block_finished_senders.remove(&block_selector);
                                me.force_refresh_senders.remove(&block_selector);
                            });
                        }

                        action_result_for_requested_command(command, result)
                    },
                )
            }
            AIAgentActionType::WriteToLongRunningShellCommand {
                block_id,
                input,
                mode,
            } => {
                let Some(block) = model.block_list().block_with_id(block_id) else {
                    return ActionExecution::Sync(
                        AIAgentActionResultType::WriteToLongRunningShellCommand(
                            WriteToLongRunningShellCommandResult::Error(
                                ShellCommandError::BlockNotFound,
                            ),
                        ),
                    );
                };
                if block.finished() {
                    let output: String = block.output_with_secrets_unobfuscated();
                    let exit_code = block.exit_code();
                    let start_ts = block.start_ts().cloned();
                    let completed_ts = block.completed_ts().cloned();
                    return ActionExecution::Sync(
                        AIAgentActionResultType::WriteToLongRunningShellCommand(
                            WriteToLongRunningShellCommandResult::CommandFinished {
                                block_id: block.id().clone(),
                                output,
                                exit_code,
                                start_ts,
                                completed_ts,
                            },
                        ),
                    );
                }
                // Drop immutable borrow.
                drop(model);

                let mut model = self.terminal_model.lock();
                if let Some(block) = model.block_list_mut().mut_block_from_id(block_id) {
                    block.mark_agent_written_to_block();
                }
                drop(model);

                ctx.emit(ShellCommandExecutorEvent::WriteToPty {
                    input: input.clone(),
                    mode: *mode,
                });

                let block_selector = BlockSelector::Id(block_id.clone());
                ActionExecution::new_async(
                    self.action_result_future(
                        block_selector.clone(),
                        Some(ShellCommandDelay::Duration(Duration::from_millis(200))),
                    ),
                    move |result, ctx| {
                        // Remove the senders from the maps.
                        if let Some(handle) = handle.upgrade(ctx) {
                            handle.update(ctx, |me, _| {
                                me.block_finished_senders.remove(&block_selector);
                                me.force_refresh_senders.remove(&block_selector);
                            });
                        }

                        action_result_for_write_to_long_running_shell_command(result)
                    },
                )
            }
            AIAgentActionType::ReadShellCommandOutput { block_id, delay } => {
                let Some(block) = model.block_list().block_with_id(block_id) else {
                    return ActionExecution::Sync(AIAgentActionResultType::ReadShellCommandOutput(
                        ReadShellCommandOutputResult::Error(ShellCommandError::BlockNotFound),
                    ));
                };
                if block.finished() {
                    let command = block.command_with_secrets_unobfuscated(false);
                    let output: String = block.output_with_secrets_unobfuscated();
                    let exit_code = block.exit_code();
                    let start_ts = block.start_ts().cloned();
                    let completed_ts = block.completed_ts().cloned();
                    return ActionExecution::Sync(AIAgentActionResultType::ReadShellCommandOutput(
                        ReadShellCommandOutputResult::CommandFinished {
                            command,
                            block_id: block_id.clone(),
                            output,
                            exit_code,
                            start_ts,
                            completed_ts,
                        },
                    ));
                }
                drop(model);

                let block_selector = BlockSelector::Id(block_id.clone());
                ActionExecution::new_async(
                    self.action_result_future(block_selector.clone(), delay.clone()),
                    move |result, ctx| {
                        // Remove the senders from the maps.
                        if let Some(handle) = handle.upgrade(ctx) {
                            handle.update(ctx, |me, _| {
                                me.block_finished_senders.remove(&block_selector);
                                me.force_refresh_senders.remove(&block_selector);
                            });
                        }

                        action_result_for_read_shell_command_output(command.clone(), result)
                    },
                )
            }
            AIAgentActionType::TransferShellCommandControlToUser { reason } => {
                let active_block = model.block_list().active_block();
                if !active_block.is_active_and_long_running() {
                    return ActionExecution::Sync(
                        AIAgentActionResultType::TransferShellCommandControlToUser(
                            TransferShellCommandControlToUserResult::Error(
                                ShellCommandError::BlockNotFound,
                            ),
                        ),
                    );
                }

                let block_id = active_block.id().clone();
                drop(model);

                // Emit event to transfer control to user.
                ctx.emit(ShellCommandExecutorEvent::TransferControlToUser {
                    action_id: action_id.clone(),
                    reason: reason.clone(),
                });

                // Create a channel to wait for control handback.
                let (handback_tx, handback_rx) = oneshot::channel();
                self.control_handback_sender = Some(handback_tx);

                let block_selector = BlockSelector::Id(block_id.clone());

                // Set up a future to also wait for block completion.
                let (block_finished_tx, block_finished_rx) = oneshot::channel();
                self.block_finished_senders
                    .insert(block_selector.clone(), block_finished_tx);

                // Build the future that captures terminal model and block data.
                let transfer_future = {
                    let terminal_model = self.terminal_model.clone();
                    let block_id = block_id.clone();
                    async move {
                        pin!(handback_rx);
                        pin!(block_finished_rx);

                        // Wait for either control handback or block completion.
                        let transfer_result = select! {
                            val = handback_rx => match val {
                                Ok(_) => TransferControlResult::ControlHandedBack,
                                Err(_) => TransferControlResult::Cancelled,
                            },
                            val = block_finished_rx => match val {
                                Ok(_) => TransferControlResult::BlockFinished,
                                Err(_) => TransferControlResult::Cancelled,
                            },
                        };

                        // Convert to ActionResult
                        let model = terminal_model.lock();
                        match transfer_result {
                            TransferControlResult::ControlHandedBack
                            | TransferControlResult::BlockFinished => {
                                match model.block_list().block_with_id(&block_id) {
                                    Some(block) => {
                                        if block.finished() {
                                            ActionResult::CommandFinished {
                                                block_id: block.id().clone(),
                                                output: block.output_with_secrets_unobfuscated(),
                                                exit_code: block.exit_code(),
                                                start_ts: block.start_ts().cloned(),
                                                completed_ts: block.completed_ts().cloned(),
                                            }
                                        } else {
                                            let grid_contents = if model.is_alt_screen_active() {
                                                formatted_terminal_contents_for_input(
                                                    model.alt_screen().grid_handler(),
                                                    None,
                                                    CURSOR_MARKER,
                                                )
                                            } else {
                                                formatted_terminal_contents_for_input(
                                                    block.output_grid().grid_handler(),
                                                    Some(1000),
                                                    CURSOR_MARKER,
                                                )
                                            };
                                            ActionResult::LongRunningCommandSnapshot {
                                                block_id: block.id().clone(),
                                                grid_contents,
                                                cursor: CURSOR_MARKER,
                                                is_alt_screen_active: model.is_alt_screen_active(),
                                                is_preempted: false,
                                            }
                                        }
                                    }
                                    None => ActionResult::BlockNotFound,
                                }
                            }
                            TransferControlResult::Cancelled => ActionResult::Cancelled,
                        }
                    }
                };

                ActionExecution::new_async(transfer_future, move |result, ctx| {
                    // Clean up.
                    if let Some(handle) = handle.upgrade(ctx) {
                        handle.update(ctx, |me, _| {
                            me.block_finished_senders.remove(&block_selector);
                            me.control_handback_sender = None;
                        });
                    }

                    action_result_for_transfer_shell_command_control_to_user(result)
                })
            }
            _ => ActionExecution::InvalidAction,
        }
    }

    /// Called when user hands control back to agent after TransferShellCommandControlToUser.
    pub fn notify_control_handed_back(&mut self) {
        if let Some(sender) = self.control_handback_sender.take() {
            let _ = sender.send(());
        }
    }

    /// Produces a future which resolves when the action is complete and
    /// we have a result to send to the agent.
    fn action_result_future(
        &mut self,
        block_selector: BlockSelector,
        delay: Option<ShellCommandDelay>,
    ) -> impl Spawnable<Output = ActionResult> {
        // Create a channel to notify us when we receive block metadata.
        let (block_metadata_received_tx, block_metadata_received_rx) = oneshot::channel();
        self.block_finished_senders
            .insert(block_selector.clone(), block_metadata_received_tx);

        // Create a channel so the `Check now` affordance can short-circuit the timeout
        // and deliver the agent a fresh snapshot immediately.
        let (force_refresh_tx, force_refresh_rx) = oneshot::channel();
        self.force_refresh_senders
            .insert(block_selector.clone(), force_refresh_tx);

        // Create a future that resolves when we should send a result to the agent.
        let terminal_model = self.terminal_model.clone();

        #[derive(Debug, Clone, Copy)]
        enum WakeReason {
            BlockFinished,
            Timeout,
            /// User clicked `Check now` in the warping indicator, short-circuiting  
            /// the agent-set poll timer. Treated as a preemption so the server does  
            /// not interpret the early snapshot as a completion.  
            ForceRefresh,
        }

        async move {
            // If we support long-running commands, set up a timeout after which we'll
            // treat the command as long-running and give the agent a snapshot of the
            // current state.  Otherwise, we'll wait indefinitely for the command to
            // finish executing.
            let mut timeout = match delay {
                Some(ShellCommandDelay::Duration(duration)) => {
                    // Enforce a maximum allowed delay that the agent may request, never waiting longer than MAX_AGENT_DELAY_DURATION.
                    // If the requested duration exceeds this cap, we'll still behave as if the agent may expect a running command,
                    // so there's no need to signal preemption (the agent already anticipates an incomplete command state).
                    Timer::after(duration.min(Self::MAX_AGENT_DELAY_DURATION))
                }
                Some(ShellCommandDelay::OnCompletion) => {
                    Timer::after(Self::MAX_AGENT_DELAY_DURATION)
                }
                None => Timer::after(Self::MAX_WAIT_DURATION),
            }
            .fuse();

            pin!(block_metadata_received_rx);
            pin!(force_refresh_rx);

            let wake_reason = select! {
                val = block_metadata_received_rx => match val {
                    Ok(_) => WakeReason::BlockFinished,
                    Err(_) => return ActionResult::Cancelled,
                },
                val = force_refresh_rx => match val {
                    // User asked the agent to check now; fall through to the snapshot
                    // code path below. Treated as a preemption (snapshot arrives before
                    // the agent's own timer would have fired).
                    Ok(_) => WakeReason::ForceRefresh,
                    // Sender was dropped (e.g. because the executor is being torn down).
                    Err(_) => return ActionResult::Cancelled,
                },
                _ = timeout => WakeReason::Timeout,
            };

            // Mark the snapshot as preempted if woken early, allowing the server to distinguish
            // true completion from a forced client poll (`ForceRefresh`) or a timeout during `on_completion`.
            let is_preempted = matches!(wake_reason, WakeReason::ForceRefresh)
                || matches!(
                    (&wake_reason, &delay),
                    (WakeReason::Timeout, Some(ShellCommandDelay::OnCompletion))
                );

            // At this point, we've either received block metadata or we've timed out.
            // Check the current state of the block and produce a result accordingly.
            let model = terminal_model.lock();
            let result = match block_selector.get_block(&model) {
                Some(block) => {
                    if block.finished() {
                        ActionResult::CommandFinished {
                            block_id: block.id().clone(),
                            output: block.output_with_secrets_unobfuscated(),
                            exit_code: block.exit_code(),
                            start_ts: block.start_ts().cloned(),
                            completed_ts: block.completed_ts().cloned(),
                        }
                    } else {
                        let grid_contents = if model.is_alt_screen_active() {
                            formatted_terminal_contents_for_input(
                                model.alt_screen().grid_handler(),
                                None,
                                CURSOR_MARKER,
                            )
                        } else {
                            formatted_terminal_contents_for_input(
                                block.output_grid().grid_handler(),
                                // TODO(vorporeal): This is probably too large.
                                Some(1000),
                                CURSOR_MARKER,
                            )
                        };
                        ActionResult::LongRunningCommandSnapshot {
                            block_id: block.id().clone(),
                            grid_contents,
                            cursor: CURSOR_MARKER,
                            is_alt_screen_active: model.is_alt_screen_active(),
                            is_preempted,
                        }
                    }
                }
                None => ActionResult::BlockNotFound,
            };

            result
        }
    }

    pub(super) fn cancel_execution(&mut self, id: &AIAgentActionId, _ctx: &mut ModelContext<Self>) {
        let terminal_model = self.terminal_model.lock();
        let active_block = terminal_model.block_list().active_block();
        if !active_block.is_active_and_long_running() {
            return;
        }

        let selector = if active_block
            .requested_command_action_id()
            .is_some_and(|requested_command_id| requested_command_id == id)
        {
            BlockSelector::RequestedCommandId(id.clone())
        } else {
            BlockSelector::Id(active_block.id().clone())
        };
        self.block_finished_senders.remove(&selector);
        self.force_refresh_senders.remove(&selector);
    }

    /// Force any in-flight poll for the given long-running command block to resolve
    /// immediately with a fresh snapshot, bypassing the agent-set timeout.
    ///
    /// Called by the `Check now` affordance in the warping indicator. No-ops if there
    /// is no matching in-flight poll (e.g. because the block already finished or the
    /// agent has transferred control to the user).
    pub fn force_refresh_block(&mut self, block_id: &BlockId) {
        let terminal_model = self.terminal_model.lock();
        // Find a sender whose selector resolves to this block. In practice there is at
        // most one: a given block can have at most one in-flight `action_result_future`
        // at a time.
        let matching_selector = self
            .force_refresh_senders
            .keys()
            .find(|selector| {
                selector
                    .get_block(&terminal_model)
                    .is_some_and(|block| block.id() == block_id)
            })
            .cloned();
        drop(terminal_model);

        if let Some(selector) = matching_selector {
            if let Some(sender) = self.force_refresh_senders.remove(&selector) {
                let _ = sender.send(());
            }
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _action: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum BlockSelector {
    Id(BlockId),
    RequestedCommandId(AIAgentActionId),
}

impl BlockSelector {
    fn get_block<'a>(&self, model: &'a TerminalModel) -> Option<&'a Block> {
        match self {
            BlockSelector::Id(block_id) => model.block_list().block_with_id(block_id),
            BlockSelector::RequestedCommandId(requested_command_id) => model
                .block_list()
                .block_for_ai_action_id(requested_command_id),
        }
    }
}

/// Returns the result from executing a requested command.
fn action_result_for_requested_command(
    command: String,
    result: ActionResult,
) -> AIAgentActionResultType {
    match result {
        ActionResult::CommandFinished {
            block_id,
            output,
            exit_code,
            start_ts,
            completed_ts,
        } => AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
            command,
            block_id,
            output,
            exit_code,
            start_ts,
            completed_ts,
        }),
        ActionResult::LongRunningCommandSnapshot {
            block_id,
            grid_contents,
            cursor,
            is_alt_screen_active,
            ..
        } => AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::LongRunningCommandSnapshot {
                command,
                block_id,
                grid_contents,
                cursor: cursor.to_owned(),
                is_alt_screen_active,
            },
        ),
        ActionResult::BlockNotFound | ActionResult::Cancelled => {
            AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::CancelledBeforeExecution,
            )
        }
    }
}

/// Returns the result from writing to a long-running shell command.
fn action_result_for_write_to_long_running_shell_command(
    result: ActionResult,
) -> AIAgentActionResultType {
    match result {
        ActionResult::CommandFinished {
            block_id,
            output,
            exit_code,
            start_ts,
            completed_ts,
        } => AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            },
        ),
        ActionResult::LongRunningCommandSnapshot {
            block_id,
            grid_contents,
            cursor,
            is_alt_screen_active,
            is_preempted,
        } => AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::Snapshot {
                block_id,
                grid_contents,
                cursor: cursor.to_owned(),
                is_alt_screen_active,
                is_preempted,
            },
        ),
        ActionResult::Cancelled => AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::Cancelled,
        ),
        ActionResult::BlockNotFound => AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::Error(ShellCommandError::BlockNotFound),
        ),
    }
}

/// Returns the result from reading shell command output.
fn action_result_for_read_shell_command_output(
    command: String,
    result: ActionResult,
) -> AIAgentActionResultType {
    match result {
        ActionResult::CommandFinished {
            output,
            exit_code,
            block_id,
            start_ts,
            completed_ts,
        } => AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::CommandFinished {
                command,
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            },
        ),
        ActionResult::LongRunningCommandSnapshot {
            block_id,
            grid_contents,
            cursor,
            is_alt_screen_active,
            is_preempted,
        } => AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                command,
                block_id,
                grid_contents,
                cursor: cursor.to_owned(),
                is_alt_screen_active,
                is_preempted,
            },
        ),
        ActionResult::Cancelled => {
            AIAgentActionResultType::ReadShellCommandOutput(ReadShellCommandOutputResult::Cancelled)
        }
        ActionResult::BlockNotFound => AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::Error(ShellCommandError::BlockNotFound),
        ),
    }
}

/// Returns the result from transferring shell command control to user.
fn action_result_for_transfer_shell_command_control_to_user(
    result: ActionResult,
) -> AIAgentActionResultType {
    match result {
        ActionResult::CommandFinished {
            block_id,
            output,
            exit_code,
            start_ts,
            completed_ts,
        } => AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            },
        ),
        ActionResult::LongRunningCommandSnapshot {
            block_id,
            grid_contents,
            cursor,
            is_alt_screen_active,
            is_preempted,
        } => AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::Snapshot {
                block_id,
                grid_contents,
                cursor: cursor.to_owned(),
                is_alt_screen_active,
                is_preempted,
            },
        ),
        ActionResult::Cancelled => AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::Cancelled,
        ),
        ActionResult::BlockNotFound => AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::Error(ShellCommandError::BlockNotFound),
        ),
    }
}

#[derive(Debug, Clone)]
pub enum ShellCommandExecutorEvent {
    ExecuteCommand {
        action_id: AIAgentActionId,
        command: String,
    },
    WriteToPty {
        input: Bytes,
        mode: AIAgentPtyWriteMode,
    },
    CancelExecution,
    /// Emitted when the agent requests to transfer control of a long-running command to the user.
    TransferControlToUser {
        action_id: AIAgentActionId,
        reason: String,
    },
}

impl Entity for ShellCommandExecutor {
    type Event = ShellCommandExecutorEvent;
}

/// Result from waiting for control transfer.
#[derive(Debug, Clone)]
enum TransferControlResult {
    ControlHandedBack,
    BlockFinished,
    Cancelled,
}

/// The possible results of taking an action.
#[derive(Debug, Clone)]
enum ActionResult {
    CommandFinished {
        block_id: BlockId,
        output: String,
        exit_code: ExitCode,
        start_ts: Option<DateTime<Local>>,
        completed_ts: Option<DateTime<Local>>,
    },
    LongRunningCommandSnapshot {
        block_id: BlockId,
        grid_contents: String,
        cursor: &'static str,
        is_alt_screen_active: bool,
        is_preempted: bool,
    },
    Cancelled,
    BlockNotFound,
}
