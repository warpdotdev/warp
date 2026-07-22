//! Authoritative input-suggestions mode for the TUI.
//!
//! This mirrors the GUI's `InputSuggestionsModeModel`: one mode owns input-menu
//! rendering and actions at a time, and replacing a visible mode transitions
//! through `Closed` before opening the next mode.

use warpui_core::{Entity, ModelContext};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInputSuggestionsMode {
    #[default]
    Closed,
    SlashCommands,
    ConversationMenu,
    ModelSelector,
    SkillMenu,
    Mcp,
    PromptHistory,
}

impl TuiInputSuggestionsMode {
    pub(crate) fn is_visible(self) -> bool {
        self != Self::Closed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TuiInputSuggestionsModeEvent {
    pub(crate) mode: TuiInputSuggestionsMode,
}

pub(crate) struct TuiInputSuggestionsModeModel {
    mode: TuiInputSuggestionsMode,
}

impl TuiInputSuggestionsModeModel {
    pub(crate) fn new() -> Self {
        Self {
            mode: TuiInputSuggestionsMode::Closed,
        }
    }

    pub(crate) fn mode(&self) -> TuiInputSuggestionsMode {
        self.mode
    }

    pub(crate) fn set_mode(&mut self, mode: TuiInputSuggestionsMode, ctx: &mut ModelContext<Self>) {
        if self.mode == mode {
            return;
        }

        if self.mode.is_visible() && mode.is_visible() {
            self.mode = TuiInputSuggestionsMode::Closed;
            ctx.emit(TuiInputSuggestionsModeEvent { mode: self.mode });
        }

        self.mode = mode;
        ctx.emit(TuiInputSuggestionsModeEvent { mode });
    }
    pub(crate) fn try_open(
        &mut self,
        mode: TuiInputSuggestionsMode,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        match self.mode {
            TuiInputSuggestionsMode::Closed => {
                self.set_mode(mode, ctx);
                true
            }
            active_mode if active_mode == mode => true,
            TuiInputSuggestionsMode::SlashCommands
            | TuiInputSuggestionsMode::ConversationMenu
            | TuiInputSuggestionsMode::ModelSelector
            | TuiInputSuggestionsMode::SkillMenu
            | TuiInputSuggestionsMode::Mcp
            | TuiInputSuggestionsMode::PromptHistory => false,
        }
    }

    pub(crate) fn close_if_active(
        &mut self,
        mode: TuiInputSuggestionsMode,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.mode == mode {
            self.set_mode(TuiInputSuggestionsMode::Closed, ctx);
        }
    }
}

impl Entity for TuiInputSuggestionsModeModel {
    type Event = TuiInputSuggestionsModeEvent;
}

#[cfg(test)]
#[path = "input_suggestions_mode_tests.rs"]
mod tests;
