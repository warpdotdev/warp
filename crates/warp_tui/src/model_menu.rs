//! Searchable TUI model picker state.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{query_model_picker_choices, LLMId, LLMPreferences, LLMPreferencesEvent};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    result_row_capacity, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, MAX_INLINE_MENU_ROWS,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone)]
struct TuiModelMenuRow {
    id: LLMId,
    title: String,
    is_selectable: bool,
}

#[derive(Debug, Clone, Default)]
enum TuiModelMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiModelMenuRow>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiModelMenuEvent;

pub(crate) struct TuiModelMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    state: TuiModelMenuState,
}

impl TuiModelMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |model, _, event, ctx| {
            if model.is_open(ctx)
                && matches!(
                    event,
                    LLMPreferencesEvent::UpdatedAvailableLLMs
                        | LLMPreferencesEvent::UpdatedActiveAgentModeLLM
                )
            {
                model.refresh_rows(ctx);
            }
        });
        Self {
            input_editor,
            suggestions_mode,
            state: TuiModelMenuState::Closed,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        rows: Vec<(LLMId, bool)>,
        selected_index: usize,
    ) -> Self {
        let mut list = TuiInlineMenuListState::default();
        list.replace_rows(
            rows.into_iter()
                .map(|(id, is_selectable)| TuiModelMenuRow {
                    title: id.to_string(),
                    id,
                    is_selectable,
                })
                .collect(),
            false,
            Some(selected_index),
            MAX_VISIBLE_ROWS,
            |row| row.is_selectable,
        );
        Self {
            input_editor,
            suggestions_mode,
            state: TuiModelMenuState::Open { list },
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiModelMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::ModelSelector
    }

    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::ModelSelector, ctx)
        });
        if !did_open {
            return;
        }
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        self.state = TuiModelMenuState::Open {
            list: TuiInlineMenuListState::default(),
        };
        self.refresh_rows(ctx);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        self.state = TuiModelMenuState::Closed;
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::ModelSelector, ctx);
        });
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.emit(TuiModelMenuEvent);
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |row| row.is_selectable);
        ctx.emit(TuiModelMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |row| row.is_selectable);
        ctx.emit(TuiModelMenuEvent);
    }

    pub(crate) fn accept_selected(&self, ctx: &AppContext) -> Option<LLMId> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiModelMenuState::Open { list } = &self.state else {
            return None;
        };
        list.selected_row().map(|row| row.id.clone())
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiModelMenuState::Open { list } = &self.state else {
            return None;
        };
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Models".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.title.clone(),
                    description: (!row.is_selectable).then(|| "disabled".to_owned()),
                    is_selectable: row.is_selectable,
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status: list
                .rows()
                .is_empty()
                .then(|| TuiInlineMenuStatus::Empty("No models found".to_owned())),
        })
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let query = input_text(&self.input_editor, ctx);
        let preferences = LLMPreferences::as_ref(ctx);
        let active_id = preferences.get_active_base_model(ctx, None).id.clone();
        let choices = query_model_picker_choices(
            preferences,
            preferences.get_base_llm_choices_for_agent_mode(ctx),
            &query,
            ctx,
        );
        let rows = choices
            .into_iter()
            .map(|choice| {
                let is_selectable = choice.is_selectable();
                TuiModelMenuRow {
                    id: choice.llm.id,
                    title: choice.llm.display_name,
                    is_selectable,
                }
            })
            .collect::<Vec<_>>();
        let preferred_index = preferred_selection_index(&rows, &active_id, query.trim().is_empty());
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |row| {
            row.is_selectable
        });
        ctx.emit(TuiModelMenuEvent);
    }
}

fn preferred_selection_index(
    rows: &[TuiModelMenuRow],
    active_id: &LLMId,
    query_is_empty: bool,
) -> Option<usize> {
    if query_is_empty {
        rows.iter()
            .position(|row| row.id == *active_id && row.is_selectable)
            .or_else(|| rows.iter().rposition(|row| row.is_selectable))
    } else {
        rows.iter().rposition(|row| row.is_selectable)
    }
}

fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

impl Entity for TuiModelMenuModel {
    type Event = TuiModelMenuEvent;
}

#[cfg(test)]
#[path = "model_menu_tests.rs"]
mod tests;
