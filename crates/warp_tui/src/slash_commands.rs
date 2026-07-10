//! TUI slash command query state.
//!
//! This module owns the TUI-side slash command state and search mixer wiring.
//! Rendering and keyboard dispatch live in later layers; this model is only
//! responsible for tracking when slash command composition is active, running
//! shared-source queries, and snapshotting render-friendly row data.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::search::data_source::QueryResult;
use warp::search::mixer::SearchMixerEvent;
use warp::tui_export::{
    slash_command_composition_filter, slash_command_query, AcceptSlashCommandOrSavedPrompt,
    SlashCommandMixer, TuiSlashCommandDataSource, UpdatedActiveCommands,
};
use warp_editor::model::CoreEditorModel;
use warp_search_core::inline_menu::{InitialSelection, InlineMenuSelection};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::inline_menu::{
    keep_selected_visible, TuiInlineMenuRow, TuiInlineMenuSnapshot, TuiInlineMenuStatus,
};

const MAX_VISIBLE_ROWS: usize = 8;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct TuiSlashCommandRow {
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) action: AcceptSlashCommandOrSavedPrompt,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) enum TuiSlashCommandState {
    #[default]
    Closed,
    Open {
        query: String,
        rows: Vec<TuiSlashCommandRow>,
        selection: InlineMenuSelection,
        scroll_offset: usize,
        select_last_result_on_refresh: bool,
        is_loading: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiSlashCommandModelEvent;

pub(crate) struct TuiSlashCommandModel {
    input_editor: ModelHandle<CodeEditorModel>,
    mixer: ModelHandle<SlashCommandMixer>,
    state: TuiSlashCommandState,
}

impl TuiSlashCommandModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        slash_commands_source: ModelHandle<TuiSlashCommandDataSource>,
        mixer: ModelHandle<SlashCommandMixer>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |me, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                me.update_from_input(ctx);
            }
        });
        ctx.subscribe_to_model(
            &slash_commands_source,
            |me, _, _: &UpdatedActiveCommands, ctx| {
                if let Some(query) = me.query().map(str::to_owned) {
                    me.run_query(query, true, ctx);
                }
            },
        );
        ctx.subscribe_to_model(&mixer, |me, _, event, ctx| {
            if matches!(event, SearchMixerEvent::ResultsChanged) {
                me.refresh_rows(ctx);
            }
        });

        let mut model = Self {
            input_editor,
            mixer,
            state: TuiSlashCommandState::Closed,
        };
        model.update_from_input(ctx);
        model
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        input_editor: ModelHandle<CodeEditorModel>,
        mixer: ModelHandle<SlashCommandMixer>,
        rows: Vec<TuiSlashCommandRow>,
        selected_index: usize,
    ) -> Self {
        let mut selection = InlineMenuSelection::default();
        selection.select(selected_index, rows.len(), |_| true);
        Self {
            input_editor,
            mixer,
            state: TuiSlashCommandState::Open {
                query: String::new(),
                rows,
                selection,
                scroll_offset: 0,
                select_last_result_on_refresh: false,
                is_loading: false,
            },
        }
    }

    pub(crate) fn query(&self) -> Option<&str> {
        match &self.state {
            TuiSlashCommandState::Closed => None,
            TuiSlashCommandState::Open { query, .. } => Some(query),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.state, TuiSlashCommandState::Open { .. })
    }

    pub(crate) fn selected_action(&self) -> Option<AcceptSlashCommandOrSavedPrompt> {
        let TuiSlashCommandState::Open {
            rows, selection, ..
        } = &self.state
        else {
            return None;
        };
        selection
            .selected_index()
            .and_then(|index| rows.get(index))
            .map(|row| row.action.clone())
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSlashCommandState::Open {
            rows,
            selection,
            scroll_offset,
            ..
        } = &mut self.state
        else {
            return;
        };
        if let Some(selected_index) = selection.select_previous(rows.len(), |_| true) {
            keep_selected_visible(rows.len(), selected_index, MAX_VISIBLE_ROWS, scroll_offset);
        }
        ctx.emit(TuiSlashCommandModelEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSlashCommandState::Open {
            rows,
            selection,
            scroll_offset,
            ..
        } = &mut self.state
        else {
            return;
        };
        if let Some(selected_index) = selection.select_next(rows.len(), |_| true) {
            keep_selected_visible(rows.len(), selected_index, MAX_VISIBLE_ROWS, scroll_offset);
        }
        ctx.emit(TuiSlashCommandModelEvent);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        self.close(ctx);
    }

    pub(crate) fn snapshot(&self) -> Option<TuiInlineMenuSnapshot> {
        let TuiSlashCommandState::Open {
            rows,
            selection,
            scroll_offset,
            is_loading,
            ..
        } = &self.state
        else {
            return None;
        };
        let status = if rows.is_empty() {
            Some(if *is_loading {
                TuiInlineMenuStatus::Loading("Loading slash commands…".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No slash commands found".to_owned())
            })
        } else {
            None
        };
        Some(TuiInlineMenuSnapshot {
            header: None,
            rows: rows
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.title.clone(),
                    description: row.description.clone(),
                    is_selectable: true,
                })
                .collect(),
            selected_index: selection.selected_index(),
            scroll_offset: *scroll_offset,
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    fn update_from_input(&mut self, ctx: &mut ModelContext<Self>) {
        let input = input_text(&self.input_editor, ctx);
        let Some(query) = slash_command_composition_filter(&input).map(str::to_owned) else {
            self.close(ctx);
            return;
        };
        self.run_query(query, false, ctx);
    }

    fn run_query(&mut self, query: String, force: bool, ctx: &mut ModelContext<Self>) {
        let (previous_query_matches, previous_selection, previous_scroll_offset) = match &self.state
        {
            TuiSlashCommandState::Closed => (false, InlineMenuSelection::default(), 0),
            TuiSlashCommandState::Open {
                query: previous_query,
                selection,
                scroll_offset,
                ..
            } => (previous_query == &query, *selection, *scroll_offset),
        };
        let select_last_result_on_refresh = query.is_empty() && !previous_query_matches;
        let scroll_offset = if select_last_result_on_refresh {
            0
        } else {
            previous_scroll_offset
        };
        self.state = TuiSlashCommandState::Open {
            query: query.clone(),
            rows: Vec::new(),
            selection: previous_selection,
            scroll_offset,
            select_last_result_on_refresh,
            is_loading: true,
        };
        self.mixer.update(ctx, |mixer, ctx| {
            if !force && mixer.current_query().is_some_and(|q| q.text == query) {
                return;
            }
            mixer.run_query(slash_command_query(&query), ctx);
        });
        self.refresh_rows(ctx);
    }

    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSlashCommandState::Open {
            selection,
            scroll_offset,
            rows,
            select_last_result_on_refresh,
            is_loading,
            ..
        } = &mut self.state
        else {
            return;
        };

        let mixer = self.mixer.as_ref(ctx);
        *rows = mixer.results().iter().filter_map(row_from_result).collect();
        *is_loading = mixer.is_loading();
        if rows.is_empty() {
            selection.clear();
            *scroll_offset = 0;
        } else {
            if *select_last_result_on_refresh {
                selection.reset(rows.len(), InitialSelection::Last, |_| true);
                *select_last_result_on_refresh = false;
            } else if let Some(selected_index) = selection.selected_index() {
                selection.select(selected_index.min(rows.len() - 1), rows.len(), |_| true);
            } else {
                selection.reset(rows.len(), InitialSelection::First, |_| true);
            }
            if let Some(selected_index) = selection.selected_index() {
                keep_selected_visible(rows.len(), selected_index, MAX_VISIBLE_ROWS, scroll_offset);
            }
        }
        ctx.emit(TuiSlashCommandModelEvent);
    }

    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open() {
            return;
        }
        self.state = TuiSlashCommandState::Closed;
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset_results(ctx);
        });
        ctx.emit(TuiSlashCommandModelEvent);
    }
}

impl Entity for TuiSlashCommandModel {
    type Event = TuiSlashCommandModelEvent;
}

fn input_text(input_editor: &ModelHandle<CodeEditorModel>, ctx: &AppContext) -> String {
    let editor = input_editor.as_ref(ctx);
    let content = editor.content().as_ref(ctx);
    if content.is_empty() {
        String::new()
    } else {
        content.text().into_string()
    }
}

fn row_from_result(
    result: &QueryResult<AcceptSlashCommandOrSavedPrompt>,
) -> Option<TuiSlashCommandRow> {
    if result.is_static_separator() || result.is_disabled() {
        return None;
    }
    let detail = result.detail_data()?;
    Some(TuiSlashCommandRow {
        title: detail.title,
        description: detail.description,
        action: result.accept_result(),
    })
}
