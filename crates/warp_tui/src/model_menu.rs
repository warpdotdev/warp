//! Searchable TUI model picker state.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::AISettings;
#[cfg(test)]
use warp::tui_export::UserWorkspaces;
use warp::tui_export::{
    AISettingsChangedEvent, LLMId, LLMPreferences, LLMPreferencesEvent, ModelPickerChoice,
    TeamContextResolver, TeamScope, query_model_picker_choices, should_show_bedrock_icon_for_model,
    should_show_gemini_enterprise_agent_platform_icon_for_model, should_show_key_icon_for_model,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone)]
struct TuiModelMenuRow {
    id: LLMId,
    title: String,
    is_selectable: bool,
    is_key_connected: bool,
    is_profile_default: bool,
    discount_percentage: Option<f32>,
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
    team_context: TeamContextResolver,
    state: TuiModelMenuState,
}

impl TuiModelMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        terminal_view_id: EntityId,
        team_context: TeamContextResolver,
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
            team_context,
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
                    is_key_connected: false,
                    is_profile_default: false,
                    discount_percentage: None,
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
            terminal_view_id: EntityId::new(),
            team_context: UserWorkspaces::teamless_context_resolver_for_test(),
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

    /// Selects the row at absolute snapshot index `index` (for mouse click).
    /// Returns `true` when the row was actually selected, `false` when the
    /// index is out of bounds, the menu is not open, or the row is not
    /// selectable.
    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiModelMenuState::Open { list } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(index, MAX_VISIBLE_ROWS, |row| row.is_selectable);
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
            rows: list.rows().iter().map(snapshot_row).collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            scroll_anchor: list.scroll_anchor(),
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
        let active_id = preferences
            .get_active_base_model(ctx, Some(self.terminal_view_id))
            .id
            .clone();
        let profile_default_id = preferences
            .get_active_profile_base_model(ctx, Some(self.terminal_view_id))
            .id
            .clone();
        let scope = (self.team_context)(ctx);
        let choices = query_model_picker_choices(
            preferences,
            preferences.get_base_llm_choices_for_agent_mode(ctx),
            &query,
            &scope,
            ctx,
        );
        let rows = choices
            .into_iter()
            .map(|choice| model_menu_row(choice, &profile_default_id, &scope, ctx))
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

fn model_menu_row(
    choice: ModelPickerChoice,
    profile_default_id: &LLMId,
    scope: &dyn TeamScope,
    app: &AppContext,
) -> TuiModelMenuRow {
    let is_key_connected = should_show_key_icon_for_model(&choice.llm, scope, app);
    let uses_external_inference = is_key_connected
        || should_show_bedrock_icon_for_model(&choice.llm, app)
        || should_show_gemini_enterprise_agent_platform_icon_for_model(&choice.llm, app);
    TuiModelMenuRow {
        is_selectable: choice.is_selectable(),
        is_key_connected,
        discount_percentage: choice
            .llm
            .discount_percentage
            .filter(|_| !uses_external_inference),
        is_profile_default: choice.llm.id == *profile_default_id,
        id: choice.llm.id,
        title: choice.llm.display_name,
    }
}

fn snapshot_row(row: &TuiModelMenuRow) -> TuiInlineMenuRow {
    let state_suffix = match (row.is_profile_default, row.is_key_connected) {
        (true, true) => Some("(default) (key connected)".to_owned()),
        (true, false) => Some("(default)".to_owned()),
        (false, true) => Some("(key connected)".to_owned()),
        (false, false) => None,
    };
    TuiInlineMenuRow {
        title: row.title.clone(),
        prefix: None,
        description: (!row.is_selectable).then(|| "disabled".to_owned()),
        state_suffix,
        promotional_suffix: discount_label(row.discount_percentage),
        is_selectable: row.is_selectable,
        style: TuiInlineMenuRowStyle::Default,
    }
}

fn discount_label(discount_percentage: Option<f32>) -> Option<String> {
    discount_percentage
        .filter(|percentage| *percentage > 0.)
        .map(|percentage| format!("{}% off", percentage.round() as u32))
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
