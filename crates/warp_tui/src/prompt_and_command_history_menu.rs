//! Up-arrow prompt-and-command history inline menu state for the TUI.
//!
//! Pressing Up with the caret on the first visual row opens one menu backed by
//! the shared GUI/TUI history combiner. Agent mode shows prompts and commands;
//! shell mode shows commands only. Selection previews both text and input type,
//! while dismissing restores the original buffer and input type.
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    ActiveSession, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, BlocklistAIInputModel,
    History, HistoryEvent, InputType, InputTypeAutoDetectionSource, TuiHistoryItemKind,
    UpArrowHistoryConfig, up_arrow_history_for_terminal_view,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowPrefix, TuiInlineMenuRowPrefixStyle, TuiInlineMenuRowStyle,
    TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity, single_line_menu_title,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::terminal_block::SHELL_COMMAND_PREFIX;

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiPromptAndCommandHistoryRow {
    pub(crate) text: String,
    pub(crate) kind: TuiHistoryItemKind,
}

#[derive(Debug, Clone, Default)]
enum TuiPromptAndCommandHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiPromptAndCommandHistoryRow>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// The input type captured when the menu opened, restored on dismiss.
        original_input_type: InputType,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
    },
}

/// Events emitted by the TUI prompt-and-command history menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiPromptAndCommandHistoryMenuEvent {
    Updated,
}

/// Query, selection, preview, and model-subscription state for the up-arrow
/// prompt-and-command history menu.
pub(crate) struct TuiPromptAndCommandHistoryMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    active_session: ModelHandle<ActiveSession>,
    terminal_surface_id: EntityId,
    state: TuiPromptAndCommandHistoryMenuState,
    /// The text most recently written into the input as a preview. Content
    /// changes matching it are the editor echoing our own preview write and are
    /// ignored so they don't clobber the typed query. Model events are delivered
    /// after the current update flushes, so a transient set/reset flag around the
    /// write would not survive to the deferred handler — hence a content compare.
    preview_text: Option<String>,
}

impl TuiPromptAndCommandHistoryMenuModel {
    /// Creates a closed prompt-and-command history menu and subscribes it to input/history
    /// changes.
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        active_session: ModelHandle<ActiveSession>,
        terminal_surface_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open(ctx) && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.on_content_changed(ctx);
            }
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |model, _, _: &BlocklistAIHistoryEvent, ctx| {
                if model.is_open(ctx) {
                    model.refresh_rows(ctx);
                }
            },
        );
        ctx.subscribe_to_model(&History::handle(ctx), |model, _, _: &HistoryEvent, ctx| {
            if model.is_open(ctx) {
                model.refresh_rows(ctx);
            }
        });
        Self {
            input_editor,
            input_mode,
            suggestions_mode,
            active_session,
            terminal_surface_id,
            state: TuiPromptAndCommandHistoryMenuState::Closed,
            preview_text: None,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiPromptAndCommandHistoryMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode()
                == TuiInputSuggestionsMode::PromptAndCommandHistory
    }

    /// Opens the menu, snapshotting the current input as both the restorable
    /// original buffer and the initial search query, then previews the default
    /// selection.
    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::PromptAndCommandHistory, ctx)
        });
        if !did_open {
            return;
        }
        let original_buffer = input_text(&self.input_editor, ctx);
        let original_input_type = self.input_mode.as_ref(ctx).input_type();
        let query = original_buffer.clone();
        self.preview_text = None;
        self.state = TuiPromptAndCommandHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
            original_input_type,
            query,
        };
        self.refresh_rows(ctx);
        self.preview_selection(ctx);
    }

    /// Closes the menu and restores the buffer the user had before opening it.
    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        let (original_buffer, original_input_type) = match &self.state {
            TuiPromptAndCommandHistoryMenuState::Open {
                original_buffer,
                original_input_type,
                ..
            } => (original_buffer.clone(), *original_input_type),
            TuiPromptAndCommandHistoryMenuState::Closed => return,
        };
        self.close(ctx);
        self.set_input_type(
            original_input_type,
            InputTypeAutoDetectionSource::RestoreSavedConfig,
            ctx,
        );
        self.set_input_text(&original_buffer, ctx);
    }

    /// Moves selection toward older history items and previews the highlighted one.
    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_open_state() {
            return;
        }
        if let TuiPromptAndCommandHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiPromptAndCommandHistoryMenuEvent::Updated);
    }

    /// Moves selection toward newer history items and previews the highlighted one.
    /// Moving down past the newest row, or from an empty list, closes the menu
    /// and restores the buffer.
    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let should_dismiss = match &self.state {
            TuiPromptAndCommandHistoryMenuState::Open { list, .. } => {
                let count = list.rows().len();
                count == 0 || list.selected_index() == Some(count - 1)
            }
            TuiPromptAndCommandHistoryMenuState::Closed => return,
        };
        if should_dismiss {
            self.dismiss(ctx);
            return;
        }
        if let TuiPromptAndCommandHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_next(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiPromptAndCommandHistoryMenuEvent::Updated);
    }

    /// Accepts the current selection, closing the menu and returning the text to
    /// submit. With a highlighted item that is its text; with an empty or
    /// filtered-to-nothing list it is the current input, so Enter behaves as a
    /// normal submit.
    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiPromptAndCommandHistoryRow> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiPromptAndCommandHistoryMenuState::Open { list, .. } => list.selected_row().cloned(),
            TuiPromptAndCommandHistoryMenuState::Closed => None,
        };
        let accepted = selected.unwrap_or_else(|| TuiPromptAndCommandHistoryRow {
            text: input_text(&self.input_editor, ctx),
            kind: match self.input_mode.as_ref(ctx).input_type() {
                InputType::AI => TuiHistoryItemKind::Prompt,
                InputType::Shell => TuiHistoryItemKind::Command {
                    linked_workflow_data: None,
                },
            },
        });
        self.close(ctx);
        Some(accepted)
    }

    /// Returns the render snapshot for the open menu.
    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiPromptAndCommandHistoryMenuState::Open { list, query, .. } = &self.state else {
            return None;
        };
        let status = list.rows().is_empty().then(|| {
            if query.trim().is_empty() {
                TuiInlineMenuStatus::Empty("No history".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No matching history".to_owned())
            }
        });
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("History".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: single_line_menu_title(&row.text),
                    prefix: matches!(row.kind, TuiHistoryItemKind::Command { .. }).then(|| {
                        TuiInlineMenuRowPrefix {
                            text: format!("{SHELL_COMMAND_PREFIX} "),
                            style: TuiInlineMenuRowPrefixStyle::ShellCommand,
                        }
                    }),
                    description: None,
                    state_suffix: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    /// Re-reads the typed query from the input when the buffer changes from real
    /// typing, ignoring the editor echo of our own preview writes.
    fn on_content_changed(&mut self, ctx: &mut ModelContext<Self>) {
        let current = input_text(&self.input_editor, ctx);
        if self.preview_text.as_deref() == Some(current.as_str()) {
            return;
        }
        self.preview_text = None;
        if let TuiPromptAndCommandHistoryMenuState::Open { query, .. } = &mut self.state {
            *query = current;
        }
        self.refresh_rows(ctx);
    }

    /// Closes the menu without touching the input buffer.
    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            self.state = TuiPromptAndCommandHistoryMenuState::Closed;
            self.preview_text = None;
            ctx.emit(TuiPromptAndCommandHistoryMenuEvent::Updated);
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::PromptAndCommandHistory, ctx);
        });
    }

    fn set_input_type(
        &self,
        input_type: InputType,
        source: InputTypeAutoDetectionSource,
        ctx: &mut ModelContext<Self>,
    ) {
        self.input_mode.update(ctx, |input_mode, ctx| {
            input_mode.set_input_type(input_type, Some(source), ctx);
        });
    }

    /// Rebuilds rows from the current query while preserving stable selection,
    /// defaulting to the row nearest the input on first populate.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let (query, previous_row, previous_index, include_prompts) = match &self.state {
            TuiPromptAndCommandHistoryMenuState::Open {
                list,
                query,
                original_input_type,
                ..
            } => (
                query.clone(),
                list.selected_row().cloned(),
                list.selected_index(),
                *original_input_type == InputType::AI,
            ),
            TuiPromptAndCommandHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        let session_id = self.active_session.as_ref(ctx).session_id(ctx);
        let rows = up_arrow_history_for_terminal_view(
            self.terminal_surface_id,
            session_id,
            UpArrowHistoryConfig {
                include_commands: true,
                include_prompts,
            },
            ctx,
        )
        .into_iter()
        .filter(|item| {
            trimmed_query.is_empty()
                || item
                    .text
                    .lines()
                    .any(|line| line.starts_with(trimmed_query))
        })
        .map(|item| TuiPromptAndCommandHistoryRow {
            text: item.text,
            kind: item.kind,
        })
        .collect::<Vec<_>>();
        let preferred_index =
            reconciled_selection_index(&rows, previous_row.as_ref(), previous_index);
        let TuiPromptAndCommandHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiPromptAndCommandHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted history item into the input as an undo-agnostic preview.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let row = match &self.state {
            TuiPromptAndCommandHistoryMenuState::Open { list, .. } => list.selected_row().cloned(),
            TuiPromptAndCommandHistoryMenuState::Closed => None,
        };
        let Some(row) = row else {
            return;
        };
        let input_type = match row.kind {
            TuiHistoryItemKind::Prompt => InputType::AI,
            TuiHistoryItemKind::Command { .. } => InputType::Shell,
        };
        self.set_input_type(
            input_type,
            InputTypeAutoDetectionSource::HistorySelection,
            ctx,
        );
        self.preview_text = Some(row.text.clone());
        self.set_input_text(&row.text, ctx);
    }

    /// Replaces the input buffer text. Preview and restore both go through here.
    ///
    /// The write is undo-agnostic: after replacing the text we reset the buffer's
    /// undo stack so preview and restore never leave undoable intermediate states
    /// the user could Ctrl+Z into. This mirrors the
    /// GUI's `set_buffer_text_ignoring_undo`; the TUI's `CodeEditorModel` has no
    /// ephemeral overlay, so we clear the stack instead.
    fn set_input_text(&self, text: &str, ctx: &mut ModelContext<Self>) {
        self.input_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
            if !text.is_empty() {
                editor.user_insert(text, ctx);
            }
            editor
                .content()
                .update(ctx, |buffer, _| buffer.reset_undo_stack());
        });
    }
}

/// Preserves selection by history item, falling back to the nearest previous
/// index and finally to the last (most-recent) row.
fn reconciled_selection_index(
    rows: &[TuiPromptAndCommandHistoryRow],
    previous_row: Option<&TuiPromptAndCommandHistoryRow>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some(previous_row) = previous_row
        && let Some(index) = rows.iter().position(|row| row == previous_row)
    {
        return Some(index);
    }
    Some(previous_index.unwrap_or(last).min(last))
}

impl Entity for TuiPromptAndCommandHistoryMenuModel {
    type Event = TuiPromptAndCommandHistoryMenuEvent;
}

/// Returns the input editor's current plain text.
fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

#[cfg(test)]
#[path = "prompt_and_command_history_menu_tests.rs"]
mod tests;
