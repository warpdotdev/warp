//! Authenticated terminal-session TUI surface.
use std::borrow::Cow;
use std::sync::Arc;

use instant::Instant;
use parking_lot::FairMutex;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::{
    AIAgentPtyWriteMode, ActiveSession, ActiveSessionEvent, AgentInteractionMetadata,
    AgentViewEntryOrigin, Appearance, BlocklistAIActionModel, BlocklistAIContextModel,
    BlocklistAIController, BlocklistAIHistoryModel, BlocklistAIInputModel, CancellationReason,
    CommandExecutionSource, ConversationSelection, ConversationSelectionHandle,
    ExecuteCommandEvent, GetRelevantFilesController, LLMPreferences, LLMPreferencesEvent,
    ModelEvent, PtyIntent, PtyIntentEvent, ShellCommandExecutorEvent, TerminalModel,
    TerminalSurface, TerminalSurfaceInit,
};
use warp_core::ui::theme::Fill as ThemeFill;
use warp_editor::model::CoreEditorModel;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Color, Modifier, TuiChildView, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiStyle,
    TuiText,
};
use warpui_core::elements::Fill as CoreFill;
use warpui_core::keymap::macros::*;
use warpui_core::keymap::FixedBinding;
use warpui_core::platform::TerminationMode;
use warpui_core::r#async::Timer;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::conversation_selection::TuiConversationSelection;
use crate::exit_confirmation::{ExitConfirmation, CTRL_C_EXIT_WINDOW};
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::transcript_view::TuiTranscriptView;
use crate::ui::abbreviate_home_prefix;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const MAX_INPUT_TEXT_ROWS: u16 = 6;

/// The footer hint shown while the ctrl-c exit confirmation is armed.
const CTRL_C_EXIT_HINT: &str = "ctrl-c again to exit";

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

/// Typed actions handled by [`TuiTerminalSessionView`].
#[derive(Debug, Clone)]
pub(crate) enum TuiTerminalSessionAction {
    /// Ctrl-c anywhere in the session surface: cancel the running
    /// conversation, else clear the input; a second press within
    /// [`CTRL_C_EXIT_WINDOW`] exits the TUI.
    Interrupt,
}

/// The authenticated terminal/session surface rendered inside [`RootTuiView`].
pub(crate) struct TuiTerminalSessionView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    conversation_selection: ConversationSelectionHandle,
    ai_controller: ModelHandle<BlocklistAIController>,
    /// Read by the footer for the active session's working directory.
    active_session: ModelHandle<ActiveSession>,
    /// This view's surface id, used to resolve the active model for the footer
    /// the same way the request path does.
    terminal_surface_id: EntityId,
    /// Armed by a ctrl-c press; a second press while armed exits the TUI.
    /// The footer shows [`CTRL_C_EXIT_HINT`] while armed.
    exit_confirmation: ExitConfirmation,
}

/// Registers the session surface's keybindings. Called once at TUI startup
/// from `keybindings::init`. Ctrl-c is a fixed (non-remappable) binding,
/// mirroring peer agent CLIs that treat it as reserved.
pub(crate) fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        TuiTerminalSessionAction::Interrupt,
        id!(TuiTerminalSessionView::ui_name()),
    )
    .with_group(TUI_BINDING_GROUP)]);
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
                active_session.clone(),
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
        // Typing after a ctrl-c press disarms the pending exit confirmation.
        // The ctrl-c buffer clear leaves the buffer empty, so the window it
        // arms survives its own clear.
        let editor_for_exit_disarm = input_editor_model.clone();
        ctx.subscribe_to_model(&input_editor_model, move |view, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                return;
            }
            let is_empty = editor_for_exit_disarm
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .is_empty();
            if !is_empty && view.exit_confirmation.disarm() {
                ctx.notify();
            }
        });
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
        // The footer shows the active model and working directory: re-render
        // when the TUI model setting changes (e.g. settings-file hot reload),
        // when model display names arrive from the server post-login, or when
        // the session's working directory changes.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if let AISettingsChangedEvent::TuiAgentModel { .. } = event {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |_, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&active_session, |_, _, event, ctx| match event {
            ActiveSessionEvent::UpdatedPwd => ctx.notify(),
            ActiveSessionEvent::Bootstrapped => {}
        });

        ctx.spawn_stream_local(wakeups_rx, |_, _, ctx| ctx.notify(), |_, _| {});
        // Focus the input view so the keymap responder chain is
        // [root, session, input]: input bindings win for keys they define,
        // and unbound keys (ctrl-c) fall through to the session/root bindings.
        ctx.focus(&input_view);

        Self {
            transcript,
            input_view,
            conversation_selection,
            ai_controller,
            active_session,
            terminal_surface_id,
            exit_confirmation: ExitConfirmation::default(),
        }
    }

    /// Handles a ctrl-c press: a second press within [`CTRL_C_EXIT_WINDOW`]
    /// exits the TUI; otherwise one contextual action runs — cancel the running
    /// conversation if there is one, else clear the input — and the exit
    /// confirmation is (re-)armed, surfacing [`CTRL_C_EXIT_HINT`] in the footer.
    fn handle_interrupt(&mut self, ctx: &mut ViewContext<Self>) {
        let now = Instant::now();
        if self.exit_confirmation.should_exit(now) {
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
            return;
        }

        if !self.cancel_active_conversation(ctx) {
            self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        }

        // Arm (or re-arm) the confirmation, and disarm + repaint when the
        // window lapses. A re-arm supersedes this (now stale) timer, making
        // its `disarm_expired` a no-op rather than clearing the newer window.
        let window_expires_at = self.exit_confirmation.arm(now);
        ctx.spawn(Timer::after(CTRL_C_EXIT_WINDOW), move |view, _, ctx| {
            if view.exit_confirmation.disarm_expired(window_expires_at) {
                ctx.notify();
            }
        });
        ctx.notify();
    }

    /// Cancels the surface's running conversation (in-flight stream or pending
    /// tool actions), returning whether there was one to cancel.
    fn cancel_active_conversation(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let terminal_surface_id = ctx.view_id();
        self.ai_controller.update(ctx, |controller, ctx| {
            let conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(terminal_surface_id)
                // A brand-new conversation reports `InProgress` before any
                // exchange exists; there is nothing to cancel yet.
                .filter(|conversation| !conversation.is_empty())
                .filter(|conversation| {
                    let status = conversation.status();
                    status.is_in_progress() || status.is_blocked()
                })
                .map(|conversation| conversation.id());
            let Some(conversation_id) = conversation_id else {
                return false;
            };
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
            true
        })
    }

    /// Builds the status footer under the input box. The left slot shows the
    /// ctrl-c exit-confirmation hint while armed (contextual key hints will
    /// live there later); the active model and working directory are pushed to
    /// the right edge behind a flex spacer. The caller must cap the row's
    /// height (a row fills the height it is offered), e.g. with a one-row
    /// [`TuiConstrainedBox`].
    fn render_footer(&self, ctx: &AppContext) -> TuiFlex {
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);
        let mut footer = TuiFlex::row();
        // Left slot: the ctrl-c exit-confirmation hint.
        if self.exit_confirmation.is_armed() {
            footer = footer.child(
                TuiText::new(CTRL_C_EXIT_HINT)
                    .with_style(dim)
                    .truncate()
                    .finish(),
            );
        }
        let model_name = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(self.terminal_surface_id))
            .display_name
            .clone();
        footer = footer
            .flex_child(TuiFlex::row().finish())
            .child(TuiText::new(model_name).truncate().finish());
        // The session's cwd only arrives once shell metadata flows (warpified
        // sessions); until then fall back to the process cwd the TUI's shell
        // was spawned with.
        let cwd = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| cwd.to_string_lossy().into_owned())
            });
        if let Some(cwd) = cwd {
            footer = footer.child(
                TuiText::new(format!(" {}", abbreviate_home_prefix(&cwd)))
                    .with_style(dim)
                    .truncate()
                    .finish(),
            );
        }
        footer
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
            // TODO(tui-agent-cancel): wire `CancelExecution` into the terminal
            // manager so an agent-requested command can be interrupted.
            // Ctrl-c conversation cancellation itself is handled by
            // `handle_interrupt`.
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

        // Ctrl-c (cancel/clear/exit) is handled by the keymap pass via the
        // fixed binding registered in [`Self::init`], so no element-level key
        // handling is needed here.
        TuiContainer::new(
            TuiFlex::column()
                .flex_child(TuiChildView::new(&self.transcript).finish())
                .child(input_box.finish())
                .child(
                    TuiConstrainedBox::new(self.render_footer(ctx))
                        .with_max_rows(1)
                        .finish(),
                ),
        )
        .with_padding(2)
        .finish()
    }
}

impl TypedActionView for TuiTerminalSessionView {
    type Action = TuiTerminalSessionAction;

    fn handle_action(&mut self, action: &TuiTerminalSessionAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiTerminalSessionAction::Interrupt => self.handle_interrupt(ctx),
        }
    }
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
