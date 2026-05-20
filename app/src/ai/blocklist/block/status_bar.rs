use std::{collections::HashSet, sync::Arc, time::Duration};

use super::{
    cli_controller::{CLISubagentController, CLISubagentEvent, UserTakeOverReason},
    model::{AIBlockModel, AIBlockModelImpl, AIBlockOutputStatus},
    view_impl::common::{
        render_switch_control_to_user_button, render_warping_indicator,
        render_warping_indicator_base, ButtonProps, ForceRefreshButtonProps, MaybeShimmeringText,
        WarpingIndicatorProps, WarpingProps, LOAD_OUTPUT_MESSAGE, WAITING_FOR_USER_INPUT_MESSAGE,
    },
};
use crate::{
    ai::agent_tips::AITipModel,
    terminal::{
        input::buffer_model::InputBufferUpdateEvent,
        view::ambient_agent::is_cloud_agent_pre_first_exchange,
    },
};
use crate::{
    ai::blocklist::agent_view::{
        agent_view_bg_fill, child_agent_status_card::ChildAgentStatusCard, AgentMessageBar,
        AgentViewController, EphemeralMessageModel,
    },
    terminal::input::{
        buffer_model::InputBufferModel,
        message_bar::common::render_standard_message_bar,
        message_bar::{Message, MessageItem},
        slash_command_model::SlashCommandModel,
        suggestions_mode_model::InputSuggestionsModeModel,
        HandoffComposeState,
    },
};
use warp_multi_agent_api as api;

use crate::{
    ai::{
        agent::{
            conversation::AIConversationId, icons, AIAgentExchangeId, AIAgentOutput,
            AIAgentOutputMessageType, CancellationReason, SummarizationType,
        },
        blocklist::{
            agent_view::shortcuts::AgentShortcutViewModel,
            ai_brand_color,
            model::AIBlockModelHelper,
            summarization_cancel_dialog::{
                self, SummarizationCancelDialog, SummarizationCancelDialogEvent,
            },
            BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIContextEvent,
            BlocklistAIContextModel, BlocklistAIController, BlocklistAIHistoryEvent,
            BlocklistAIInputEvent, BlocklistAIInputModel, ResponseStreamId,
        },
        llms::LLMPreferences,
        AgentTip,
    },
    send_telemetry_from_app_ctx,
    server::telemetry::TelemetryEvent,
    settings::{InputModeSettings, InputSettings},
    settings_view::keybindings::KeybindingChangedNotifier,
    terminal::{
        input::SET_INPUT_MODE_TERMINAL_ACTION_NAME,
        model::block::LONG_RUNNING_COMMAND_DURATION_MS,
        model_events::{ModelEvent, ModelEventDispatcher},
        view::ambient_agent::{AmbientAgentViewModel, AmbientAgentViewModelEvent},
        warpify::render::LEFT_STRIPE_WIDTH,
        TerminalModel, CANCEL_COMMAND_KEYBINDING, TOGGLE_AUTOEXECUTE_MODE_KEYBINDING,
        TOGGLE_HIDE_CLI_RESPONSES_KEYBINDING, TOGGLE_QUEUE_NEXT_PROMPT_KEYBINDING,
    },
    util::bindings::keybinding_name_to_keystroke,
    BlocklistAIHistoryModel,
};
use instant::Instant;
use parking_lot::FairMutex;
use pathfinder_color::ColorU;
use warp_core::{
    features::FeatureFlag,
    ui::{appearance::Appearance, theme::Fill, Icon as CoreIcon},
};
use warpui::elements::shimmering_text::ShimmeringTextStateHandle;
use warpui::{
    elements::{Border, Container, Empty, Flex, MouseStateHandle, ParentElement, Text},
    keymap::Keystroke,
    presenter::ChildView,
    r#async::SpawnedFutureHandle,
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, View, ViewContext,
    ViewHandle,
};
use warpui::{r#async::Timer, TypedActionView};

pub fn init(app: &mut AppContext) {
    summarization_cancel_dialog::init(app);
}

#[derive(Default)]
struct StateHandles {
    autoexecute_button: MouseStateHandle,
    queue_next_prompt_button: MouseStateHandle,
    stop_button: MouseStateHandle,
    take_over_button: MouseStateHandle,
    hide_cli_responses_button: MouseStateHandle,
    github_auth_link: MouseStateHandle,
    /// Tracks hover/press state for the inline `Check now` affordance rendered next to
    /// `Last seen by agent ...` while the agent is polling a long-running command.
    force_refresh_button: MouseStateHandle,
}

pub struct BlocklistAIStatusBar {
    active_exchange_model: Option<Box<dyn AIBlockModel<View = BlocklistAIStatusBar>>>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    controller: ModelHandle<BlocklistAIController>,
    cli_subagent_controller: ModelHandle<CLISubagentController>,
    context_model: ModelHandle<BlocklistAIContextModel>,
    input_model: ModelHandle<BlocklistAIInputModel>,
    agent_view_controller: ModelHandle<AgentViewController>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    shimmering_text_handle: ShimmeringTextStateHandle,
    state_handles: StateHandles,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,

    autoexecute_keystroke: Option<Keystroke>,
    queue_next_prompt_keystroke: Option<Keystroke>,
    stop_keystroke: Option<Keystroke>,
    set_terminal_input_keystroke: Option<Keystroke>,
    hide_cli_responses_keystroke: Option<Keystroke>,

    // Whether the summarization cancellation confirmation dialog is open.
    is_summarization_cancel_dialog_open: bool,
    summarization_cancel_dialog: ViewHandle<SummarizationCancelDialog>,

    /// Handle for the periodic timer that updates the summarization timer UI.
    summarization_timer_handle: Option<SpawnedFutureHandle>,
    summarization_start_time: Option<Instant>,
    /// Handle for the 1-second periodic timer that refreshes the "Last read …" suffix in
    /// the warping indicator while the active block has a recorded LRC snapshot.
    last_read_refresh_handle: Option<SpawnedFutureHandle>,

    latest_response_stream_id: Option<ResponseStreamId>,

    /// Agent tip to display below the warping indicator.
    current_tip: Option<AgentTip>,

    ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
    agent_message_bar: ViewHandle<AgentMessageBar>,
    child_agent_status_card: ViewHandle<ChildAgentStatusCard>,
}

impl BlocklistAIStatusBar {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controller: ModelHandle<BlocklistAIController>,
        agent_view_controller: ModelHandle<AgentViewController>,
        cli_subagent_controller: ModelHandle<CLISubagentController>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        context_model: ModelHandle<BlocklistAIContextModel>,
        input_model: ModelHandle<BlocklistAIInputModel>,
        input_buffer_model: ModelHandle<InputBufferModel>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        shortcut_view_model: ModelHandle<AgentShortcutViewModel>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        input_suggestions_model: ModelHandle<InputSuggestionsModeModel>,
        slash_command_model: ModelHandle<SlashCommandModel>,
        ephemeral_message_model: ModelHandle<EphemeralMessageModel>,
        handoff_compose_state: ModelHandle<HandoffComposeState>,
        terminal_view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, _, event, ctx| {
            if event
                .terminal_view_id()
                .is_some_and(|id| id != terminal_view_id)
            {
                return;
            }
            match event {
                BlocklistAIHistoryEvent::AppendedExchange {
                    response_stream_id,
                    exchange_id,
                    conversation_id,
                    ..
                } => {
                    if let Some(response_stream_id) = response_stream_id.clone() {
                        me.latest_response_stream_id = Some(response_stream_id);
                    }
                    me.reset_model_for_exchange(*exchange_id, *conversation_id, ctx);
                }
                BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. } => {
                    me.active_exchange_model = None;
                    ctx.notify();
                }
                BlocklistAIHistoryEvent::ClearedActiveConversation {
                    conversation_id, ..
                }
                | BlocklistAIHistoryEvent::RemoveConversation {
                    conversation_id, ..
                } => {
                    if me.active_exchange_model.as_ref().is_some_and(|model| {
                        model
                            .conversation_id(ctx)
                            .is_some_and(|id| id == *conversation_id)
                    }) {
                        me.active_exchange_model = None;
                        ctx.notify();
                    }
                }
                BlocklistAIHistoryEvent::UpdatedConversationStatus { .. } => {
                    ctx.notify();
                }
                BlocklistAIHistoryEvent::SetActiveConversation {
                    conversation_id, ..
                } => {
                    let Some(conversation) =
                        BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
                    else {
                        return;
                    };
                    let Some(new_latest_exchange_id) =
                        conversation.latest_exchange().map(|exchange| exchange.id)
                    else {
                        return;
                    };

                    me.reset_model_for_exchange(new_latest_exchange_id, conversation.id(), ctx);
                }
                BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. } => ctx.notify(),
                _ => (),
            }
        });
        ctx.subscribe_to_model(&context_model, |_, _, event, ctx| {
            if matches!(
                event,
                BlocklistAIContextEvent::PendingQueryStateUpdated
                    | BlocklistAIContextEvent::QueueNextPromptToggled
            ) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&input_model, |_, _, event, ctx| {
            if let BlocklistAIInputEvent::InputTypeChanged { .. } = event {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(
            &cli_subagent_controller,
            move |me, _, event, ctx| match event {
                CLISubagentEvent::UpdatedControl { .. }
                | CLISubagentEvent::ToggledHideResponses => {
                    ctx.notify();
                }
                CLISubagentEvent::UpdatedLastSnapshot => {
                    let has_active_snapshot = me.should_refresh_last_read_timer(ctx);
                    if has_active_snapshot {
                        me.start_last_read_timer(ctx);
                    } else {
                        me.stop_last_read_timer();
                    }
                    ctx.notify();
                }
                _ => {}
            },
        );
        ctx.subscribe_to_model(&input_buffer_model, |me, _, event, ctx| {
            let InputBufferUpdateEvent {
                old_content: old,
                new_content: new,
                ..
            } = event;
            if !me.input_model.as_ref(ctx).is_ai_input_enabled() && old.is_empty() != new.is_empty()
            {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&agent_view_controller, |_, _, _, ctx| ctx.notify());

        let input_settings = InputSettings::handle(ctx);
        ctx.subscribe_to_model(&input_settings, |_, _, _, ctx| ctx.notify());
        let input_mode_settings = InputModeSettings::handle(ctx);
        ctx.subscribe_to_model(&input_mode_settings, |_, _, _, ctx| ctx.notify());
        let stop_keystroke = keybinding_name_to_keystroke(CANCEL_COMMAND_KEYBINDING, ctx);
        let autoexecute_keystroke =
            keybinding_name_to_keystroke(TOGGLE_AUTOEXECUTE_MODE_KEYBINDING, ctx);
        let queue_next_prompt_keystroke =
            keybinding_name_to_keystroke(TOGGLE_QUEUE_NEXT_PROMPT_KEYBINDING, ctx);
        let set_terminal_input_keystroke =
            keybinding_name_to_keystroke(SET_INPUT_MODE_TERMINAL_ACTION_NAME, ctx);
        let hide_cli_responses_keystroke =
            keybinding_name_to_keystroke(TOGGLE_HIDE_CLI_RESPONSES_KEYBINDING, ctx);
        ctx.subscribe_to_model(&KeybindingChangedNotifier::handle(ctx), |me, _, _, ctx| {
            me.stop_keystroke = keybinding_name_to_keystroke(CANCEL_COMMAND_KEYBINDING, ctx);
            me.autoexecute_keystroke =
                keybinding_name_to_keystroke(TOGGLE_AUTOEXECUTE_MODE_KEYBINDING, ctx);
            me.queue_next_prompt_keystroke =
                keybinding_name_to_keystroke(TOGGLE_QUEUE_NEXT_PROMPT_KEYBINDING, ctx);
            me.set_terminal_input_keystroke =
                keybinding_name_to_keystroke(SET_INPUT_MODE_TERMINAL_ACTION_NAME, ctx);
            ctx.notify();
        });

        let summarization_cancel_dialog =
            ctx.add_typed_action_view(|_| SummarizationCancelDialog::default());
        ctx.subscribe_to_view(
            &summarization_cancel_dialog,
            |me, _, event, ctx| match event {
                SummarizationCancelDialogEvent::ConfirmCancel => {
                    me.cancel_active_request_or_action(ctx);
                    me.close_summarization_cancel_dialog(ctx);
                }
                SummarizationCancelDialogEvent::Continue => {
                    me.close_summarization_cancel_dialog(ctx);
                }
            },
        );

        ctx.subscribe_to_model(&action_model, |_, _, event, ctx| match event {
            BlocklistAIActionEvent::ExecutingAction(..)
            | BlocklistAIActionEvent::FinishedAction { .. } => ctx.notify(),
            _ => (),
        });
        ctx.subscribe_to_model(model_event_dispatcher, |me, _, event, ctx| match event {
            ModelEvent::AfterBlockStarted { block_id, .. } => {
                let terminal_model = me.terminal_model.lock();
                if terminal_model
                    .block_list()
                    .block_with_id(block_id)
                    .is_some_and(|block| block.agent_interaction_metadata().is_some())
                {
                    ctx.spawn(
                        Timer::after(Duration::from_millis(LONG_RUNNING_COMMAND_DURATION_MS)),
                        |_, _, ctx| ctx.notify(),
                    );
                }
            }
            ModelEvent::BlockCompleted(_) => {
                ctx.notify();
            }
            _ => (),
        });

        ctx.subscribe_to_model(&ephemeral_message_model, |_, _, _, ctx| {
            ctx.notify();
        });

        let agent_message_bar = ctx.add_view(|ctx| {
            AgentMessageBar::new(
                agent_view_controller.clone(),
                ephemeral_message_model.clone(),
                shortcut_view_model.clone(),
                input_buffer_model,
                input_model.clone(),
                input_suggestions_model,
                slash_command_model,
                context_model.clone(),
                handoff_compose_state,
                terminal_model.clone(),
                ctx,
            )
        });

        let child_agent_status_card = ctx.add_typed_action_view(|ctx| {
            ChildAgentStatusCard::new(agent_view_controller.clone(), ctx)
        });
        if let Some(ambient_agent_view_model) = ambient_agent_view_model.as_ref() {
            ctx.subscribe_to_model(ambient_agent_view_model, |me, _, event, ctx| match event {
                AmbientAgentViewModelEvent::DispatchedAgent
                | AmbientAgentViewModelEvent::ProgressUpdated => {
                    me.update_agent_tip(ctx);
                    ctx.notify();
                }
                AmbientAgentViewModelEvent::SessionReady { .. }
                | AmbientAgentViewModelEvent::Failed { .. }
                | AmbientAgentViewModelEvent::NeedsGithubAuth
                | AmbientAgentViewModelEvent::Cancelled => {
                    ctx.notify();
                }
                _ => (),
            });
        }

        Self {
            active_exchange_model: None,
            shimmering_text_handle: ShimmeringTextStateHandle::new(),
            action_model,
            context_model,
            input_model,
            terminal_model,
            controller,
            agent_view_controller,
            cli_subagent_controller,
            state_handles: Default::default(),
            autoexecute_keystroke,
            queue_next_prompt_keystroke,
            stop_keystroke,
            set_terminal_input_keystroke,
            hide_cli_responses_keystroke,
            summarization_cancel_dialog,
            latest_response_stream_id: None,
            is_summarization_cancel_dialog_open: false,
            summarization_timer_handle: None,
            summarization_start_time: None,
            last_read_refresh_handle: None,
            ambient_agent_view_model,
            current_tip: None,
            ephemeral_message_model,
            agent_message_bar,
            child_agent_status_card,
        }
    }

    pub fn should_show_summarization_cancel_dialog(&self, app: &AppContext) -> bool {
        self.is_summarization_cancel_dialog_open
            && self
                .active_exchange_model
                .as_ref()
                .is_some_and(|model| model.is_conversation_summarization_active(app))
    }
    pub fn summarization_cancel_dialog_handle(&self) -> &ViewHandle<SummarizationCancelDialog> {
        &self.summarization_cancel_dialog
    }

    pub fn handle_ctrl_c(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(model) = self.active_exchange_model.as_ref() else {
            return;
        };

        // Show confirmation dialog if summarization is active and feature flag enabled
        if FeatureFlag::SummarizationCancellationConfirmation.is_enabled()
            && model.is_conversation_summarization_active(ctx)
        {
            // If the dialog is already open, treat Ctrl-C as confirm-cancel
            if self.is_summarization_cancel_dialog_open {
                // Confirm cancel the running/pending action/request, then close the dialog overlay.
                self.cancel_active_request_or_action(ctx);
                self.close_summarization_cancel_dialog(ctx);
                return;
            }

            self.open_summarization_cancel_dialog(ctx);
            return;
        }

        // If summarization isn't active, fall back to canceling the running request/action directly
        self.cancel_active_request_or_action(ctx);
        ctx.notify();
    }

    pub fn notify_and_notify_children(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
        self.agent_message_bar.update(ctx, |_, ctx| ctx.notify());
    }

    fn reset_model_for_exchange(
        &mut self,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let conversation = history_model.conversation(&conversation_id);
        let exchange =
            conversation.and_then(|conversation| conversation.exchange_with_id(exchange_id));

        if self.active_exchange_model.as_ref().is_none_or(|model| {
            model.exchange_id(ctx).is_none_or(|id| id != exchange_id)
                || model
                    .conversation(ctx)
                    .is_none_or(|conversation| conversation.id() != conversation_id)
        }) {
            let Some(conversation) = conversation else {
                self.active_exchange_model = None;
                ctx.notify();
                return;
            };
            self.active_exchange_model = exchange
                .and_then(|e| {
                    AIBlockModelImpl::<BlocklistAIStatusBar>::new(
                        e.id,
                        conversation.id(),
                        false,
                        false,
                        ctx,
                    )
                    .ok()
                })
                .map(|model| {
                    model.on_updated_output(
                        Box::new(|me, ctx| me.on_updated_active_exchange_output(ctx)),
                        ctx,
                    );
                    Box::new(model) as Box<dyn AIBlockModel<View = BlocklistAIStatusBar>>
                });
            self.is_summarization_cancel_dialog_open = false;
            self.stop_summarization_timer();

            if FeatureFlag::AgentTips.is_enabled() {
                self.update_agent_tip(ctx);
            }
        }
    }

    fn on_updated_active_exchange_output(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(model) = self.active_exchange_model.as_ref() else {
            return;
        };
        let status = model.status(ctx);

        // Auto-clear summarization confirmation dialog if summarization is no longer active
        if self.is_summarization_cancel_dialog_open
            && !model.is_conversation_summarization_active(ctx)
        {
            self.is_summarization_cancel_dialog_open = false;
            ctx.emit(
                BlocklistAIStatusBarEvent::SummarizationCancelDialogToggled { is_open: false },
            );
            self.stop_summarization_timer();
        }

        match status {
            AIBlockOutputStatus::PartiallyReceived { output }
            | AIBlockOutputStatus::Complete { output } => {
                let output = output.get();
                self.handle_updated_output(&output, ctx);
            }
            AIBlockOutputStatus::Cancelled { partial_output, .. } => {
                if let Some(output) = partial_output.as_ref() {
                    let output = output.get();
                    self.handle_updated_output(&output, ctx);
                }
            }
            AIBlockOutputStatus::Pending | AIBlockOutputStatus::Failed { .. } => (),
        }

        ctx.notify();
    }

    /// Closes the summarization cancel dialog.
    fn close_summarization_cancel_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_summarization_cancel_dialog_open = false;
        ctx.focus_self();
        ctx.emit(BlocklistAIStatusBarEvent::SummarizationCancelDialogToggled { is_open: false });
        ctx.notify();
    }

    /// Opens the summarization cancel dialog.
    fn open_summarization_cancel_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_summarization_cancel_dialog_open = true;
        ctx.focus(&self.summarization_cancel_dialog);
        ctx.emit(BlocklistAIStatusBarEvent::SummarizationCancelDialogToggled { is_open: true });
        ctx.notify();
    }

    fn handle_updated_output(&mut self, output: &AIAgentOutput, ctx: &mut ViewContext<Self>) {
        // Register element state for reasoning messages and track summarization timing.
        for message in &output.messages {
            // Track summarization start time and token count when summarization message arrives
            if let AIAgentOutputMessageType::Summarization {
                finished_duration,
                summarization_type,
                ..
            } = &message.message
            {
                // Only track conversation summarization, not tool call result summarization
                if matches!(summarization_type, SummarizationType::ConversationSummary) {
                    if finished_duration.is_none() {
                        // Starting summarization - record start time and start periodic updates
                        if self.summarization_start_time.is_none() {
                            self.summarization_start_time = Some(instant::Instant::now());
                            self.start_summarization_timer(ctx);
                            ctx.notify();
                        }
                    } else if self.summarization_start_time.is_some() {
                        self.stop_summarization_timer();
                    }
                }
            }
        }
    }

    /// Cancels either the in-flight request stream or a pending/running action if present.
    /// If neither is found but the conversation is still in progress (e.g., a subagent is running),
    /// cancels the entire conversation's progress.
    fn cancel_active_request_or_action(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(model) = self.active_exchange_model.as_ref() else {
            return;
        };
        if model.status(ctx).is_streaming() {
            if let Some(response_stream_id) = self.latest_response_stream_id.as_ref() {
                self.controller.update(ctx, |controller, ctx| {
                    controller.cancel_request(
                        response_stream_id,
                        CancellationReason::ManuallyCancelled,
                        ctx,
                    );
                });
            }
        } else {
            let Some(conversation_id) = model.conversation_id(ctx) else {
                return;
            };
            let Some(output) = model.status(ctx).output_to_render() else {
                return;
            };
            let actions = output
                .get()
                .actions()
                .map(|action| action.id.clone())
                .collect::<HashSet<_>>();
            if let Some(active_action_id) = self
                .action_model
                .as_ref(ctx)
                .get_pending_or_running_action_id(ctx)
                .filter(|id| actions.contains(id))
                .cloned()
            {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.cancel_action_with_id(
                        conversation_id,
                        &active_action_id,
                        CancellationReason::ManuallyCancelled,
                        ctx,
                    );
                });
            } else if model
                .conversation(ctx)
                .is_some_and(|c| c.status().is_in_progress())
            {
                // No streaming request or pending action, but conversation is still in progress.
                // This happens when a subagent (e.g., computer use or advice) is running.
                // Cancel the entire conversation's progress.
                self.controller.update(ctx, |controller, ctx| {
                    controller.cancel_conversation_progress(
                        conversation_id,
                        CancellationReason::ManuallyCancelled,
                        ctx,
                    );
                });
            }
        }
    }

    /// Starts the periodic timer that updates the summarization UI while summarization is active.
    fn start_summarization_timer(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't start a new timer if one is already running
        if self.summarization_timer_handle.is_some() {
            return;
        }

        // Start a new timer that keeps the elapsed-time indicator fresh.
        let handle = ctx.spawn(
            async move {
                Timer::after(Duration::from_secs(1)).await;
            },
            |me, _unit, ctx| {
                // Clear the handle first so we can restart
                me.summarization_timer_handle = None;

                // Check if summarization is still active
                if me.summarization_start_time.is_some() {
                    ctx.notify();
                    // Restart the timer for the next update
                    me.start_summarization_timer(ctx);
                }
            },
        );

        self.summarization_timer_handle = Some(handle);
    }

    fn stop_summarization_timer(&mut self) {
        self.summarization_start_time = None;
        if let Some(handle) = self.summarization_timer_handle.take() {
            handle.abort();
        }
    }

    fn should_refresh_last_read_timer(&self, ctx: &ViewContext<Self>) -> bool {
        let active_block_id = self
            .terminal_model
            .lock()
            .block_list()
            .active_block()
            .id()
            .clone();
        self.cli_subagent_controller
            .as_ref(ctx)
            .last_snapshot_at(&active_block_id)
            .is_some()
    }

    /// Starts the 1-second periodic timer that keeps the elapsed "Last read Xs ago" suffix
    /// updating in real time. No-ops if the timer is already running or if the active block
    /// no longer has a recorded snapshot.
    fn start_last_read_timer(&mut self, ctx: &mut ViewContext<Self>) {
        if self.last_read_refresh_handle.is_some() || !self.should_refresh_last_read_timer(ctx) {
            return;
        }
        let handle = ctx.spawn(
            async move {
                Timer::after(Duration::from_secs(1)).await;
            },
            |me, _, ctx| {
                me.last_read_refresh_handle = None;
                ctx.notify();
                me.start_last_read_timer(ctx);
            },
        );
        self.last_read_refresh_handle = Some(handle);
    }

    /// Stops and discards the last-read refresh timer.
    fn stop_last_read_timer(&mut self) {
        if let Some(handle) = self.last_read_refresh_handle.take() {
            handle.abort();
        }
    }

    fn update_agent_tip(&mut self, ctx: &mut ViewContext<Self>) {
        if FeatureFlag::AgentTips.is_enabled() && *InputSettings::as_ref(ctx).show_agent_tips {
            let current_working_directory = self
                .terminal_model
                .lock()
                .active_block_metadata()
                .current_working_directory()
                .map(|cwd| cwd.to_string());

            // Update the tip using the model's cooldown-based API
            let tip_model = AITipModel::<AgentTip>::handle(ctx);
            tip_model.update(ctx, |model, model_ctx| {
                model.maybe_refresh_tip(current_working_directory.as_deref(), model_ctx);
            });

            // Get the current tip from the model
            self.current_tip = tip_model.as_ref(ctx).current_tip().cloned();

            if let Some(tip) = self.current_tip.as_ref() {
                send_telemetry_from_app_ctx!(
                    TelemetryEvent::AgentTipShown {
                        tip: tip.description.clone()
                    },
                    ctx
                );
            }
        } else {
            self.current_tip = None;
        }
    }

    fn render_tip(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if FeatureFlag::AgentTips.is_enabled() && *InputSettings::as_ref(app).show_agent_tips {
            self.current_tip
                .as_ref()
                .map(|tip| render_agent_tip(tip, app))
        } else {
            None
        }
    }

    fn render_warping_indicator_for_latest_exchange(
        &self,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let model = self.active_exchange_model.as_ref()?;
        let conversation = model.conversation(app)?;
        let terminal_model = self.terminal_model.lock();
        let active_block = terminal_model.block_list().active_block();
        let has_expanded_requested_command_with_no_subagent = active_block
            .is_active_and_long_running()
            && active_block
                .agent_interaction_metadata()
                .is_some_and(|metadata| {
                    !metadata.should_hide_block() && metadata.long_running_control_state().is_none()
                });
        let should_render_warping = !model.request_type(app).is_passive()
            && !has_expanded_requested_command_with_no_subagent
            && (conversation.status().is_in_progress()
                || (active_block.is_agent_in_control() && !active_block.is_agent_blocked()));

        if !should_render_warping {
            return None;
        }

        let is_agent_in_control = active_block.is_agent_in_control();
        let should_hide_responses = active_block.should_hide_responses();
        let active_block_id = active_block.id().clone();
        let is_agent_blocked = active_block.is_agent_blocked();
        let last_snapshot_at = self
            .cli_subagent_controller
            .as_ref(app)
            .last_snapshot_at(&active_block_id);
        // Surface the inline `Check now` affordance only when the agent has sent at
        // least one snapshot (so there's a known poll in flight) and still holds
        // control of the command. Hidden while the agent is blocked on user
        // confirmation, since no poll future is active.
        let force_refresh_button = (last_snapshot_at.is_some()
            && is_agent_in_control
            && !is_agent_blocked)
            .then_some(ForceRefreshButtonProps {
                button_handle: &self.state_handles.force_refresh_button,
                block_id: active_block_id,
            });

        let output_status = model.status(app);
        let output_to_render = output_status.output_to_render();
        let (current_is_fallback, current_display_name) = output_to_render
            .as_ref()
            .and_then(|o| {
                let o = o.get();
                let m = o.model_info.as_ref()?;
                Some((Some(m.is_fallback), Some(m.display_name.clone())))
            })
            .unwrap_or((None, None));

        let fallback_warping_text = resolve_fallback_warping_message(
            current_is_fallback,
            current_display_name,
            model.as_ref(),
            app,
        );
        let default_warping_text = fallback_warping_text
            .as_deref()
            .unwrap_or(LOAD_OUTPUT_MESSAGE)
            .to_owned();
        let secondary_element = if fallback_warping_text.is_some() {
            Some(render_fallback_explanation(model.as_ref(), app))
        } else {
            self.render_tip(app)
        };

        Some(render_warping_indicator(
            WarpingProps {
                model: model.as_ref(),
                terminal_model: &terminal_model,
                action_model: self.action_model.as_ref(app),
                shimmering_text_handle: &self.shimmering_text_handle,
                summarization_start_time: self.summarization_start_time,
                auto_execute_button: (!model.request_type(app).is_passive_code_diff()).then_some(
                    ButtonProps {
                        button_handle: &self.state_handles.autoexecute_button,
                        keystroke: self.autoexecute_keystroke.as_ref(),
                        is_active: model
                            .conversation(app)
                            .map(|c| c.autoexecute_any_action())
                            .unwrap_or(false),
                    },
                ),
                queue_next_prompt_button: FeatureFlag::QueueSlashCommand.is_enabled().then_some(
                    ButtonProps {
                        button_handle: &self.state_handles.queue_next_prompt_button,
                        keystroke: self.queue_next_prompt_keystroke.as_ref(),
                        is_active: self
                            .context_model
                            .as_ref(app)
                            .is_queue_next_prompt_enabled(),
                    },
                ),
                stop_button: Some(ButtonProps {
                    button_handle: &self.state_handles.stop_button,
                    keystroke: self.stop_keystroke.as_ref(),
                    is_active: false,
                }),
                take_over_lrc_control_button: is_agent_in_control.then_some(ButtonProps {
                    button_handle: &self.state_handles.take_over_button,
                    keystroke: self.set_terminal_input_keystroke.as_ref(),
                    is_active: false,
                }),
                hide_responses_button: is_agent_in_control.then_some((
                    ButtonProps {
                        button_handle: &self.state_handles.hide_cli_responses_button,
                        keystroke: self.hide_cli_responses_keystroke.as_ref(),
                        is_active: false,
                    },
                    should_hide_responses,
                )),
                force_refresh_button,
                default_warping_text,
                secondary_element,
                last_snapshot_at,
            },
            app,
        ))
    }

    fn render_cloud_mode_setup_status(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !FeatureFlag::CloudModeSetupV2.is_enabled() {
            return None;
        }

        let ambient_agent_model = self
            .ambient_agent_view_model
            .as_ref()
            .map(|ambient_agent_view_model| ambient_agent_view_model.as_ref(app))?;

        // The step indicator is only meaningful while a spawn is in flight. Terminal states
        // (`Failed`, `NeedsGithubAuth`, `Cancelled`) still carry an `AgentProgress` for
        // telemetry purposes, so guard on `is_waiting_for_session()` rather than relying on
        // `agent_progress()` being `None`.
        if !ambient_agent_model.is_waiting_for_session() {
            return None;
        }

        let progress = ambient_agent_model.agent_progress()?;
        let progress_text = progress.setup_status_text();
        Some(render_warping_indicator_base(
            WarpingIndicatorProps {
                icon: None,
                warping_indicator_text: MaybeShimmeringText::Shimmering {
                    text: progress_text.into(),
                    shimmering_text_handle: self.shimmering_text_handle.clone(),
                },
                non_shimmering_text: None,
                non_shimmering_suffix: None,
                buttons: None,
                is_passive_code_diff: false,
                secondary_element: self.render_tip(app),
            },
            app,
        ))
    }

    fn render_cloud_mode_setup_terminal_message(&self, app: &AppContext) -> Option<Message> {
        if !FeatureFlag::CloudModeSetupV2.is_enabled() {
            return None;
        }

        let ambient_agent_model = self
            .ambient_agent_view_model
            .as_ref()
            .map(|ambient_agent_view_model| ambient_agent_view_model.as_ref(app))?;
        let theme = Appearance::as_ref(app).theme();
        let error_color = theme.ansi_fg_red();

        if let Some(auth_url) = ambient_agent_model.github_auth_url() {
            return Some(Message::new(vec![
                MessageItem::Icon {
                    icon: CoreIcon::Triangle,
                    color: Some(error_color),
                },
                MessageItem::Text {
                    content: "Missing GitHub authentication. ".into(),
                    color: Some(error_color),
                },
                MessageItem::hyperlink(
                    "Authenticate GitHub",
                    auth_url.to_owned(),
                    self.state_handles.github_auth_link.clone(),
                ),
            ]));
        }

        if ambient_agent_model.is_cancelled() {
            let color = theme.disabled_text_color(theme.background()).into_solid();
            return Some(Message::new(vec![
                MessageItem::Icon {
                    icon: CoreIcon::StopFilled,
                    color: Some(color),
                },
                MessageItem::Text {
                    content: "Cloud agent run cancelled".into(),
                    color: Some(color),
                },
            ]));
        }

        if let Some(error_message) = ambient_agent_model.error_message() {
            return Some(Message::new(vec![
                MessageItem::Icon {
                    icon: CoreIcon::Triangle,
                    color: Some(error_color),
                },
                MessageItem::Text {
                    content: error_message.to_owned().into(),
                    color: Some(error_color),
                },
            ]));
        }

        None
    }
}

/// Checks only the immediately previous exchange for model info (from ModelUsed messages
/// during streaming). We intentionally limit this to a single exchange to avoid reaching
/// back to stale fallback data from much earlier in the conversation.
fn latest_model_used_before_exchange<V: View>(
    model: &dyn AIBlockModel<View = V>,
    app: &AppContext,
) -> Option<api::message::ModelUsed> {
    let conversation = model.conversation(app)?;
    conversation
        .exchanges_reversed()
        .nth(1)
        .and_then(|exchange| {
            let output = exchange.output_status.output()?;
            let output = output.get();
            let model_info = output.model_info.as_ref()?;
            Some(api::message::ModelUsed {
                model_id: model_info.model_id.to_string(),
                model_display_name: model_info.display_name.clone(),
                is_fallback: model_info.is_fallback,
            })
        })
}

fn render_agent_tip(tip: &AgentTip, app: &AppContext) -> Box<dyn Element> {
    use crate::ai::agent_tips::AITip;
    use markdown_parser::{FormattedTextFragment, FormattedTextLine};
    use warpui::text_layout::ClipConfig;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let tip_description = tip.description.clone();
    let action_text = tip.action.clone().and_then(|action| action.display_text());

    let mut fragments = tip.to_formatted_text(app);

    if let (Some(action), Some(text)) = (tip.action.clone(), action_text.clone()) {
        fragments.push(FormattedTextFragment::plain_text(" "));
        fragments.push(FormattedTextFragment::hyperlink_action(text, action));
    } else if let Some(link_target) = tip.link.clone() {
        fragments.push(FormattedTextFragment::plain_text(" "));
        fragments.push(FormattedTextFragment::hyperlink("Learn more", link_target));
    }

    let formatted_text =
        markdown_parser::FormattedText::new(vec![FormattedTextLine::Line(fragments)]);
    warpui::elements::FormattedTextElement::new(
        formatted_text,
        appearance.monospace_font_size() - 3.,
        appearance.ui_font_family(),
        appearance.monospace_font_family(),
        theme.disabled_ui_text_color().into_solid(),
        Default::default(),
    )
    .with_hyperlink_font_color(theme.accent().into())
    .set_selectable(true)
    .with_clip(ClipConfig::ellipsis())
    .register_default_click_handlers_with_action_support(move |link, evt, app| {
        use warpui::elements::HyperlinkLens;
        match link {
            HyperlinkLens::Url(url) => {
                send_telemetry_from_app_ctx!(
                    TelemetryEvent::AgentTipClicked {
                        tip: tip_description.clone(),
                        click_target: url.to_string(),
                    },
                    app
                );
                app.open_url(url);
            }
            HyperlinkLens::Action(action_ref) => {
                if let Some(action) = action_ref
                    .as_any()
                    .downcast_ref::<crate::workspace::WorkspaceAction>()
                {
                    send_telemetry_from_app_ctx!(
                        TelemetryEvent::AgentTipClicked {
                            tip: tip_description.clone(),
                            click_target: action_text.clone().unwrap_or_default(),
                        },
                        app
                    );
                    evt.dispatch_typed_action(action.clone());
                }
            }
        }
    })
    .finish()
}

fn render_fallback_explanation<V: View>(
    model: &dyn AIBlockModel<View = V>,
    app: &AppContext,
) -> Box<dyn Element> {
    let llm_prefs = LLMPreferences::as_ref(app);
    let base_model_id = model.base_model(app);
    let primary_name = base_model_id
        .and_then(|base_id| llm_prefs.get_llm_info(base_id))
        .map(|info| info.base_model_name.as_str());
    let text = match primary_name {
        Some(primary) => {
            format!("The primary model ({primary}) failed. Retrying with the fallback model.")
        }
        None => "The primary model failed. Retrying with the fallback model.".to_owned(),
    };
    let appearance = Appearance::as_ref(app);
    Text::new_inline(
        text,
        appearance.ui_font_family(),
        appearance.monospace_font_size() - 3.,
    )
    .with_color(appearance.theme().disabled_ui_text_color().into_solid())
    .with_selectable(false)
    .soft_wrap(false)
    .finish()
}

/// If the current exchange is using a fallback model, returns the warping message to display
/// (e.g. "Warping with Claude 3.5 Haiku."). When the current exchange's output doesn't have
/// model info yet (the ModelUsed message hasn't arrived), we check the most recent previous
/// exchange as a best guess — if the conversation already fell back, the next exchange likely
/// will too. This avoids a flicker from "Warping..." to "Warping with {name}." on follow-ups.
///
/// We skip the lookback for new user queries because the underlying model may have recovered
/// since the previous exchange. For agent-initiated follow-up exchanges (action results, etc.)
/// the lookback is still applied.
fn resolve_fallback_warping_message<V: View>(
    current_is_fallback: Option<bool>,
    current_display_name: Option<String>,
    model: &dyn AIBlockModel<View = V>,
    app: &AppContext,
) -> Option<String> {
    if !FeatureFlag::FallbackModelLoadOutputMessaging.is_enabled() {
        return None;
    }
    let mut is_fallback = current_is_fallback;
    let mut display_name = current_display_name;
    let is_new_user_query = model
        .conversation(app)
        .and_then(|conv| {
            let exchange_id = model.exchange_id(app)?;
            conv.exchange_with_id(exchange_id)
        })
        .is_some_and(|exchange| exchange.has_user_query());
    if is_fallback.is_none() && !is_new_user_query {
        if let Some(prev) = latest_model_used_before_exchange(model, app) {
            is_fallback = Some(prev.is_fallback);
            if !prev.model_display_name.is_empty() {
                display_name = Some(prev.model_display_name);
            }
        }
    }
    if !is_fallback.unwrap_or(false) {
        return None;
    }
    Some(match display_name.as_deref() {
        Some(name) => format!("Warping with {name}."),
        None => "Warping with another model.".to_owned(),
    })
}

impl View for BlocklistAIStatusBar {
    fn ui_name() -> &'static str {
        "BlocklistAIStatusBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let agent_view_controller = self.agent_view_controller.as_ref(app);
        if let Some(cloud_mode_setup_terminal_message) =
            self.render_cloud_mode_setup_terminal_message(app)
        {
            return render_standard_message_bar(cloud_mode_setup_terminal_message, None, app);
        }
        let status_element =
            if let Some(cloud_mode_setup_status) = self.render_cloud_mode_setup_status(app) {
                cloud_mode_setup_status
            } else if FeatureFlag::CloudModeSetupV2.is_enabled()
                && self
                    .ambient_agent_view_model
                    .as_ref()
                    .is_some_and(|ambient_agent_view_model| {
                        let terminal_model = self.terminal_model.lock();
                        is_cloud_agent_pre_first_exchange(
                            Some(ambient_agent_view_model),
                            &self.agent_view_controller,
                            &terminal_model,
                            app,
                        )
                    })
            {
                render_warping_indicator_base(
                    WarpingIndicatorProps {
                        icon: None,
                        warping_indicator_text: MaybeShimmeringText::Shimmering {
                            text: "Setting up environment".into(),
                            shimmering_text_handle: self.shimmering_text_handle.clone(),
                        },
                        non_shimmering_text: None,
                        non_shimmering_suffix: None,
                        buttons: None,
                        is_passive_code_diff: false,
                        secondary_element: self.render_tip(app),
                    },
                    app,
                )
            } else if self
                .terminal_model
                .lock()
                .block_list()
                .active_block()
                .is_agent_tagged_in()
                && self
                    .ephemeral_message_model
                    .as_ref(app)
                    .current_message()
                    .is_none()
            {
                render_warping_indicator_base(
                    WarpingIndicatorProps {
                        icon: Some(icons::gray_clock_icon(appearance).finish()),
                        warping_indicator_text: MaybeShimmeringText::Static(
                            WAITING_FOR_USER_INPUT_MESSAGE.into(),
                        ),
                        non_shimmering_text: None,
                        non_shimmering_suffix: None,
                        buttons: Some(render_switch_control_to_user_button(
                            "Exit",
                            "Exit agent input",
                            ButtonProps {
                                button_handle: &self.state_handles.take_over_button,
                                keystroke: self.set_terminal_input_keystroke.as_ref(),
                                is_active: false,
                            },
                            appearance,
                        )),
                        is_passive_code_diff: false,
                        secondary_element: self.render_tip(app),
                    },
                    app,
                )
            } else if let (Some(warping_indicator), true) = (
                self.render_warping_indicator_for_latest_exchange(app),
                self.ephemeral_message_model
                    .as_ref(app)
                    .current_message()
                    .is_none(),
            ) {
                warping_indicator
            } else if self.ambient_agent_view_model.as_ref().is_some_and(
                |ambient_agent_view_model| {
                    ambient_agent_view_model
                        .as_ref(app)
                        .is_waiting_for_session()
                },
            ) {
                // Don't render warping indicator - the loading screen is shown in the main view
                return Empty::new().finish();
            } else if agent_view_controller.is_active() {
                // The new orchestration pill bar in the agent view header
                // replaces the legacy child-agent status card rows; when
                // it's enabled, render only the message bar here.
                let mut column = Flex::column();
                if !FeatureFlag::OrchestrationPillBar.is_enabled() {
                    column =
                        column.with_child(ChildView::new(&self.child_agent_status_card).finish());
                }
                column = column.with_child(ChildView::new(&self.agent_message_bar).finish());
                return column.finish();
            } else {
                return Empty::new().finish();
            };

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let background = if agent_view_controller.is_inline() {
            agent_view_bg_fill(app)
        } else if InputSettings::as_ref(app).is_universal_developer_input_enabled(app)
            || FeatureFlag::AgentView.is_enabled()
        {
            // Use a fully transparent background for universal developer input (or unconditionally, if the new
            // modality is enabled)
            Fill::Solid(ColorU::transparent_black())
        } else {
            theme.ai_blocks_overlay()
        };

        let mut container = Container::new(status_element).with_background(background);

        let is_passive_code_diff = self
            .active_exchange_model
            .as_ref()
            .is_some_and(|model| model.request_type(app).is_passive_code_diff());
        let is_active_exchange_in_selected_conversation = self
            .active_exchange_model
            .as_ref()
            .and_then(|model| model.conversation_id(app))
            .is_some_and(|id| {
                self.context_model.as_ref(app).selected_conversation_id(app) == Some(id)
            });

        if !FeatureFlag::AgentView.is_enabled()
            && self.input_model.as_ref(app).is_ai_input_enabled()
            && !is_passive_code_diff
            && is_active_exchange_in_selected_conversation
            && !self.terminal_model.lock().is_alt_screen_active()
        {
            container = container
                .with_border(
                    Border::left(LEFT_STRIPE_WIDTH)
                        .with_border_color(ai_brand_color(appearance.theme())),
                )
                // Offset the horizontal layout shift caused by the border.
                .with_padding_left(-LEFT_STRIPE_WIDTH);
        }

        let is_input_pinned_to_top = InputModeSettings::as_ref(app).is_pinned_to_top();
        let is_udi_enabled = InputSettings::as_ref(app).is_universal_developer_input_enabled(app);
        if !FeatureFlag::AgentView.is_enabled() && is_udi_enabled {
            if is_input_pinned_to_top {
                // Use 2px padding on the top, so combined with the 6px padding on the universal
                // input it's an equal 8px on both sides.
                container = container.with_padding_top(2.).with_padding_bottom(8.);
            } else {
                // Use 2px padding on the bottom, so combined with the 6px padding on the universal
                // input it's an equal 8px on both sides.
                container = container.with_padding_top(8.).with_padding_bottom(2.);
            }
        } else {
            container = container.with_vertical_padding(8.);
        }

        // When the agent view is active, keep the child agent status card
        // visible above the warping/status indicator so it doesn't disappear
        // while the agent is working. The new orchestration pill bar
        // replaces this card, so skip it when that flag is on.
        if agent_view_controller.is_active() && !FeatureFlag::OrchestrationPillBar.is_enabled() {
            return Flex::column()
                .with_child(ChildView::new(&self.child_agent_status_card).finish())
                .with_child(container.finish())
                .finish();
        }

        container.finish()
    }
}

#[derive(Debug, Clone)]
pub enum BlocklistAIStatusBarEvent {
    SummarizationCancelDialogToggled { is_open: bool },
    Stop,
}

impl Entity for BlocklistAIStatusBar {
    type Event = BlocklistAIStatusBarEvent;
}

#[derive(Debug, Clone)]
pub enum BlocklistAIStatusBarAction {
    ToggleHideResponses,
    SwitchCommandControlToUser,
    Stop,
    /// Force the agent's pending poll for a long-running command to resolve
    /// immediately with a fresh snapshot, bypassing its agent-set timer. Dispatched
    /// by the inline `Check now` affordance in the warping indicator.
    ForceRefreshAgentView {
        block_id: crate::terminal::model::block::BlockId,
    },
}

impl TypedActionView for BlocklistAIStatusBar {
    type Action = BlocklistAIStatusBarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BlocklistAIStatusBarAction::ToggleHideResponses => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.toggle_hide_responses(ctx);
                });
            }
            BlocklistAIStatusBarAction::SwitchCommandControlToUser => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
                });
            }
            BlocklistAIStatusBarAction::Stop => {
                ctx.emit(BlocklistAIStatusBarEvent::Stop);
            }
            BlocklistAIStatusBarAction::ForceRefreshAgentView { block_id } => {
                let block_id = block_id.clone();
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.request_force_refresh(&block_id, ctx);
                });
            }
        }
    }
}
