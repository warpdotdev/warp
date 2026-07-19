//! TUI presentation for an agent monitoring a long-running terminal command.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentActionType, AIAgentInput, AIBlockModel, AIBlockModelImpl, AIConversationId, BlockId,
    BlocklistAIActionModel, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    CLISubagentController, CLISubagentTarget, LongRunningCommandControlState, ShellCommandDelay,
    ShellCommandExecutor, TaskId, TerminalModel, UserTakeOverReason,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiHoverable, TuiLayoutContext,
    TuiParentElement, TuiSize, TuiText,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::r#async::Timer;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext,
};

use crate::tui_builder::TuiUiBuilder;

pub(super) const TAKE_CONTROL_KEY_BINDING: &str = "ctrl-c";
pub(super) const HAND_BACK_KEY_BINDING: &str = "ctrl-g";

fn terminal_use_status_text(
    control_state: &LongRunningCommandControlState,
    command_finished: bool,
    output_streaming: bool,
) -> String {
    if command_finished {
        return "Command finished".to_owned();
    }
    let (status, key_binding, action) = match control_state {
        LongRunningCommandControlState::Agent {
            is_blocked: true, ..
        } => (
            "Agent needs your input",
            TAKE_CONTROL_KEY_BINDING,
            "to take control",
        ),
        LongRunningCommandControlState::Agent { .. } if output_streaming => (
            "Agent is monitoring command",
            TAKE_CONTROL_KEY_BINDING,
            "to take control",
        ),
        LongRunningCommandControlState::Agent { .. } => (
            "Agent waiting for instructions",
            TAKE_CONTROL_KEY_BINDING,
            "to take control",
        ),
        LongRunningCommandControlState::User { reason } => match reason {
            UserTakeOverReason::Manual => {
                ("User is in control", HAND_BACK_KEY_BINDING, "to hand back")
            }
            UserTakeOverReason::Stop { .. } => (
                "Agent paused · user is in control",
                HAND_BACK_KEY_BINDING,
                "to hand back",
            ),
            UserTakeOverReason::TransferFromAgent { .. } => (
                "Agent handed control to you",
                HAND_BACK_KEY_BINDING,
                "to hand back",
            ),
        },
    };
    format!("{status} · {key_binding} {action}")
}

fn resolve_latest_instruction(
    controller_instruction: Option<String>,
    exchange_instruction: Option<String>,
) -> Option<String> {
    controller_instruction.or(exchange_instruction)
}

fn remaining_for_fixed_delay(delay: Duration, elapsed: Duration) -> Option<Duration> {
    let delay = delay.min(ShellCommandExecutor::MAX_AGENT_DELAY_DURATION);
    let remaining = delay.saturating_sub(elapsed);
    (remaining.as_secs() > 0).then_some(remaining)
}

fn format_next_check_remaining(remaining: Duration) -> String {
    let seconds = remaining.as_secs();
    if seconds < 60 {
        format!(" · Check in {seconds}s")
    } else {
        format!(" · Check in {}m", seconds / 60)
    }
}

/// Events emitted to whichever command surface hosts this view.
pub(super) enum TuiCLISubagentViewEvent {
    LayoutChanged,
}

/// User interactions handled by the terminal-use view.
#[derive(Clone, Debug)]
pub(super) enum TuiCLISubagentViewAction {
    Allow,
}

/// Compact terminal-use state rendered alongside one command block.
pub(super) struct TuiCLISubagentView {
    block_id: BlockId,
    task_id: TaskId,
    conversation_id: AIConversationId,
    model: Option<Rc<dyn AIBlockModel<View = Self>>>,
    subagent_controller: ModelHandle<CLISubagentController>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    last_measured_width: Cell<Option<u16>>,
    allow_mouse_state: MouseStateHandle,
}

impl TuiCLISubagentView {
    pub(super) fn new(
        target: CLISubagentTarget,
        subagent_controller: ModelHandle<CLISubagentController>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let model = Self::model_for_task(target.conversation_id, &target.task_id, ctx);
        if let Some(model) = model.as_ref() {
            model.on_updated_output(Box::new(|view, ctx| view.invalidate_layout(ctx)), ctx);
        }

        let mut task_id_for_events = target.task_id.clone();
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |view, _, event, ctx| match event {
                BlocklistAIHistoryEvent::UpgradedTask {
                    optimistic_id,
                    server_id,
                    ..
                } if *optimistic_id == task_id_for_events => {
                    task_id_for_events = server_id.clone();
                    view.task_id = server_id.clone();
                }
                BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    task_id,
                    conversation_id,
                    ..
                } if *task_id == task_id_for_events => {
                    if let Ok(model) = AIBlockModelImpl::<Self>::new(
                        *exchange_id,
                        *conversation_id,
                        false,
                        false,
                        ctx,
                    ) {
                        model.on_updated_output(
                            Box::new(|view, ctx| view.invalidate_layout(ctx)),
                            ctx,
                        );
                        view.model = Some(Rc::new(model));
                        view.conversation_id = *conversation_id;
                        view.invalidate_layout(ctx);
                    }
                }
                _ => {}
            },
        );

        ctx.subscribe_to_model(&subagent_controller, |view, _, event, ctx| {
            if event
                .block_id()
                .is_none_or(|block_id| block_id == &view.block_id)
            {
                view.invalidate_layout(ctx);
            }
        });

        let mut view = Self {
            block_id: target.block_id,
            task_id: target.task_id,
            conversation_id: target.conversation_id,
            model,
            subagent_controller,
            action_model,
            terminal_model,
            last_measured_width: Cell::new(None),
            allow_mouse_state: MouseStateHandle::default(),
        };
        view.start_countdown_refresh(ctx);
        view
    }

    fn model_for_task(
        conversation_id: AIConversationId,
        task_id: &TaskId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Rc<dyn AIBlockModel<View = Self>>> {
        let exchange_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)?
            .get_task(task_id)?
            .last_exchange()?
            .id;
        let model =
            AIBlockModelImpl::<Self>::new(exchange_id, conversation_id, false, false, ctx).ok()?;
        Some(Rc::new(model))
    }

    fn target(&self, app: &AppContext) -> Option<CLISubagentTarget> {
        self.subagent_controller
            .as_ref(app)
            .target_for_block(&self.block_id)
    }

    fn command_finished(&self) -> bool {
        self.terminal_model
            .lock()
            .block_list()
            .block_with_id(&self.block_id)
            .is_none_or(|block| block.finished())
    }

    fn status_text(&self, target: &CLISubagentTarget, app: &AppContext) -> String {
        terminal_use_status_text(
            &target.control_state,
            self.command_finished(),
            self.model
                .as_ref()
                .is_some_and(|model| model.status(app).is_streaming()),
        )
    }

    fn latest_instruction(&self, target: &CLISubagentTarget, app: &AppContext) -> Option<String> {
        let exchange_instruction = self.model.as_ref().and_then(|model| {
            model
                .inputs_to_render(app)
                .iter()
                .rev()
                .find_map(AIAgentInput::display_query)
        });
        resolve_latest_instruction(target.latest_instruction.clone(), exchange_instruction)
    }

    fn next_check_remaining(
        &self,
        target: &CLISubagentTarget,
        app: &AppContext,
    ) -> Option<Duration> {
        let action = self
            .action_model
            .as_ref(app)
            .get_async_running_action(app)?;
        if action.task_id != self.task_id {
            return None;
        }
        let AIAgentActionType::ReadShellCommandOutput {
            delay: Some(ShellCommandDelay::Duration(delay)),
            ..
        } = &action.action
        else {
            return None;
        };
        let last_snapshot_at = target.last_snapshot_at?;
        remaining_for_fixed_delay(*delay, last_snapshot_at.elapsed())
    }

    fn start_countdown_refresh(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(Timer::after(Duration::from_secs(1)), |view, _, ctx| {
            if !view.command_finished() {
                ctx.notify();
                view.start_countdown_refresh(ctx);
            }
        });
    }

    pub(super) fn conversation_id(&self) -> AIConversationId {
        self.conversation_id
    }

    fn render_action(
        label: &'static str,
        mouse_state: &MouseStateHandle,
        action: TuiCLISubagentViewAction,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let style = if mouse_state.lock().unwrap().is_hovered() {
            builder.primary_text_style()
        } else {
            builder.accent_text_style()
        };
        TuiHoverable::new(
            mouse_state.clone(),
            TuiText::new(format!("[{label}]"))
                .with_style(style)
                .truncate()
                .finish(),
        )
        .on_click(move |event_ctx, _| {
            event_ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_content(&self, target: &CLISubagentTarget, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let status = self.status_text(target, app);
        let countdown = self
            .next_check_remaining(target, app)
            .map(format_next_check_remaining)
            .unwrap_or_default();
        let mut content = TuiFlex::column().child(
            TuiText::new(format!("{status}{countdown}"))
                .with_style(builder.muted_text_style())
                .truncate()
                .finish(),
        );
        if let Some(instruction) = self.latest_instruction(target, app) {
            content.add_child(
                TuiText::new(format!("Last instruction: {instruction}"))
                    .with_style(builder.muted_text_style())
                    .truncate()
                    .finish(),
            );
        }
        if target.control_state.is_agent_blocked() {
            content.add_child(
                TuiContainer::new(Self::render_action(
                    "Allow",
                    &self.allow_mouse_state,
                    TuiCLISubagentViewAction::Allow,
                    app,
                ))
                .finish(),
            );
        }
        content.finish()
    }

    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiCLISubagentViewEvent::LayoutChanged);
        ctx.notify();
    }

    pub(super) fn needs_height_measurement(&self, width: u16, app: &AppContext) -> bool {
        self.last_measured_width.get() != Some(width)
            || self
                .model
                .as_ref()
                .is_some_and(|model| model.status(app).is_streaming())
    }

    pub(super) fn record_height_measurement(&self, width: u16) {
        self.last_measured_width.set(Some(width));
    }

    pub(super) fn desired_height(
        &self,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> usize {
        let mut element = self.render(app);
        usize::from(
            element
                .layout(
                    TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                    ctx,
                    app,
                )
                .height,
        )
    }
}

impl Entity for TuiCLISubagentView {
    type Event = TuiCLISubagentViewEvent;
}

impl TuiView for TuiCLISubagentView {
    fn ui_name() -> &'static str {
        "TuiCLISubagentView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        Vec::new()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let Some(target) = self.target(app) else {
            return TuiFlex::column().finish();
        };
        self.render_content(&target, app)
    }
}

impl TypedActionView for TuiCLISubagentView {
    type Action = TuiCLISubagentViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiCLISubagentViewAction::Allow => {
                self.action_model.update(ctx, |action_model, ctx| {
                    action_model.execute_next_action_for_user(self.conversation_id, ctx);
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "tui_cli_subagent_view_tests.rs"]
mod tests;
