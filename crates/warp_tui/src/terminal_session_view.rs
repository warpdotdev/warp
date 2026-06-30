//! Authenticated terminal-session TUI surface.

use warp::editor::CodeEditorModel;
use warp::tui_export::{
    ActiveSession, AgentViewEntryOrigin, Appearance, BlocklistAIActionModel,
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIInputModel, ConversationSelection,
    ConversationSelectionHandle, GetRelevantFilesController, PtyIntent, PtyIntentEvent,
    TerminalSurface, TerminalSurfaceInit,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    Color, TuiChildView, TuiColumn, TuiConstrainedBox, TuiContainer, TuiElement, TuiStyle,
};
use warpui_core::elements::Fill as GuiFill;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::conversation_selection::TuiConversationSelection;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::transcript_view::TuiTranscriptView;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const MAX_INPUT_TEXT_ROWS: u16 = 6;
const BORDER_ROWS: u16 = 2;
const SESSION_PADDING: u16 = 2;

/// This surface emits no PTY intents; commands are driven only by the spawned shell.
pub(crate) enum TuiTerminalSessionEvent {}

impl PtyIntentEvent for TuiTerminalSessionEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        match *self {}
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
        let input_model = ctx.add_model(|ctx| {
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
                input_model,
                context_model,
                conversation_selection.clone(),
                action_model,
                active_session,
                model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let transcript = ctx.add_typed_action_tui_view(|ctx| {
            TuiTranscriptView::new(terminal_surface_id, model.clone(), ctx)
        });
        let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_INPUT_WIDTH, ctx));
        let input_view =
            ctx.add_typed_action_tui_view(move |ctx| TuiInputView::new(input_model, ctx));
        ctx.subscribe_to_view(&input_view, |view, _, event, ctx| {
            view.handle_submitted_prompt(event, ctx);
        });

        ctx.subscribe_to_model(&model_events, |_, _, _, ctx| ctx.notify());
        ctx.spawn_stream_local(wakeups_rx, |_, _, ctx| ctx.notify(), |_, _| {});
        ctx.focus_self();

        Self {
            transcript,
            input_view,
            conversation_selection,
            ai_controller,
        }
    }

    /// Routes submitted prompts to the surface's conversation.
    fn handle_submitted_prompt(&mut self, event: &TuiInputViewEvent, ctx: &mut ViewContext<Self>) {
        match event {
            TuiInputViewEvent::Submitted(prompt) => {
                let prompt = prompt.trim().to_owned();
                if prompt.is_empty() {
                    return;
                }
                self.send_prompt(prompt, ctx);
                ctx.notify();
            }
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
                selection.try_start_new_conversation(AgentViewEntryOrigin::Cli, ctx)
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

    fn render_session(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let theme = Appearance::as_ref(app).theme();
        let border_color: Color = GuiFill::from(theme.tui_transcript_accent_color()).into();
        let background: Color = GuiFill::from(theme.tui_transcript_background()).into();
        let input_box = TuiConstrainedBox::new(
            TuiContainer::new(TuiChildView::new(&self.input_view))
                .with_border_style(TuiStyle::default().fg(border_color)),
        )
        .with_max_rows(MAX_INPUT_TEXT_ROWS + BORDER_ROWS);
        Box::new(
            TuiContainer::new(
                TuiColumn::new()
                    .flex_child(TuiChildView::new(&self.transcript))
                    .child(input_box),
            )
            .with_background(background)
            .with_padding(SESSION_PADDING),
        )
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
        self.render_session(ctx)
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
