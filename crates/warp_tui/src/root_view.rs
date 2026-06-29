//! [`RootTuiView`]: the root view of the `warp-tui` front-end.

use warp::editor::CodeEditorModel;
use warp::tui_export::{
    ActiveSession, AfterBlockCompletedEvent, BlockIndex, BlocklistAIActionModel,
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIInputModel, ConversationSelection,
    GetRelevantFilesController, PtyIntent, PtyIntentEvent, ShellLaunchData, TerminalSurface,
    TerminalSurfaceInit,
};
use warp::{TuiLoginModel, TuiLoginPhase};
use warpui_core::elements::tui::{
    TuiChildView, TuiColumn, TuiElement, TuiParentElement, TuiText,
};
use warpui_core::{
    keymap, AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TuiView, TypedActionView,
    ViewContext, ViewHandle,
};

use crate::conversation_model::TuiConversationModel;
use crate::conversation_selection::TuiConversationSelection;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::transcript_view::TuiTranscriptView;
use crate::ui::{login_failed, login_placeholder};

/// Char-cell width for the TUI prompt editor. Matches the surface's default
/// columns; the editor wraps at this width independent of live terminal resize.
const INPUT_WIDTH: u16 = 120;

/// This surface emits no PTY intents; commands are driven only by the shell
/// the terminal manager spawns, not by the transcript UI.
pub enum RootEvent {}

impl PtyIntentEvent for RootEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        match *self {}
    }
}

pub struct RootTuiView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    conversation_model: ModelHandle<TuiConversationModel>,
}

impl RootTuiView {
    /// Builds the transcript-capable root surface for a manager-backed terminal session.
    pub fn new(surface_init: TerminalSurfaceInit, ctx: &mut ViewContext<Self>) -> Self {
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
        let conversation_model = ctx.add_model(|ctx| {
            TuiConversationModel::new(
                terminal_surface_id,
                conversation_selection,
                ai_controller,
                ctx,
            )
        });
        let transcript = ctx.add_typed_action_tui_view(|ctx| {
            TuiTranscriptView::new(terminal_surface_id, model.clone(), ctx)
        });
        let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(INPUT_WIDTH, ctx));
        let input_view =
            ctx.add_typed_action_tui_view(move |ctx| TuiInputView::new(input_model, ctx));
        ctx.subscribe_to_view(&input_view, Self::handle_submitted_prompt);

        ctx.subscribe_to_model(&model_events, |_, _, _, ctx| ctx.notify());
        ctx.spawn_stream_local(wakeups_rx, |_, _, ctx| ctx.notify(), |_, _| {});
        ctx.focus_self();

        Self {
            transcript,
            input_view,
            conversation_model,
        }
    }

    /// Routes submitted prompts to the surface's conversation.
    fn handle_submitted_prompt(
        &mut self,
        _input_view: ViewHandle<TuiInputView>,
        event: &TuiInputViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiInputViewEvent::Submitted(prompt) => {
                let prompt = prompt.trim().to_owned();
                if prompt.is_empty() {
                    return;
                }
                self.conversation_model
                    .update(ctx, |model, ctx| model.send_prompt(prompt, ctx));
                ctx.notify();
            }
        }
    }

    fn render_transcript(&self) -> Box<dyn TuiElement> {
        Box::new(
            TuiColumn::new()
                .with_child(Box::new(TuiChildView::new(&self.input_view)))
                .with_child(Box::new(
                    TuiText::new("──── transcript · wheel to scroll · Ctrl-C quits ────")
                        .truncate(),
                ))
                .with_child(Box::new(TuiChildView::new(&self.transcript))),
        )
    }
}

impl Entity for RootTuiView {
    type Event = RootEvent;
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn child_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => vec![self.input_view.id(), self.transcript.id()],
            TuiLoginPhase::AwaitingLogin { .. } | TuiLoginPhase::Failed { .. } => Vec::new(),
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => self.render_transcript(),
            TuiLoginPhase::AwaitingLogin {
                verification_uri,
                user_code,
            } => login_placeholder(verification_uri.as_deref(), user_code.as_deref()),
            TuiLoginPhase::Failed { message } => login_failed(message.as_str()),
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}

impl TerminalSurface for RootTuiView {
    #[cfg(unix)]
    fn should_start_password_prompt_polling(&self, _command: &str, _ctx: &AppContext) -> bool {
        false
    }

    #[cfg(unix)]
    fn should_stop_password_prompt_polling(&self, _completed: &AfterBlockCompletedEvent) -> bool {
        false
    }

    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn on_active_shell_launch_data_updated(
        &mut self,
        _shell_launch_data: Option<ShellLaunchData>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        log::error!("TUI PTY spawn failed: {error:#}");
        ctx.notify();
    }

    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        _block_index: Option<BlockIndex>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(unix)]
    fn on_polled_block_completed(
        &mut self,
        _completed: &AfterBlockCompletedEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}
