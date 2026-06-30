//! Authenticated terminal-session TUI surface.

use warp::editor::CodeEditorModel;
use warp::tui_export::{
    ActiveSession, AfterBlockCompletedEvent, BlockIndex, BlocklistAIActionModel,
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIInputModel, ConversationSelection,
    GetRelevantFilesController, PtyIntent, PtyIntentEvent, ShellLaunchData, TerminalSurface,
    TerminalSurfaceInit,
};
use warpui_core::elements::tui::{
    TuiBuffer, TuiChildView, TuiColumn, TuiConstrainedBox, TuiConstraint, TuiContainer, TuiElement,
    TuiEvent, TuiEventContext, TuiLayoutContext, TuiPresentationContext, TuiRect, TuiSize,
};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::conversation_model::TuiConversationModel;
use crate::conversation_selection::TuiConversationSelection;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::transcript_view::TuiTranscriptView;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const MAX_INPUT_TEXT_ROWS: u16 = 6;
const BORDER_ROWS: u16 = 2;
const INPUT_HORIZONTAL_INSET: u16 = 2;

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
    conversation_model: ModelHandle<TuiConversationModel>,
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
        let input_model = ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_INPUT_WIDTH, ctx));
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

    fn render_session(&self) -> Box<dyn TuiElement> {
        let input_box = TuiConstrainedBox::new(
            TuiContainer::new(TuiChildView::new(&self.input_view)).with_border(),
        )
        .with_max_rows(MAX_INPUT_TEXT_ROWS + BORDER_ROWS);
        Box::new(
            TuiColumn::new()
                .flex_child(TuiChildView::new(&self.transcript))
                .child(TuiHorizontalInset::new(input_box, INPUT_HORIZONTAL_INSET)),
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

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        self.render_session()
    }
}

impl TypedActionView for TuiTerminalSessionView {
    type Action = ();
}

impl TerminalSurface for TuiTerminalSessionView {
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

/// Adds exterior horizontal margin while preserving child layout, cursor, and events.
struct TuiHorizontalInset {
    child: Box<dyn TuiElement>,
    inset: u16,
}

impl TuiHorizontalInset {
    fn new(child: impl TuiElement + 'static, inset: u16) -> Self {
        Self {
            child: Box::new(child),
            inset,
        }
    }

    fn inner_rect(&self, area: TuiRect) -> TuiRect {
        let inset = self.inset.min(area.width / 2);
        TuiRect::new(
            area.x.saturating_add(inset),
            area.y,
            area.width.saturating_sub(inset.saturating_mul(2)),
            area.height,
        )
    }

    fn inner_constraint(&self, constraint: TuiConstraint) -> TuiConstraint {
        let inset = self.inset.saturating_mul(2);
        let max_width = constraint.max.width.saturating_sub(inset);
        let min_width = constraint.min.width.saturating_sub(inset).min(max_width);
        TuiConstraint::new(
            TuiSize::new(min_width, constraint.min.height),
            TuiSize::new(max_width, constraint.max.height),
        )
    }
}

impl TuiElement for TuiHorizontalInset {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let child_size = self
            .child
            .layout(self.inner_constraint(constraint), ctx, app);
        constraint.clamp(TuiSize::new(
            child_size
                .width
                .saturating_add(self.inset.saturating_mul(2)),
            child_size.height,
        ))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        self.child.render(self.inner_rect(area), buffer, ctx);
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn cursor_position(&self, area: TuiRect, ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        self.child
            .cursor_position(self.inner_rect(area), ctx)
            .map(|(x, y)| (x.saturating_add(self.inset.min(area.width / 2)), y))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        self.child
            .dispatch_event(event, self.inner_rect(area), event_ctx, ctx, app)
    }
}
