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
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

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
        selected_index: usize,
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

    pub(crate) fn query(&self) -> Option<&str> {
        match &self.state {
            TuiSlashCommandState::Closed => None,
            TuiSlashCommandState::Open { query, .. } => Some(query),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.state, TuiSlashCommandState::Open { .. })
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
        let previous_selected_index = match &self.state {
            TuiSlashCommandState::Closed => 0,
            TuiSlashCommandState::Open { selected_index, .. } => *selected_index,
        };
        self.state = TuiSlashCommandState::Open {
            query: query.clone(),
            rows: Vec::new(),
            selected_index: previous_selected_index,
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
            selected_index,
            rows,
            is_loading,
            ..
        } = &mut self.state
        else {
            return;
        };

        let mixer = self.mixer.as_ref(ctx);
        *rows = mixer.results().iter().filter_map(row_from_result).collect();
        *is_loading = mixer.is_loading();
        *selected_index = (*selected_index).min(rows.len().saturating_sub(1));
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
