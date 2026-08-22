//! Searchable TUI model picker state.
#[cfg(test)]
use std::collections::HashMap;

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::AISettings;
use warp::tui_export::{
    AISettingsChangedEvent, LLMId, LLMPreferences, LLMPreferencesEvent, TuiModelPickerPresentation,
    tui_active_model_id_for_view, tui_model_picker_catalog_ids,
    tui_model_picker_presentation_for_view,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{
    AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity, WeakViewHandle,
};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::terminal_session_view::TuiTerminalSessionView;

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone)]
struct TuiModelMenuRow {
    id: LLMId,
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
    terminal_view_id: EntityId,
    owner_view: Option<WeakViewHandle<TuiTerminalSessionView>>,
    #[cfg(test)]
    test_presentations: HashMap<LLMId, TuiModelPickerPresentation>,
    state: TuiModelMenuState,
}

impl TuiModelMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        terminal_view_id: EntityId,
        owner_view: WeakViewHandle<TuiTerminalSessionView>,
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
        ctx.subscribe_to_model(&AISettings::handle(ctx), |model, _, event, ctx| {
            if model.is_open(ctx)
                && matches!(event, AISettingsChangedEvent::ExecutionProfiles { .. })
            {
                model.refresh_rows(ctx);
            }
        });
        Self {
            input_editor,
            suggestions_mode,
            terminal_view_id,
            owner_view: Some(owner_view),
            #[cfg(test)]
            test_presentations: HashMap::new(),
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
        let test_presentations: HashMap<LLMId, TuiModelPickerPresentation> = rows
            .iter()
            .map(|(id, is_selectable)| {
                (
                    id.clone(),
                    TuiModelPickerPresentation {
                        id: id.clone(),
                        title: id.to_string(),
                        is_selectable: *is_selectable,
                        is_key_connected: false,
                        is_profile_default: false,
                        discount_percentage: None,
                    },
                )
            })
            .collect();
        list.replace_rows(
            rows.into_iter()
                .map(|(id, _)| TuiModelMenuRow { id })
                .collect(),
            false,
            Some(selected_index),
            MAX_VISIBLE_ROWS,
            |row| {
                test_presentations
                    .get(&row.id)
                    .is_some_and(|presentation| presentation.is_selectable)
            },
        );
        Self {
            input_editor,
            suggestions_mode,
            terminal_view_id: EntityId::new(),
            owner_view: None,
            test_presentations,
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
        let selectable_ids = self.selectable_ids(ctx);
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |row| selectable_ids.contains(&row.id));
        ctx.emit(TuiModelMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let selectable_ids = self.selectable_ids(ctx);
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |row| selectable_ids.contains(&row.id));
        ctx.emit(TuiModelMenuEvent);
    }

    /// Selects the row at absolute snapshot index `index` (for mouse click).
    /// Returns `true` when the row was actually selected, `false` when the
    /// index is out of bounds, the menu is not open, or the row is not
    /// selectable.
    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some((retained_index, presentation)) = self.presentations(ctx).into_iter().nth(index)
        else {
            return false;
        };
        if !presentation.is_selectable {
            return false;
        }
        let selectable_ids = self.selectable_ids(ctx);
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(retained_index, MAX_VISIBLE_ROWS, |row| {
            selectable_ids.contains(&row.id)
        });
        ctx.emit(TuiModelMenuEvent);
        selected
    }

    /// Scrolls the viewport by `delta` rows without changing the selection.
    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiModelMenuEvent);
    }

    pub(crate) fn accept_selected(&self, ctx: &AppContext) -> Option<LLMId> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiModelMenuState::Open { list } = &self.state else {
            return None;
        };
        let row = list.selected_row()?;
        self.presentation(row, ctx)
            .filter(|presentation| presentation.is_selectable)
            .map(|presentation| presentation.id)
    }

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiModelMenuState::Open { list } = &self.state else {
            return None;
        };
        let presentations = self.presentations(ctx);
        let selected_index = list.selected_index().and_then(|selected_index| {
            presentations
                .iter()
                .position(|(retained_index, presentation)| {
                    *retained_index == selected_index && presentation.is_selectable
                })
        });
        let rows = presentations
            .into_iter()
            .map(|(_, presentation)| snapshot_row(&presentation))
            .collect::<Vec<_>>();
        let status = rows
            .is_empty()
            .then(|| TuiInlineMenuStatus::Empty("No models found".to_owned()));
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Models".to_owned()),
                tabs: Vec::new(),
            }),
            selected_index,
            scroll_offset: list
                .scroll_offset()
                .min(rows.len().saturating_sub(MAX_VISIBLE_ROWS)),
            rows,
            scroll_anchor: list.scroll_anchor(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    fn presentation(
        &self,
        row: &TuiModelMenuRow,
        ctx: &AppContext,
    ) -> Option<TuiModelPickerPresentation> {
        #[cfg(test)]
        if self.owner_view.is_none() {
            return self.test_presentations.get(&row.id).cloned();
        }
        tui_model_picker_presentation_for_view(
            self.owner_view.as_ref()?,
            self.terminal_view_id,
            &row.id,
            ctx,
        )
    }

    fn presentations(&self, ctx: &AppContext) -> Vec<(usize, TuiModelPickerPresentation)> {
        let TuiModelMenuState::Open { list } = &self.state else {
            return Vec::new();
        };
        list.rows()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                self.presentation(row, ctx)
                    .map(|presentation| (index, presentation))
            })
            .collect()
    }

    fn selectable_ids(&self, ctx: &AppContext) -> Vec<LLMId> {
        self.presentations(ctx)
            .into_iter()
            .filter_map(|(_, presentation)| presentation.is_selectable.then_some(presentation.id))
            .collect()
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let query = input_text(&self.input_editor, ctx);
        let rows = tui_model_picker_catalog_ids(&query, ctx)
            .into_iter()
            .map(|id| TuiModelMenuRow { id })
            .collect::<Vec<_>>();
        let presentations = rows
            .iter()
            .filter_map(|row| self.presentation(row, ctx))
            .collect::<Vec<_>>();
        let active_id = self
            .owner_view
            .as_ref()
            .map(|owner_view| tui_active_model_id_for_view(owner_view, self.terminal_view_id, ctx))
            .unwrap_or_else(|| LLMId::from(""));
        let preferred_id =
            preferred_selection_id(&presentations, &active_id, query.trim().is_empty());
        let preferred_index = preferred_id.and_then(|id| rows.iter().position(|row| row.id == id));
        let selectable_ids = presentations
            .into_iter()
            .filter_map(|presentation| presentation.is_selectable.then_some(presentation.id))
            .collect::<Vec<_>>();
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |row| {
            selectable_ids.contains(&row.id)
        });
        ctx.emit(TuiModelMenuEvent);
    }

    pub(crate) fn active_model_title(&self, ctx: &AppContext) -> Option<String> {
        let owner_view = self.owner_view.as_ref()?;
        let id = tui_active_model_id_for_view(owner_view, self.terminal_view_id, ctx);
        tui_model_picker_presentation_for_view(owner_view, self.terminal_view_id, &id, ctx)
            .map(|presentation| presentation.title)
    }
}

fn snapshot_row(presentation: &TuiModelPickerPresentation) -> TuiInlineMenuRow {
    let state_suffix = match (
        presentation.is_profile_default,
        presentation.is_key_connected,
    ) {
        (true, true) => Some("(default) (key connected)".to_owned()),
        (true, false) => Some("(default)".to_owned()),
        (false, true) => Some("(key connected)".to_owned()),
        (false, false) => None,
    };
    TuiInlineMenuRow {
        title: presentation.title.clone(),
        prefix: None,
        description: (!presentation.is_selectable).then(|| "disabled".to_owned()),
        state_suffix,
        promotional_suffix: discount_label(presentation.discount_percentage),
        is_selectable: presentation.is_selectable,
        style: TuiInlineMenuRowStyle::Default,
    }
}

fn discount_label(discount_percentage: Option<f32>) -> Option<String> {
    discount_percentage
        .filter(|percentage| *percentage > 0.)
        .map(|percentage| format!("{}% off", percentage.round() as u32))
}

fn preferred_selection_id(
    presentations: &[TuiModelPickerPresentation],
    active_id: &LLMId,
    query_is_empty: bool,
) -> Option<LLMId> {
    if query_is_empty {
        presentations
            .iter()
            .find(|presentation| presentation.id == *active_id && presentation.is_selectable)
            .or_else(|| {
                presentations
                    .iter()
                    .rev()
                    .find(|presentation| presentation.is_selectable)
            })
    } else {
        presentations
            .iter()
            .rev()
            .find(|presentation| presentation.is_selectable)
    }
    .map(|presentation| presentation.id.clone())
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
