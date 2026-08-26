//! Local-to-cloud handoff coordination for a TUI terminal session.
//!
//! [`TuiHandoffModel`] owns handoff state and execution. This module owns the
//! active card's lifetime within the session surface and applies
//! session-specific outcomes such as restoring input or persisting the
//! completed card into the transcript.

use warp::tui_export::{HandoffRestoration, UserWorkspaces, record_static_slash_command_accepted};
use warpui_core::{AppContext, SingletonEntity, ViewContext, ViewHandle};

use super::TuiTerminalSessionView;
use crate::handoff::{
    TuiHandoffBlock, TuiHandoffBlockEvent, TuiHandoffModel, TuiHandoffModelEvent,
};

impl TuiTerminalSessionView {
    pub(super) fn active_handoff(&self, ctx: &AppContext) -> Option<ViewHandle<TuiHandoffBlock>> {
        let handoff = self.handoff.as_ref()?;
        handoff.as_ref(ctx).is_active(ctx).then(|| handoff.clone())
    }

    pub(super) fn start_handoff(&mut self, argument: Option<&String>, ctx: &mut ViewContext<Self>) {
        if self
            .session_state(ctx)
            .is_ok_and(|state| state.has_blocking_interaction())
        {
            return;
        }
        let current_working_directory = self.current_working_directory(ctx);
        // Captured from the source view's window when the handoff started, matching the
        // GUI's `resolve_upload_target`: the window's live team, not an account default.
        let team_uid = UserWorkspaces::as_ref(ctx).team_uid_for_window(ctx.window_id());
        let model = match TuiHandoffModel::new(
            self.terminal_surface_id,
            self.terminal_model.clone(),
            self.ai_controller.clone(),
            self.ai_context_model.clone(),
            current_working_directory,
            argument.cloned(),
            team_uid,
            ctx,
        ) {
            Ok(model) => model,
            Err(failure) => {
                let (replacement_input, message) = failure.into_parts();
                if let Some(input) = replacement_input {
                    self.input_view.update(ctx, |input_view, ctx| {
                        input_view.set_text(&input, ctx);
                    });
                }
                self.show_transient_hint(message, ctx);
                return;
            }
        };

        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        let model_for_view = model.clone();
        let handoff_block =
            ctx.add_typed_action_tui_view(move |ctx| TuiHandoffBlock::new(model_for_view, ctx));
        let handoff_block_for_events = handoff_block.clone();
        ctx.subscribe_to_model(&model, move |view, _, event, ctx| {
            view.handle_handoff_model_event(&handoff_block_for_events, event, ctx);
        });
        ctx.subscribe_to_view(&handoff_block, |_, _, event, ctx| match event {
            TuiHandoffBlockEvent::LayoutInvalidated => ctx.notify(),
        });
        self.handoff = Some(handoff_block);
        self.reconcile_focus(ctx);
        record_static_slash_command_accepted("/handoff", true, ctx);
    }

    fn restore_handoff_input(
        &mut self,
        restoration: &HandoffRestoration,
        ctx: &mut ViewContext<Self>,
    ) {
        self.input_view.update(ctx, |input, ctx| {
            input.set_text(&restoration.prompt, ctx);
        });
        if !restoration.attachments.is_empty() {
            self.ai_context_model.update(ctx, |context, ctx| {
                context.append_pending_attachments(restoration.attachments.clone(), ctx);
            });
        }
    }

    fn clear_handoff_interaction(&mut self) {
        self.handoff = None;
    }

    fn handle_handoff_model_event(
        &mut self,
        handoff_block: &ViewHandle<TuiHandoffBlock>,
        event: &TuiHandoffModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiHandoffModelEvent::Changed { focus_block } => {
                if *focus_block {
                    self.reconcile_focus(ctx);
                }
                return;
            }
            TuiHandoffModelEvent::Cancelled(restoration) => {
                self.clear_handoff_interaction();
                if let Some(restoration) = restoration {
                    self.restore_handoff_input(restoration, ctx);
                }
            }
            TuiHandoffModelEvent::Failed {
                restoration,
                message,
            } => {
                self.clear_handoff_interaction();
                if let Some(restoration) = restoration {
                    self.restore_handoff_input(restoration, ctx);
                }
                self.show_transient_hint(message.clone(), ctx);
            }
            TuiHandoffModelEvent::ContinueLocally => {
                self.clear_handoff_interaction();
                self.transcript.update(ctx, |transcript, ctx| {
                    transcript.attach_handoff(handoff_block.clone(), ctx);
                });
            }
            TuiHandoffModelEvent::StartNewConversation => {
                self.clear_handoff_interaction();
                self.start_new_conversation(None, ctx);
            }
        }
        self.reconcile_focus(ctx);
    }
}
