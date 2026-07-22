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
    AcceptSlashCommandOrSavedPrompt, ParsedSlashCommandInput, SlashCommandDataSource as _,
    SlashCommandMixer, TuiSlashCommandDataSource, UpdatedActiveCommands,
    should_close_slash_command_menu_for_exact_match, slash_command_query,
};
use warp_editor::model::CoreEditorModel;
use warp_search_core::inline_menu::{InlineMenuResultsUpdate, InputDrivenInlineMenuLifecycle};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuListState, TuiInlineMenuRow, TuiInlineMenuRowStyle,
    TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

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
        list: TuiInlineMenuListState<TuiSlashCommandRow>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiSlashCommandModelEvent;

pub(crate) struct TuiSlashCommandModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
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
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
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
            suggestions_mode,
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
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        mixer: ModelHandle<SlashCommandMixer>,
        rows: Vec<TuiSlashCommandRow>,
        selected_index: usize,
    ) -> Self {
        let mut list = TuiInlineMenuListState::default();
        list.replace_rows(rows, false, Some(selected_index), MAX_VISIBLE_ROWS, |_| {
            true
        });
        Self {
            input_editor,
            suggestions_mode,
            slash_commands_source: None,
            mixer,
            state: TuiSlashCommandState::Open {
                query: String::new(),
                list,
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

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiSlashCommandState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::SlashCommands
    }
    pub(crate) fn highlighted_prefix_range(&self) -> Option<Range<CharOffset>> {
        self.highlighted_prefix_len
            .map(|len| CharOffset::zero()..CharOffset::from(len))
    }

    pub(crate) fn argument_hint_text(&self) -> Option<&'static str> {
        self.argument_hint_text
    }

    pub(crate) fn selected_action(&self) -> Option<AcceptSlashCommandOrSavedPrompt> {
        let TuiSlashCommandState::Open { list, .. } = &self.state else {
            return None;
        };
        list.selected_row().map(|row| row.action.clone())
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiSlashCommandState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |_| true);
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
        let TuiSlashCommandState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiSlashCommandModelEvent);
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
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

    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiSlashCommandState::Open { list, .. } = &self.state else {
            return None;
        };
        let status = if list.rows().is_empty() {
            Some(if list.is_loading() {
                TuiInlineMenuStatus::Loading("Loading slash commands…".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No slash commands found".to_owned())
            })
        } else {
            None
        };
        Some(TuiInlineMenuSnapshot {
            header: None,
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.title.clone(),
                    description: row.description.clone(),
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::InlineMenuItem,
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    fn update_from_input(&mut self, force_query: bool, ctx: &mut ModelContext<Self>) {
        let input = input_text(&self.input_editor, ctx);
        if matches!(
            self.suggestions_mode.as_ref(ctx).mode(),
            TuiInputSuggestionsMode::ConversationMenu
                | TuiInputSuggestionsMode::ModelSelector
                | TuiInputSuggestionsMode::SkillMenu
                | TuiInputSuggestionsMode::Mcp
        ) {
            self.set_highlighted_prefix_len(None, ctx);
            self.set_argument_hint_text(None, ctx);
            self.close(ctx);
            return;
        }
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
        let menu_was_open = self.is_open(ctx);
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
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::SlashCommands, ctx)
        });
        if !did_open {
            return;
        }
        match &mut self.state {
            TuiSlashCommandState::Closed => {
                let mut list = TuiInlineMenuListState::default();
                list.set_loading(true);
                self.state = TuiSlashCommandState::Open {
                    query: query.clone(),
                    list,
                };
            }
            TuiSlashCommandState::Open {
                query: current_query,
                list,
            } => {
                *current_query = query.clone();
                list.set_loading(true);
            }
        }
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
            let TuiSlashCommandState::Open { list, .. } = &mut self.state else {
                return;
            };
            list.reconcile_mixer_rows(new_rows, mixer_is_loading, MAX_VISIBLE_ROWS, |_| true)
        };
        match results_update {
            InlineMenuResultsUpdate::Loading => return,
            InlineMenuResultsUpdate::Empty => {
                self.close(ctx);
                return;
            }
            InlineMenuResultsUpdate::Ready { .. } => {}
        }
        ctx.emit(TuiSlashCommandModelEvent);
    }

    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            self.state = TuiSlashCommandState::Closed;
            self.mixer.update(ctx, |mixer, ctx| {
                mixer.reset_results(ctx);
            });
            ctx.emit(TuiSlashCommandModelEvent);
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::SlashCommands, ctx);
        });
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
