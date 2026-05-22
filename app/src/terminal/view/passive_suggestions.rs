use warpui::ViewContext;

use super::TerminalView;
use crate::server::telemetry::InteractionSource;
use crate::terminal::view::CodeDiffAction;

#[derive(Copy, Clone, Debug)]
pub enum PromptSuggestionResolution {
    Accept {
        interaction_source: InteractionSource,
    },
    Reject {
        ctrl_c: bool,
    },
}

impl From<PromptSuggestionResolution> for CodeDiffAction {
    fn from(value: PromptSuggestionResolution) -> Self {
        match value {
            PromptSuggestionResolution::Accept { .. } => CodeDiffAction::Accept,
            PromptSuggestionResolution::Reject { .. } => CodeDiffAction::Reject,
        }
    }
}

impl TerminalView {
    pub(super) fn resolve_passive_suggestion(
        &mut self,
        resolution: PromptSuggestionResolution,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self.resolve_prompt_suggestion_diff(resolution, ctx) {
            return true;
        }
        if self.resolve_unit_test_suggestion(resolution, ctx) {
            return true;
        }
        if self.resolve_prompt_suggestion(resolution, ctx) {
            return true;
        }

        false
    }

    pub(super) fn resolve_prompt_suggestion_diff(
        &mut self,
        action: impl Into<CodeDiffAction>,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(ai_block) = self.last_ai_block() else {
            return false;
        };
        let action = action.into();
        ai_block.update(ctx, |ai_block, ctx| {
            ai_block.handle_passive_code_diff_action(action, ctx)
        })
    }

    fn resolve_unit_test_suggestion(
        &mut self,
        resolution: PromptSuggestionResolution,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(ai_block) = self.last_ai_block() else {
            return false;
        };

        let handled = match resolution {
            PromptSuggestionResolution::Accept { interaction_source } => {
                ai_block.update(ctx, |ai_block, ctx| {
                    ai_block.accept_pending_unit_test_suggestion(interaction_source, ctx)
                })
            }
            PromptSuggestionResolution::Reject { .. } => ai_block.update(ctx, |ai_block, ctx| {
                ai_block.dismiss_pending_suggested_prompt(InteractionSource::Keybinding, ctx)
            }),
        };
        ctx.notify();
        handled
    }
}
