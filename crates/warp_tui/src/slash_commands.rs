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
use warpui_core::elements::tui::{TuiContainer, TuiElement, TuiFlex, TuiText};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::tui_builder::TuiUiBuilder;

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
        selected_index: usize,
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
            rows,
            selected_index,
            ..
        } = &self.state
        else {
            return None;
        };
        rows.get(*selected_index).map(|row| row.action.clone())
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSlashCommandState::Open {
            rows,
            selected_index,
            scroll_offset,
            ..
        } = &mut self.state
        else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        *selected_index = if *selected_index == 0 {
            rows.len() - 1
        } else {
            *selected_index - 1
        };
        keep_selected_visible(rows.len(), *selected_index, scroll_offset);
        ctx.emit(TuiSlashCommandModelEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSlashCommandState::Open {
            rows,
            selected_index,
            scroll_offset,
            ..
        } = &mut self.state
        else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        *selected_index = (*selected_index + 1) % rows.len();
        keep_selected_visible(rows.len(), *selected_index, scroll_offset);
        ctx.emit(TuiSlashCommandModelEvent);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        self.close(ctx);
    }

    pub(crate) fn render_menu(&self, app: &AppContext) -> Option<Box<dyn TuiElement>> {
        let TuiSlashCommandState::Open {
            rows,
            selected_index,
            scroll_offset,
            is_loading,
            ..
        } = &self.state
        else {
            return None;
        };

        let builder = TuiUiBuilder::from_app(app);
        let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        if rows.is_empty() {
            let label = if *is_loading {
                "Loading slash commands…"
            } else {
                "No slash commands found"
            };
            column = column.child(menu_status_row(label, &builder));
        } else {
            for (index, row) in rows
                .iter()
                .enumerate()
                .skip(*scroll_offset)
                .take(MAX_VISIBLE_ROWS)
            {
                column = column.child(menu_result_row(row, index == *selected_index, &builder));
            }
        }

        Some(
            TuiContainer::new(column.finish())
                .with_border_style(builder.accent_border_style())
                .finish(),
        )
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
        let (previous_query_matches, previous_selected_index, previous_scroll_offset) =
            match &self.state {
                TuiSlashCommandState::Closed => (false, 0, 0),
                TuiSlashCommandState::Open {
                    query: previous_query,
                    selected_index,
                    scroll_offset,
                    ..
                } => (previous_query == &query, *selected_index, *scroll_offset),
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
            selected_index: previous_selected_index,
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
            selected_index,
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
            *selected_index = 0;
            *scroll_offset = 0;
        } else {
            if *select_last_result_on_refresh {
                *selected_index = rows.len() - 1;
                *select_last_result_on_refresh = false;
            } else {
                *selected_index = (*selected_index).min(rows.len() - 1);
            }
            keep_selected_visible(rows.len(), *selected_index, scroll_offset);
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
fn keep_selected_visible(rows_len: usize, selected_index: usize, scroll_offset: &mut usize) {
    if rows_len == 0 {
        *scroll_offset = 0;
        return;
    }

    let max_scroll_offset = rows_len.saturating_sub(MAX_VISIBLE_ROWS);
    *scroll_offset = (*scroll_offset).min(max_scroll_offset);
    if selected_index < *scroll_offset {
        *scroll_offset = selected_index;
    } else if selected_index >= *scroll_offset + MAX_VISIBLE_ROWS {
        *scroll_offset = selected_index + 1 - MAX_VISIBLE_ROWS;
    }
}

fn menu_status_row(label: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiContainer::new(
        TuiText::new(label.to_owned())
            .with_style(builder.dim_text_style())
            .truncate()
            .finish(),
    )
    .with_padding_left(1)
    .with_padding_right(1)
    .finish()
}

fn menu_result_row(
    row: &TuiSlashCommandRow,
    is_selected: bool,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let title_style = if is_selected {
        builder.input_text_style()
    } else {
        builder.primary_text_style()
    };
    let description_style = if is_selected {
        builder.input_text_style()
    } else {
        builder.muted_text_style()
    };

    let mut content = TuiFlex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            TuiText::new(row.title.clone())
                .with_style(title_style)
                .truncate()
                .finish(),
        );
    if let Some(description) = &row.description {
        content = content.child(
            TuiText::new(format!("  {description}"))
                .with_style(description_style)
                .truncate()
                .finish(),
        );
    }

    let mut container = TuiContainer::new(content.finish())
        .with_padding_left(1)
        .with_padding_right(1);
    if is_selected {
        container = container.with_background(builder.input_background());
    }
    container.finish()
}
