//! Shell-command completion state presented through the shared TUI inline menu.

use std::ops::Range;

use string_offset::CharOffset;
use warp_completer::completer::{EngineFileType, MatchedSuggestion};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuAccepted, TuiInlineMenuHandle, TuiInlineMenuListState,
    TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, false, false);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiCompletionAcceptance {
    pub(crate) replacement: String,
    pub(crate) replacement_range: Range<usize>,
    pub(crate) append_space: bool,
}

#[derive(Clone, Debug)]
struct TuiCompletionRow {
    display: String,
    description: Option<String>,
    acceptance: TuiCompletionAcceptance,
}

#[derive(Clone, Debug, Default)]
enum TuiCompletionMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiCompletionRow>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TuiCompletionMenuEvent;

pub(crate) struct TuiCompletionMenuModel {
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    state: TuiCompletionMenuState,
}

impl TuiCompletionMenuModel {
    pub(crate) fn new(suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>) -> Self {
        Self {
            suggestions_mode,
            state: TuiCompletionMenuState::Closed,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        rows: Vec<(String, TuiCompletionAcceptance)>,
        selected_index: usize,
    ) -> Self {
        let mut list = TuiInlineMenuListState::default();
        list.replace_rows(
            rows.into_iter()
                .map(|(display, acceptance)| TuiCompletionRow {
                    display,
                    description: None,
                    acceptance,
                })
                .collect(),
            false,
            Some(selected_index),
            MAX_VISIBLE_ROWS,
            |_| true,
        );
        Self {
            suggestions_mode,
            state: TuiCompletionMenuState::Open { list },
        }
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        matches!(self.state, TuiCompletionMenuState::Open { .. })
            && self.suggestions_mode.as_ref(ctx).mode()
                == TuiInputSuggestionsMode::CompletionSuggestions
    }

    pub(crate) fn show(
        &mut self,
        suggestions: Vec<MatchedSuggestion>,
        replacement_range: Range<usize>,
        append_space_at_buffer_end: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::CompletionSuggestions, ctx)
        });
        if !did_open {
            return;
        }

        let rows = suggestions
            .into_iter()
            .map(|suggestion| {
                let append_space = append_space_at_buffer_end
                    && suggestion.suggestion.file_type != Some(EngineFileType::Directory);
                TuiCompletionRow {
                    display: suggestion.display().to_owned(),
                    description: suggestion.description(),
                    acceptance: TuiCompletionAcceptance {
                        replacement: suggestion.replacement().to_owned(),
                        replacement_range: replacement_range.clone(),
                        append_space,
                    },
                }
            })
            .collect();
        let mut list = TuiInlineMenuListState::default();
        list.replace_rows(rows, false, Some(0), MAX_VISIBLE_ROWS, |_| true);
        self.state = TuiCompletionMenuState::Open { list };
        ctx.emit(TuiCompletionMenuEvent);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open(ctx) {
            self.close(ctx);
        }
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiCompletionMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiCompletionMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiCompletionMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiCompletionMenuEvent);
    }

    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiCompletionAcceptance> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiCompletionMenuState::Open { list } = &self.state else {
            return None;
        };
        let acceptance = list.selected_row()?.acceptance.clone();
        self.close(ctx);
        Some(acceptance)
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiCompletionMenuState::Open { list } = &self.state else {
            return None;
        };
        Some(TuiInlineMenuSnapshot {
            header: None,
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.display.clone(),
                    description: row.description.clone(),
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::InlineMenuItem,
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status: None,
        })
    }

    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        self.state = TuiCompletionMenuState::Closed;
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::CompletionSuggestions, ctx);
        });
        ctx.emit(TuiCompletionMenuEvent);
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiCompletionMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::CompletionSuggestions
    }

    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _ctx: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.update(ctx, |model, ctx| model.accept_selected(ctx))
            .map(TuiInlineMenuAccepted::Completion)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }
}
impl Entity for TuiCompletionMenuModel {
    type Event = TuiCompletionMenuEvent;
}

#[cfg(test)]
#[path = "completion_menu_tests.rs"]
mod tests;
