//! TUI slash command query state.
//!
//! This module owns the TUI-side slash command state and search mixer wiring.
//! Rendering and keyboard dispatch live in later layers; this model is only
//! responsible for tracking when slash command composition is active, running
//! shared-source queries, and snapshotting render-friendly row data.
use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::search::data_source::QueryResult;
use warp::search::mixer::SearchMixerEvent;
use warp::tui_export::{
    should_close_slash_command_menu_for_exact_match, slash_command_query,
    AcceptSlashCommandOrSavedPrompt, ParsedSlashCommandInput, SlashCommandDataSource as _,
    SlashCommandMixer, TuiSlashCommandDataSource, UpdatedActiveCommands,
};
use warp_editor::model::CoreEditorModel;
use warp_search_core::inline_menu::{
    InlineMenuResultsUpdate, InlineMenuSelection, InputDrivenInlineMenuLifecycle,
};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::inline_menu::{
    keep_selected_visible, result_row_capacity, TuiInlineMenuRow, TuiInlineMenuRowStyle,
    TuiInlineMenuSnapshot, TuiInlineMenuStatus, MAX_INLINE_MENU_ROWS,
};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, false, false);

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct TuiSlashCommandRow {
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) action: AcceptSlashCommandOrSavedPrompt,
}
fn highlighted_prefix_len_for_parsed_input(
    parsed_input: &ParsedSlashCommandInput,
    input: &str,
) -> Option<usize> {
    match parsed_input {
        ParsedSlashCommandInput::SlashCommand(detected) => input
            .starts_with(detected.command.name)
            .then(|| detected.command.name.chars().count()),
        ParsedSlashCommandInput::SkillCommand(detected) => {
            let prefix = format!("/{}", detected.name);
            input.starts_with(&prefix).then(|| prefix.chars().count())
        }
        ParsedSlashCommandInput::None | ParsedSlashCommandInput::Composing { .. } => None,
    }
}

fn argument_hint_text_for_parsed_input(
    parsed_input: &ParsedSlashCommandInput,
    input: &str,
) -> Option<&'static str> {
    let ParsedSlashCommandInput::SlashCommand(detected) = parsed_input else {
        return None;
    };
    detected
        .command
        .argument_hint()
        .filter(|hint| hint.input_prefix == input)
        .map(|hint| hint.text)
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
        is_loading: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiSlashCommandModelEvent;

pub(crate) struct TuiSlashCommandModel {
    input_editor: ModelHandle<CodeEditorModel>,
    slash_commands_source: Option<ModelHandle<TuiSlashCommandDataSource>>,
    mixer: ModelHandle<SlashCommandMixer>,
    state: TuiSlashCommandState,
    lifecycle: InputDrivenInlineMenuLifecycle,
    highlighted_prefix_len: Option<usize>,
    argument_hint_text: Option<&'static str>,
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
                me.update_from_input(false, ctx);
            }
        });
        ctx.subscribe_to_model(
            &slash_commands_source,
            |me, _, _: &UpdatedActiveCommands, ctx| {
                me.update_from_input(true, ctx);
            },
        );
        ctx.subscribe_to_model(&mixer, |me, _, event, ctx| {
            if matches!(event, SearchMixerEvent::ResultsChanged) {
                me.refresh_rows(ctx);
            }
        });

        let mut model = Self {
            input_editor,
            slash_commands_source: Some(slash_commands_source),
            mixer,
            state: TuiSlashCommandState::Closed,
            lifecycle: InputDrivenInlineMenuLifecycle::default(),
            highlighted_prefix_len: None,
            argument_hint_text: None,
        };
        model.update_from_input(false, ctx);
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
            slash_commands_source: None,
            mixer,
            state: TuiSlashCommandState::Open {
                query: String::new(),
                rows,
                selection,
                scroll_offset: 0,
                is_loading: false,
            },
            lifecycle: InputDrivenInlineMenuLifecycle::default(),
            highlighted_prefix_len: None,
            argument_hint_text: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_highlighted_prefix_len_for_test(&mut self, len: Option<usize>) {
        self.highlighted_prefix_len = len;
    }

    #[cfg(test)]
    pub(crate) fn set_argument_hint_text_for_test(&mut self, text: Option<&'static str>) {
        self.argument_hint_text = text;
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self.state, TuiSlashCommandState::Open { .. })
    }
    pub(crate) fn highlighted_prefix_range(&self) -> Option<Range<CharOffset>> {
        self.highlighted_prefix_len
            .map(|len| CharOffset::zero()..CharOffset::from(len))
    }

    pub(crate) fn argument_hint_text(&self) -> Option<&'static str> {
        self.argument_hint_text
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

    fn set_argument_hint_text(
        &mut self,
        argument_hint_text: Option<&'static str>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.argument_hint_text == argument_hint_text {
            return;
        }
        self.argument_hint_text = argument_hint_text;
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
        if !self.is_open() {
            return;
        }
        let input_is_empty = input_text(&self.input_editor, ctx).is_empty();
        self.lifecycle.disable_until_empty_buffer(input_is_empty);
        self.close(ctx);
    }

    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<AcceptSlashCommandOrSavedPrompt> {
        let action = self.selected_action();
        self.close(ctx);
        action
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
                    style: TuiInlineMenuRowStyle::SlashCommand,
                })
                .collect(),
            selected_index: selection.selected_index(),
            scroll_offset: *scroll_offset,
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    fn update_from_input(&mut self, force_query: bool, ctx: &mut ModelContext<Self>) {
        let input = input_text(&self.input_editor, ctx);
        if !self
            .lifecycle
            .input_changed(input.is_empty(), input.starts_with('/'))
        {
            self.set_highlighted_prefix_len(None, ctx);
            self.set_argument_hint_text(None, ctx);
            self.close(ctx);
            return;
        }
        let Some(slash_commands_source) = &self.slash_commands_source else {
            self.set_highlighted_prefix_len(None, ctx);
            return;
        };
        let parsed_input = slash_commands_source.as_ref(ctx).parse_input(&input, ctx);
        self.set_highlighted_prefix_len(
            highlighted_prefix_len_for_parsed_input(&parsed_input, &input),
            ctx,
        );
        self.set_argument_hint_text(
            argument_hint_text_for_parsed_input(&parsed_input, &input),
            ctx,
        );
        let menu_was_open = self.is_open();
        let result_count = self.mixer.as_ref(ctx).results().len();
        let Some(query) = menu_query_for_parsed_input(&parsed_input, menu_was_open, result_count)
        else {
            self.close(ctx);
            return;
        };
        self.run_query(query, force_query, ctx);
    }

    fn set_highlighted_prefix_len(
        &mut self,
        highlighted_prefix_len: Option<usize>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.highlighted_prefix_len == highlighted_prefix_len {
            return;
        }
        self.highlighted_prefix_len = highlighted_prefix_len;
        ctx.emit(TuiSlashCommandModelEvent);
    }

    fn run_query(&mut self, query: String, force: bool, ctx: &mut ModelContext<Self>) {
        let (previous_rows, previous_selection, previous_scroll_offset) = match &self.state {
            TuiSlashCommandState::Closed => (Vec::new(), InlineMenuSelection::default(), 0),
            TuiSlashCommandState::Open {
                rows,
                selection,
                scroll_offset,
                ..
            } => (rows.clone(), *selection, *scroll_offset),
        };
        self.state = TuiSlashCommandState::Open {
            query: query.clone(),
            rows: previous_rows,
            selection: previous_selection,
            scroll_offset: previous_scroll_offset,
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
        let (mixer_is_loading, new_rows): (bool, Vec<TuiSlashCommandRow>) = {
            let mixer = self.mixer.as_ref(ctx);
            (
                mixer.is_loading(),
                mixer.results().iter().filter_map(row_from_result).collect(),
            )
        };
        let results_update = {
            let TuiSlashCommandState::Open {
                selection,
                is_loading,
                ..
            } = &mut self.state
            else {
                return;
            };
            *is_loading = mixer_is_loading;
            selection.reconcile_results(mixer_is_loading, new_rows.len(), |_| true)
        };
        let selected_index = match results_update {
            InlineMenuResultsUpdate::Loading => return,
            InlineMenuResultsUpdate::Empty => {
                self.close(ctx);
                return;
            }
            InlineMenuResultsUpdate::Ready { selected_index } => selected_index,
        };

        let TuiSlashCommandState::Open {
            scroll_offset,
            rows,
            ..
        } = &mut self.state
        else {
            return;
        };
        *rows = new_rows;
        if let Some(selected_index) = selected_index {
            keep_selected_visible(rows.len(), selected_index, MAX_VISIBLE_ROWS, scroll_offset);
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

fn menu_query_for_parsed_input(
    parsed_input: &ParsedSlashCommandInput,
    menu_was_open: bool,
    result_count: usize,
) -> Option<String> {
    match parsed_input {
        ParsedSlashCommandInput::None => None,
        ParsedSlashCommandInput::Composing { filter } => Some(filter.clone()),
        ParsedSlashCommandInput::SlashCommand(detected_command) => {
            if !menu_was_open
                || should_close_slash_command_menu_for_exact_match(
                    result_count,
                    detected_command.argument.is_some(),
                )
            {
                None
            } else {
                Some(
                    detected_command
                        .command
                        .name
                        .strip_prefix('/')
                        .unwrap_or(detected_command.command.name)
                        .to_owned(),
                )
            }
        }
        ParsedSlashCommandInput::SkillCommand(detected_skill) => {
            if !menu_was_open
                || should_close_slash_command_menu_for_exact_match(
                    result_count,
                    detected_skill.argument.is_some(),
                )
            {
                None
            } else {
                Some(detected_skill.name.clone())
            }
        }
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

#[cfg(test)]
#[path = "slash_commands_tests.rs"]
mod tests;
