//! Authenticated terminal-session TUI surface.
use std::borrow::Cow;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::editor::CodeEditorModel;
use warp::tui_export::{
    AIAgentPtyWriteMode, ActiveSession, AgentInteractionMetadata, AgentViewEntryOrigin, Appearance,
    BlocklistAIActionModel, BlocklistAIContextModel, BlocklistAIController,
    BlocklistAIHistoryModel, BlocklistAIInputModel, CommandExecutionSource, ConversationSelection,
    ConversationSelectionHandle, ExecuteCommandEvent, GetRelevantFilesController, ModelEvent,
    PtyIntent, PtyIntentEvent, ShellCommandExecutorEvent, TerminalModel, TerminalSurface,
    TerminalSurfaceInit,
};
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Color, TuiChildView, TuiColumn, TuiConstrainedBox, TuiContainer, TuiElement, TuiStyle,
};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::conversation_selection::TuiConversationSelection;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::transcript_view::TuiTranscriptView;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const MAX_INPUT_TEXT_ROWS: u16 = 6;

/// Events emitted by the TUI terminal session surface.
pub(crate) enum TuiTerminalSessionEvent {
    ExecuteCommand(Box<ExecuteCommandEvent>),
    WriteAgentInput {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
}

impl PtyIntentEvent for TuiTerminalSessionEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        match self {
            Self::ExecuteCommand(event) => Some(PtyIntent::ExecuteCommand((**event).clone())),
            Self::WriteAgentInput { bytes, mode } => Some(PtyIntent::WriteAgentInput {
                bytes: bytes.clone(),
                mode: *mode,
            }),
        }
    }
}

/// The authenticated terminal/session surface rendered inside [`RootTuiView`].
pub(crate) struct TuiTerminalSessionView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    conversation_selection: ConversationSelectionHandle,
    ai_controller: ModelHandle<BlocklistAIController>,
}

impl TuiTerminalSessionView {
    /// Builds the transcript-capable terminal surface for a manager-backed session.
    pub(crate) fn new(surface_init: TerminalSurfaceInit, ctx: &mut ViewContext<Self>) -> Self {
        let TerminalSurfaceInit {
            model,
            sessions,
            model_events,
            wakeups_rx,
            ..
        } = surface_init;

        let terminal_surface_id: EntityId = ctx.view_id();
        let active_session =
            ctx.add_model(|ctx| ActiveSession::new(sessions.clone(), model_events.clone(), ctx));
        let conversation_selection = ctx.add_model(|ctx| {
            Box::new(TuiConversationSelection::new(terminal_surface_id, ctx))
                as Box<dyn ConversationSelection>
        });
        let context_model = ctx.add_model(|ctx| {
            BlocklistAIContextModel::new(
                sessions,
                &model_events,
                model.clone(),
                terminal_surface_id,
                conversation_selection.clone(),
                ctx,
            )
        });
        let ai_input_model = ctx.add_model(|ctx| {
            BlocklistAIInputModel::new(
                model.clone(),
                conversation_selection.clone(),
                context_model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let get_relevant_files_controller = ctx.add_model(GetRelevantFilesController::new);
        let action_model = ctx.add_model(|ctx| {
            BlocklistAIActionModel::new(
                model.clone(),
                active_session.clone(),
                &model_events,
                get_relevant_files_controller,
                terminal_surface_id,
                ctx,
            )
        });
        let ai_controller = ctx.add_model(|ctx| {
            BlocklistAIController::new(
                ai_input_model,
                context_model,
                conversation_selection.clone(),
                action_model.clone(),
                active_session,
                model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let transcript = ctx.add_typed_action_tui_view(|ctx| {
            TuiTranscriptView::new(terminal_surface_id, model.clone(), ctx)
        });
        let input_editor_model =
            ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_INPUT_WIDTH, ctx));
        let input_view =
            ctx.add_typed_action_tui_view(move |ctx| TuiInputView::new(input_editor_model, ctx));
        ctx.subscribe_to_view(&input_view, |view, _, event, ctx| match event {
            TuiInputViewEvent::Submitted(prompt) => {
                let prompt = prompt.trim().to_owned();
                if !prompt.is_empty() {
                    view.send_prompt(prompt, ctx);
                    ctx.notify();
                }
            }
        });

        // Bridge shared shell-tool executor events into terminal-manager PTY intents.
        let shell_command_executor = action_model.as_ref(ctx).shell_command_executor(ctx);
        let model_for_shell_events = model.clone();
        ctx.subscribe_to_model(&shell_command_executor, move |view, _, event, ctx| {
            view.handle_shell_command_executor_event(event, &model_for_shell_events, ctx);
        });

        // These events update block metadata or grids the transcript reads.
        // PTY output redraws are driven by `wakeups_rx` below.
        ctx.subscribe_to_model(&model_events, |_, _, event, ctx| match event {
            ModelEvent::BlockCompleted(_)
            | ModelEvent::AfterBlockStarted { .. }
            | ModelEvent::BlockMetadataReceived(_)
            | ModelEvent::BlockWorkingDirectoryUpdated(_)
            | ModelEvent::BackgroundBlockStarted
            | ModelEvent::TerminalClear
            | ModelEvent::PromptUpdated
            | ModelEvent::Typeahead
            | ModelEvent::Handler(_)
            | ModelEvent::FinishUpdate(_) => ctx.notify(),
            _ => {}
        });
        ctx.spawn_stream_local(wakeups_rx, |_, _, ctx| ctx.notify(), |_, _| {});
        ctx.focus_self();

        Self {
            transcript,
            input_view,
            conversation_selection,
            ai_controller,
        }
    }

    /// Sends a prompt to the selected conversation, creating one if needed.
    fn send_prompt(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        let conversation_id = match self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            Some(conversation_id) => conversation_id,
            None => match self.conversation_selection.update(ctx, |selection, ctx| {
                selection.try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
            }) {
                Ok(conversation_id) => conversation_id,
                Err(error) => {
                    log::error!("Failed to create TUI conversation: {error:#}");
                    return;
                }
            },
        };
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_user_query_in_conversation(prompt, conversation_id, None, ctx);
        });
    }

    /// Bridges shared shell-tool executor events into terminal-manager PTY intents.
    fn handle_shell_command_executor_event(
        &mut self,
        event: &ShellCommandExecutorEvent,
        model: &Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ShellCommandExecutorEvent::ExecuteCommand { action_id, command } => {
                let Some((session_id, conversation_id)) = (|| {
                    let model = model.lock();
                    let session_id = model.block_list().active_block().session_id()?;
                    let conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation_id_for_action(action_id, ctx.view_id())?;
                    Some((session_id, conversation_id))
                })() else {
                    log::warn!(
                        "Unable to execute TUI agent-requested command for action {action_id:?}"
                    );
                    return;
                };

                ctx.emit(TuiTerminalSessionEvent::ExecuteCommand(Box::new(
                    ExecuteCommandEvent {
                        command: command.clone(),
                        session_id,
                        workflow_id: None,
                        workflow_command: None,
                        should_add_command_to_history: true,
                        source: CommandExecutionSource::AI {
                            metadata: AgentInteractionMetadata::new_hidden(
                                action_id.clone(),
                                conversation_id,
                            ),
                        },
                    },
                )));
            }
            ShellCommandExecutorEvent::WriteToPty { input, mode } => {
                ctx.emit(TuiTerminalSessionEvent::WriteAgentInput {
                    bytes: Cow::Owned(input.to_vec()),
                    mode: *mode,
                });
            }
            // TODO(tui-agent-cancel): we need to think about how we want to handle ctrl c.
            // Right now it shuts down the entire app, but we should probably mimic the pattern from claude code, amp, etc.
            // and have one ctrl c shut down any in progress conversation or tool call, and a double ctrl c actually close the app
            // (with some ephemeral message after the first ctrl c).
            ShellCommandExecutorEvent::CancelExecution
            | ShellCommandExecutorEvent::TransferControlToUser { .. } => {}
        }
    }
}

impl Entity for TuiTerminalSessionView {
    type Event = TuiTerminalSessionEvent;
}

impl TuiView for TuiTerminalSessionView {
    fn ui_name() -> &'static str {
        "TuiTerminalSessionView"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![self.transcript.id(), self.input_view.id()]
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(ctx).theme();
        let border_color: Color =
            CoreFill::from(ThemeFill::from(theme.terminal_colors().normal.cyan)).into();
        let input_box = TuiConstrainedBox::new(
            TuiContainer::new(TuiChildView::new(&self.input_view))
                .with_border_style(TuiStyle::default().fg(border_color)),
        )
        .with_max_rows(MAX_INPUT_TEXT_ROWS + 2);

        TuiContainer::new(
            TuiColumn::new()
                .flex_child(TuiChildView::new(&self.transcript))
                .child(input_box),
        )
        .with_padding(2)
        .finish()
    }
}

impl TypedActionView for TuiTerminalSessionView {
    type Action = ();
}

impl TerminalSurface for TuiTerminalSessionView {
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        log::error!("TUI PTY spawn failed: {error:#}");
        ctx.notify();
    }
}
