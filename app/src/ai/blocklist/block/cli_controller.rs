use std::collections::HashMap;
use std::sync::Arc;

use instant::Instant;
use parking_lot::FairMutex;
use serde::{Deserialize, Deserializer, Serialize};
use warp_core::send_telemetry_from_ctx;
use warp_errors::report_error;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResultType, AIAgentContext, CancellationReason,
    ReadShellCommandOutputResult, RequestCommandOutputResult,
    TransferShellCommandControlToUserResult, WriteToLongRunningShellCommandResult,
};
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewEntryOrigin};
use crate::ai::blocklist::context_model::block_context_from_terminal_model;
use crate::ai::blocklist::{
    BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIController, BlocklistAIHistoryEvent,
};
use crate::server::telemetry::{CLISubagentControlState, TelemetryEvent};
use crate::terminal::TerminalModel;
use crate::terminal::model::block::BlockId;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum UserTakeOverReason {
    Manual,
    /// The user interrupted the command and took control. `should_auto_resume` is `true` for a
    /// live interrupt (e.g. Ctrl-C) that keeps the conversation alive so it resumes once the
    /// command completes, and `false` for teardown flows (stop/rewind) that have cancelled it.
    Stop {
        should_auto_resume: bool,
    },
    /// The agent explicitly transferred control to the user via the
    /// TransferShellCommandControlToUser tool call.
    TransferFromAgent {
        /// The reason the agent gave for transferring control.
        reason: String,
    },
}

impl<'de> Deserialize<'de> for UserTakeOverReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // `Current` mirrors the derived shape so serde can parse it without recursing back into
        // this impl. `LegacyStop` accepts the bare `"Stop"` persisted before `should_auto_resume`
        // existed: an externally tagged enum can't accept `Stop` as both a unit (legacy) and a
        // struct (current) variant, and `#[serde(default)]` can't bridge the two, so the forms are
        // unioned as `untagged`.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Current(Current),
            LegacyStop(LegacyStop),
        }

        #[derive(Deserialize)]
        enum Current {
            Manual,
            Stop { should_auto_resume: bool },
            TransferFromAgent { reason: String },
        }

        #[derive(Deserialize)]
        enum LegacyStop {
            Stop,
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Current(Current::Manual) => Self::Manual,
            Wire::Current(Current::Stop { should_auto_resume }) => {
                Self::Stop { should_auto_resume }
            }
            Wire::Current(Current::TransferFromAgent { reason }) => {
                Self::TransferFromAgent { reason }
            }
            Wire::LegacyStop(LegacyStop::Stop) => Self::Stop {
                should_auto_resume: false,
            },
        })
    }
}

#[derive(Debug, Clone, Default)]
struct ActiveCLISubagentState {
    task_id: Option<TaskId>,
    last_snapshot_at: Option<Instant>,
    latest_instruction: Option<String>,
}
/// Read-only identity and control state for a terminal command currently
/// associated with a CLI subagent.
///
/// The terminal block remains the canonical owner of this state. Front-ends
/// use this snapshot to keep rendering and input routing in agreement without
/// exposing mutable block internals.
#[derive(Debug, Clone, PartialEq)]
pub struct CLISubagentTarget {
    pub block_id: BlockId,
    pub task_id: TaskId,
    pub conversation_id: AIConversationId,
    pub requested_command_action_id: Option<AIAgentActionId>,
    pub control_state: LongRunningCommandControlState,
    pub last_snapshot_at: Option<Instant>,
    pub latest_instruction: Option<String>,
}

impl UserTakeOverReason {
    pub fn is_stop(&self) -> bool {
        matches!(self, Self::Stop { .. })
    }

    pub fn is_transfer_from_agent(&self) -> bool {
        matches!(self, Self::TransferFromAgent { .. })
    }

    /// Returns `true` if the conversation should resume once the user-controlled command
    /// completes. Only a teardown `Stop` opts out.
    pub fn should_auto_resume(&self) -> bool {
        match self {
            Self::Manual | Self::TransferFromAgent { .. } => true,
            Self::Stop { should_auto_resume } => *should_auto_resume,
        }
    }

    pub fn transfer_reason(&self) -> Option<&str> {
        match self {
            Self::TransferFromAgent { reason } => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// Represents which party is in control of the active long running command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LongRunningCommandControlState {
    /// The agent is in control.
    ///
    /// When the agent has control, the user cannot submit input to the command.
    Agent {
        /// `true` if the agent is blocked on approval from the user for submitting input.
        is_blocked: bool,
        /// `true` if agent responses should be hidden in the UI.
        should_hide_responses: bool,
    },
    /// The user is in control.
    User { reason: UserTakeOverReason },
}

impl LongRunningCommandControlState {
    pub fn is_agent_in_control(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    pub fn is_agent_blocked(&self) -> bool {
        matches!(
            self,
            Self::Agent {
                is_blocked: true,
                ..
            }
        )
    }

    pub fn is_user_in_control(&self) -> bool {
        matches!(self, Self::User { .. })
    }

    /// Returns `true` if a completing user-controlled command should auto-resume the conversation.
    pub fn should_auto_resume(&self) -> bool {
        match self {
            Self::Agent { .. } => false,
            Self::User { reason } => reason.should_auto_resume(),
        }
    }

    pub fn should_hide_responses(&self) -> bool {
        matches!(
            self,
            Self::Agent {
                should_hide_responses: true,
                ..
            }
        )
    }

    pub fn user_take_over_reason(&self) -> Option<&UserTakeOverReason> {
        match &self {
            LongRunningCommandControlState::Agent { .. } => None,
            LongRunningCommandControlState::User { reason } => Some(reason),
        }
    }
}

/// Responsible for managing 'control' (e.g. write permissions) for the active long running
/// agent-requested command.
///
/// Control state is canonically stored on the relevant command `Block` owned by terminal model,
/// but wrapping update APIs in this controller ensures consistent update semantics and makes
/// control state updates subscribable.
pub struct CLISubagentController {
    controller: ModelHandle<BlocklistAIController>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    agent_view_controller: Option<ModelHandle<AgentViewController>>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    terminal_view_id: EntityId,
    // Active or recently-active CLI subagent state, keyed by the associated block.
    active_subagents_by_block: HashMap<BlockId, ActiveCLISubagentState>,
}

impl CLISubagentController {
    pub fn new(
        controller: &ModelHandle<BlocklistAIController>,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        agent_view_controller: Option<ModelHandle<AgentViewController>>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, Self::handle_history_model_event);

        ctx.subscribe_to_model(action_model, |me, _, event, ctx| match event {
            BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_) => {
                let mut terminal_model = me.terminal_model.lock();
                let active_block = terminal_model.block_list_mut().active_block_mut();
                active_block.update_is_agent_blocked(true);

                let action_id = active_block.requested_command_action_id().cloned();
                ctx.emit(CLISubagentEvent::UpdatedControl {
                    block_id: active_block.id().clone(),
                    requested_command_action_id: action_id,
                    agent_has_control: active_block.is_agent_in_control(),
                });
            }
            BlocklistAIActionEvent::ExecutingAction(..) => {
                let mut terminal_model = me.terminal_model.lock();
                let active_block = terminal_model.block_list_mut().active_block_mut();
                active_block.update_is_agent_blocked(false);

                let action_id = active_block.requested_command_action_id().cloned();
                ctx.emit(CLISubagentEvent::UpdatedControl {
                    block_id: active_block.id().clone(),
                    requested_command_action_id: action_id,
                    agent_has_control: active_block.is_agent_in_control(),
                });
            }
            BlocklistAIActionEvent::FinishedAction { action_id, .. } => {
                let snapshot_block_id = me
                    .action_model
                    .as_ref(ctx)
                    .get_action_result(action_id)
                    .and_then(|result| snapshot_block_id_for_action_result(&result.result))
                    .cloned();
                let mut terminal_model = me.terminal_model.lock();
                let active_block = terminal_model.block_list_mut().active_block_mut();
                active_block.update_is_agent_blocked(false);

                let action_id = active_block.requested_command_action_id().cloned();
                ctx.emit(CLISubagentEvent::UpdatedControl {
                    block_id: active_block.id().clone(),
                    requested_command_action_id: action_id,
                    agent_has_control: active_block.is_agent_in_control(),
                });

                // Updates the last snapshot timestamp for the active block after the agent has read the block output.
                if let Some(snapshot_block_id) = snapshot_block_id {
                    me.active_subagents_by_block
                        .entry(snapshot_block_id.clone())
                        .or_default()
                        .last_snapshot_at = Some(Instant::now());
                    ctx.emit(CLISubagentEvent::UpdatedLastSnapshot);
                }
            }
            _ => (),
        });

        ctx.subscribe_to_model(model_event_dispatcher, |me, _, event, ctx| {
            if let ModelEvent::BlockCompleted(block_completed_event) = event {
                let terminal_model = me.terminal_model.lock();
                let Some(block) = terminal_model
                    .block_list()
                    .block_with_id(&block_completed_event.block_id)
                else {
                    return;
                };

                let block_id = block.id().clone();
                let conversation_id = block.ai_conversation_id();
                let requested_command_action_id = block.requested_command_action_id().cloned();
                let was_agent_tagged_in = block.interaction_mode().is_agent_tagged_in();
                let has_agent_metadata = block.agent_interaction_metadata().is_some();
                drop(terminal_model);
                let removed_subagent_state = me.active_subagents_by_block.remove(&block_id);
                if removed_subagent_state
                    .as_ref()
                    .is_some_and(|state| state.last_snapshot_at.is_some())
                {
                    ctx.emit(CLISubagentEvent::UpdatedLastSnapshot);
                }

                if removed_subagent_state
                    .as_ref()
                    .is_some_and(|state| state.task_id.is_some())
                {
                    let is_inline_agent_view =
                        me.agent_view_controller.as_ref().is_some_and(|controller| {
                            controller.read(ctx, |controller, _| controller.is_inline())
                        });

                    if is_inline_agent_view {
                        // Mark conversation as successfully completed BEFORE exiting agent view.
                        // The command finished naturally, so this is a successful completion.
                        if let Some(conversation_id) = conversation_id {
                            me.controller.update(ctx, |controller, ctx| {
                                controller.cancel_conversation_progress(
                                    conversation_id,
                                    CancellationReason::CommandFinishedDuringInlineAgentView,
                                    ctx,
                                );
                            });
                        }
                    }

                    ctx.emit(CLISubagentEvent::FinishedSubagent {
                        block_id,
                        conversation_id,
                        initial_requested_command_action_id: requested_command_action_id,
                    });
                }

                // Exit inline agent view if agent was tagged in or had metadata (was in control).
                if let Some(agent_view_controller) = &me.agent_view_controller {
                    agent_view_controller.update(ctx, |controller, ctx| {
                        if controller.is_inline() && (was_agent_tagged_in || has_agent_metadata) {
                            controller.exit_agent_view(ctx);
                        }
                    });
                }
            }
        });

        Self {
            controller: controller.clone(),
            action_model: action_model.clone(),
            agent_view_controller,
            terminal_model,
            terminal_view_id,
            active_subagents_by_block: HashMap::new(),
        }
    }

    pub fn is_agent_in_control(&self) -> bool {
        let terminal_model = self.terminal_model.lock();
        terminal_model
            .block_list()
            .active_block()
            .is_agent_in_control()
    }

    pub(crate) fn is_agent_in_control_or_tagged_in(&self) -> bool {
        let terminal_model = self.terminal_model.lock();
        terminal_model
            .block_list()
            .active_block()
            .is_agent_in_control_or_tagged_in()
    }

    pub fn last_snapshot_at(&self, block_id: &BlockId) -> Option<Instant> {
        self.active_subagents_by_block
            .get(block_id)
            .and_then(|state| state.last_snapshot_at)
    }

    pub fn set_latest_instruction(
        &mut self,
        block_id: BlockId,
        instruction: String,
        ctx: &mut ModelContext<Self>,
    ) -> Option<String> {
        let previous = self
            .active_subagents_by_block
            .entry(block_id.clone())
            .or_default()
            .latest_instruction
            .replace(instruction);
        ctx.emit(CLISubagentEvent::UpdatedInstruction { block_id });
        previous
    }

    pub fn restore_latest_instruction(
        &mut self,
        block_id: BlockId,
        instruction: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(state) = self.active_subagents_by_block.get_mut(&block_id) {
            state.latest_instruction = instruction;
            ctx.emit(CLISubagentEvent::UpdatedInstruction { block_id });
        }
    }
    /// Returns the CLI subagent associated with the active command block.
    pub fn active_target(&self) -> Option<CLISubagentTarget> {
        let terminal_model = self.terminal_model.lock();
        let block = terminal_model.block_list().active_block();
        if !block.is_active_and_long_running() {
            return None;
        }
        self.target_for_block_in_model(block.id(), &terminal_model)
    }

    /// Returns the CLI subagent associated with `block_id`.
    pub fn target_for_block(&self, block_id: &BlockId) -> Option<CLISubagentTarget> {
        let terminal_model = self.terminal_model.lock();
        self.target_for_block_in_model(block_id, &terminal_model)
    }

    fn target_for_block_in_model(
        &self,
        block_id: &BlockId,
        terminal_model: &TerminalModel,
    ) -> Option<CLISubagentTarget> {
        let block = terminal_model.block_list().block_with_id(block_id)?;
        Some(CLISubagentTarget {
            block_id: block.id().clone(),
            task_id: block.cli_subagent_task_id()?.clone(),
            conversation_id: block.ai_conversation_id()?,
            requested_command_action_id: block.requested_command_action_id().cloned(),
            control_state: block.long_running_control_state()?.clone(),
            last_snapshot_at: self.last_snapshot_at(block_id),
            latest_instruction: self
                .active_subagents_by_block
                .get(block_id)
                .and_then(|state| state.latest_instruction.clone()),
        })
    }

    /// Force the currently in-flight poll for the given long-running command block to
    /// resolve immediately with a fresh snapshot, bypassing the agent-set timeout.
    /// Backs the `Check now` affordance surfaced next to the `Last seen by agent ...`
    /// indicator in the warping footer.
    pub fn request_force_refresh(&self, block_id: &BlockId, ctx: &mut ModelContext<Self>) {
        let executor_handle = self.action_model.as_ref(ctx).shell_command_executor(ctx);
        let block_id = block_id.clone();
        executor_handle.update(ctx, move |executor, _| {
            executor.force_refresh_block(&block_id);
        });
    }

    pub fn switch_control_to_user(&self, reason: UserTakeOverReason, ctx: &mut ModelContext<Self>) {
        let should_cancel_conversation = !reason.is_transfer_from_agent();
        let mut terminal_model = self.terminal_model.lock();

        let active_block = terminal_model.block_list_mut().active_block_mut();
        let block_id = active_block.id().clone();
        let interaction_mode_debug = format!("{:?}", active_block.interaction_mode());
        let lrc_state_debug = format!("{:?}", active_block.long_running_control_state());
        if let Err(e) = active_block.take_over_control_for_user(reason.clone()) {
            report_error!(
                anyhow::Error::new(e).context("Failed to take control for user"),
                extra: {
                    "reason" => ?reason,
                    "block_id" => ?block_id,
                    "interaction_mode" => %interaction_mode_debug,
                    "lrc_state" => %lrc_state_debug
                }
            );
            return;
        }

        let action_id = active_block.requested_command_action_id().cloned();
        let conversation_id = active_block.ai_conversation_id();
        let agent_has_control = active_block.is_agent_in_control();
        // Conversation cancellation potentially takes a lock on terminal model if the
        // cancelled action is a shell command action, so we have to drop the terminal
        // model lock before actually cancelling the conversation.
        drop(terminal_model);

        // Cancel the in-flight stream to stop the CLI subagent monitoring loop.
        // When the user manually takes over, we use CLISubagentUserTakeover so the
        // conversation status stays InProgress — the agent will resume once the command
        // finishes or the user hands control back. We do NOT use ManuallyCancelled here
        // because that would mark the conversation (and ambient task) as cancelled,
        // which is incorrect since the conversation is still proceeding.
        if should_cancel_conversation && let Some(conversation_id) = conversation_id {
            self.controller.update(ctx, |controller, ctx| {
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::CLISubagentUserTakeover,
                    ctx,
                );
            });
        }

        ctx.emit(CLISubagentEvent::UpdatedControl {
            block_id: block_id.clone(),
            requested_command_action_id: action_id,
            agent_has_control,
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::CLISubagentControlStateChanged {
                conversation_id,
                block_id,
                control_state: CLISubagentControlState::UserInControl,
            },
            ctx
        );
    }

    pub fn handoff_active_command_control_to_agent(&self, ctx: &mut ModelContext<Self>) {
        let mut terminal_model = self.terminal_model.lock();

        let active_block = terminal_model.block_list_mut().active_block_mut();
        let conversation_id = active_block.ai_conversation_id();
        let block_id = active_block.id().clone();
        let lrc_state_debug = format!("{:?}", active_block.long_running_control_state());
        // Check if control was transferred from agent before handoff.
        let was_transfer_from_agent = active_block
            .long_running_control_state()
            .and_then(|state| state.user_take_over_reason())
            .is_some_and(|reason| reason.is_transfer_from_agent());
        if let Err(e) = active_block.handoff_control_to_agent() {
            report_error!(
                anyhow::Error::new(e).context("Failed to handoff control to agent"),
                extra: { "block_id" => ?block_id, "lrc_state" => %lrc_state_debug }
            );
            return;
        }
        log::info!(
            "handoff_active_command_control_to_agent: block_id={block_id:?}, \
             conversation_id={conversation_id:?}, lrc_state={lrc_state_debug}"
        );
        let action_id = active_block.requested_command_action_id().cloned();
        let agent_has_control = active_block.is_agent_in_control();
        drop(terminal_model);
        if let Some(agent_view_controller) = &self.agent_view_controller {
            agent_view_controller.update(ctx, |controller, ctx| {
                if !controller.is_inline()
                    && let Err(e) = controller.try_enter_inline_agent_view(
                        conversation_id,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                {
                    report_error!(
                        anyhow::Error::new(e)
                            .context("Failed to enter inline agent view for LRC handoff")
                    );
                }
            });
        }

        // Trigger an auto-resume of the conversation when handing control to the agent.
        if let Some(conversation_id) = conversation_id {
            let is_viewing_shared_session = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .is_some_and(|conversation| conversation.is_viewing_shared_session());
            if !is_viewing_shared_session {
                let resume_context = {
                    let terminal_model = self.terminal_model.lock();
                    block_context_from_terminal_model(&terminal_model, &block_id, false)
                        .map(Box::new)
                        .map(AIAgentContext::Block)
                        .into_iter()
                        .collect()
                };
                self.controller.update(ctx, |controller, ctx| {
                    controller.resume_conversation(
                        conversation_id,
                        /*can_attempt_resume_on_error*/ true,
                        /*is_auto_resume_after_error*/ false,
                        resume_context,
                        ctx,
                    );
                });
            }
        }

        ctx.emit(CLISubagentEvent::UpdatedControl {
            block_id: block_id.clone(),
            requested_command_action_id: action_id,
            agent_has_control,
        });

        // Emit a special event if control was transferred from agent, so the executor can be notified.
        if was_transfer_from_agent {
            ctx.emit(CLISubagentEvent::ControlHandedBackAfterTransfer);
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::CLISubagentControlStateChanged {
                conversation_id,
                block_id,
                control_state: CLISubagentControlState::AgentInControl,
            },
            ctx
        );
    }

    pub fn toggle_hide_responses(&self, ctx: &mut ModelContext<Self>) {
        let mut terminal_model = self.terminal_model.lock();
        let active_block = terminal_model.block_list_mut().active_block_mut();

        if active_block.toggle_subagent_response_visibility() {
            let conversation_id = active_block.ai_conversation_id();
            let block_id = active_block.id().clone();
            let is_hidden = active_block.should_hide_responses();

            ctx.emit(CLISubagentEvent::ToggledHideResponses);

            if let Some(conversation_id) = conversation_id {
                send_telemetry_from_ctx!(
                    TelemetryEvent::CLISubagentResponsesToggled {
                        conversation_id,
                        block_id,
                        is_hidden,
                    },
                    ctx
                );
            }
        }
    }

    fn handle_history_model_event(
        &mut self,
        _: ModelHandle<BlocklistAIHistoryModel>,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if event
            .terminal_surface_id()
            .is_some_and(|id| id != self.terminal_view_id)
        {
            return;
        }
        match event {
            BlocklistAIHistoryEvent::CreatedSubtask {
                task_id,
                conversation_id,
                ..
            } => {
                let history_model = BlocklistAIHistoryModel::handle(ctx);
                let Some(cli_subagent_block_id) = history_model
                    .as_ref(ctx)
                    .conversation(conversation_id)
                    .and_then(|c| c.get_task(task_id))
                    .and_then(|task| task.cli_subagent_block_id())
                else {
                    return;
                };

                let mut terminal_model = self.terminal_model.lock();
                let Some(block) = terminal_model
                    .block_list_mut()
                    .mut_block_from_id(&cli_subagent_block_id)
                else {
                    return;
                };
                let block_id = block.id().clone();
                if let Err(e) = block.set_agent_interaction_mode_for_agent_monitored_command(
                    task_id,
                    *conversation_id,
                ) {
                    report_error!(
                        anyhow::Error::new(e)
                            .context("Could not update interaction mode to agent-monitored")
                    );
                    return;
                };

                let action_id = block.requested_command_action_id().cloned();
                let agent_has_control = block.is_agent_in_control();
                drop(terminal_model);

                // When the CLI subagent is first created for a long running command,
                // the agent now has control. Emit an UpdatedControl event so that
                // shared-session state can reflect this initial control state.
                ctx.emit(CLISubagentEvent::UpdatedControl {
                    block_id: block_id.clone(),
                    requested_command_action_id: action_id.clone(),
                    agent_has_control,
                });
                self.active_subagents_by_block
                    .entry(block_id.clone())
                    .or_default()
                    .task_id = Some(task_id.clone());

                ctx.emit(CLISubagentEvent::SpawnedSubagent {
                    task_id: task_id.clone(),
                    conversation_id: *conversation_id,
                    block_id: block_id.clone(),
                    initial_requested_command_action_id: action_id,
                });
            }
            BlocklistAIHistoryEvent::UpgradedTask {
                optimistic_id: old_id,
                server_id: new_id,
                ..
            } => {
                let block_id =
                    self.active_subagents_by_block
                        .iter()
                        .find_map(|(block_id, state)| {
                            (state.task_id.as_ref() == Some(old_id)).then_some(block_id.clone())
                        });
                if let Some(block_id) = block_id {
                    let mut terminal_model = self.terminal_model.lock();
                    if let Some(block) =
                        terminal_model.block_list_mut().mut_block_from_id(&block_id)
                    {
                        match block.upgrade_cli_subagent_task_id(new_id.clone()) {
                            Ok(()) => {
                                if let Some(state) =
                                    self.active_subagents_by_block.get_mut(&block_id)
                                {
                                    state.task_id = Some(new_id.clone());
                                }
                            }
                            Err(e) => {
                                report_error!(e.context(
                                    "Tried to upgrade CLISubagent task ID for non-existent block"
                                ));
                            }
                        }
                    }
                }
            }
            _ => (),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CLISubagentEvent {
    // Emitted when a CLI subagent is spawned for a running command block.
    SpawnedSubagent {
        task_id: TaskId,
        block_id: BlockId,
        conversation_id: AIConversationId,

        /// The ID of the requested command for which this subagent was spawned, if any.
        ///
        /// None if the subagent was spawned by entering agent mode during a user-executed command,
        /// rather than a requested command.
        initial_requested_command_action_id: Option<AIAgentActionId>,
    },
    // Emitted when a CLI subagent's execution ends.
    FinishedSubagent {
        block_id: BlockId,
        conversation_id: Option<AIConversationId>,
        initial_requested_command_action_id: Option<AIAgentActionId>,
    },
    UpdatedControl {
        block_id: BlockId,
        requested_command_action_id: Option<AIAgentActionId>,
        agent_has_control: bool,
    },
    UpdatedInstruction {
        block_id: BlockId,
    },
    UpdatedLastSnapshot,
    ToggledHideResponses,
    /// Emitted when the user hands control back to the agent after a
    /// TransferShellCommandControlToUser action.
    ControlHandedBackAfterTransfer,
}

impl CLISubagentEvent {
    pub fn block_id(&self) -> Option<&BlockId> {
        match self {
            Self::SpawnedSubagent { block_id, .. }
            | Self::FinishedSubagent { block_id, .. }
            | Self::UpdatedControl { block_id, .. }
            | Self::UpdatedInstruction { block_id } => Some(block_id),
            Self::UpdatedLastSnapshot
            | Self::ToggledHideResponses
            | Self::ControlHandedBackAfterTransfer => None,
        }
    }
}

impl Entity for CLISubagentController {
    type Event = CLISubagentEvent;
}

fn snapshot_block_id_for_action_result(result: &AIAgentActionResultType) -> Option<&BlockId> {
    // Enumerates all possible action result types that read a command output.
    match result {
        AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::LongRunningCommandSnapshot { block_id, .. },
        ) => Some(block_id),
        AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::Snapshot { block_id, .. },
        ) => Some(block_id),
        AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::LongRunningCommandSnapshot { block_id, .. },
        ) => Some(block_id),
        AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::Snapshot { block_id, .. },
        ) => Some(block_id),
        _ => None,
    }
}

#[cfg(test)]
#[path = "cli_controller_tests.rs"]
mod tests;
