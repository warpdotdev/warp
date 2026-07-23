//! Up-arrow prompt-history inline menu state for the TUI.
//!
//! Mirrors the GUI's inline prompt-history recall (see `CODE-1871`): pressing Up
//! with the caret on the first visual row opens this menu of previously-submitted
//! agent prompts, filtered by whatever is already typed. Selection previews the
//! highlighted prompt into the input, Enter fills + submits it, and Escape (or
//! moving down past the newest row) restores the buffer the user started with.
//!
//! The prompt list comes from the shared [`prompt_history_for_terminal_view`]
//! getter so the TUI and GUI read identically ordered and de-duplicated history.
//! The model owns filtering, menu lifecycle, selection, preview, and buffer
//! snapshot/restore; the terminal session view submits an accepted prompt.
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, prompt_history_for_terminal_view,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuHeader, TuiInlineMenuListState, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, result_row_capacity,
    single_line_menu_title,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiPromptHistoryRow {
    text: String,
}

#[derive(Debug, Clone, Default)]
enum TuiPromptHistoryMenuState {
    #[default]
    Closed,
    Open {
        list: TuiInlineMenuListState<TuiPromptHistoryRow>,
        /// The input buffer captured when the menu opened, restored on dismiss.
        original_buffer: String,
        /// The user's typed search query. Held separately from the input buffer
        /// so selection previews (which overwrite the buffer) do not change what
        /// the list filters against.
        query: String,
    },
}

/// Events emitted by the TUI prompt-history menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiPromptHistoryMenuEvent {
    Updated,
}

/// Query, selection, preview, and model-subscription state for the up-arrow
/// prompt-history menu.
pub(crate) struct TuiPromptHistoryMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    terminal_surface_id: EntityId,
    state: TuiPromptHistoryMenuState,
    /// The text most recently written into the input as a preview. Content
    /// changes matching it are the editor echoing our own preview write and are
    /// ignored so they don't clobber the typed query. Model events are delivered
    /// after the current update flushes, so a transient set/reset flag around the
    /// write would not survive to the deferred handler — hence a content compare.
    preview_text: Option<String>,
}

impl TuiPromptHistoryMenuModel {
    /// Creates a closed prompt-history menu and subscribes it to input/history
    /// changes.
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
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
        Self {
            input_editor,
            suggestions_mode,
            terminal_surface_id,
            state: TuiPromptHistoryMenuState::Closed,
            preview_text: None,
        }
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiPromptHistoryMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(ctx).mode() == TuiInputSuggestionsMode::PromptHistory
    }

    /// Opens the menu, snapshotting the current input as both the restorable
    /// original buffer and the initial search query, then previews the default
    /// selection.
    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            return;
        }
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::PromptHistory, ctx)
        });
        if !did_open {
            return;
        }
        let original_buffer = input_text(&self.input_editor, ctx);
        let query = original_buffer.clone();
        self.preview_text = None;
        self.state = TuiPromptHistoryMenuState::Open {
            list: TuiInlineMenuListState::default(),
            original_buffer,
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
        let original_buffer = match &self.state {
            TuiPromptHistoryMenuState::Open {
                original_buffer, ..
            } => original_buffer.clone(),
            TuiPromptHistoryMenuState::Closed => return,
        };
        self.close(ctx);
        self.set_input_text(&original_buffer, ctx);
    }

    /// Moves selection toward older prompts and previews the highlighted one.
    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_open_state() {
            return;
        }
        if let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Moves selection toward newer prompts and previews the highlighted one.
    /// Moving down past the newest row, or from an empty list, closes the menu
    /// and restores the buffer.
    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let should_dismiss = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => {
                let count = list.rows().len();
                count == 0 || list.selected_index() == Some(count - 1)
            }
            TuiPromptHistoryMenuState::Closed => return,
        };
        if should_dismiss {
            self.dismiss(ctx);
            return;
        }
        if let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state {
            list.select_next(MAX_VISIBLE_ROWS, |_| true);
        }
        self.preview_selection(ctx);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Accepts the current selection, closing the menu and returning the text to
    /// submit. With a highlighted prompt that is its text; with an empty or
    /// filtered-to-nothing list it is the current input, so Enter behaves as a
    /// normal submit.
    pub(crate) fn accept_selected(&mut self, ctx: &mut ModelContext<Self>) -> Option<String> {
        if !self.is_open(ctx) {
            return None;
        }
        let selected = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => {
                list.selected_row().map(|row| row.text.clone())
            }
            TuiPromptHistoryMenuState::Closed => None,
        };
        let submit_text = selected.unwrap_or_else(|| input_text(&self.input_editor, ctx));
        self.close(ctx);
        Some(submit_text)
    }

    /// Returns the render snapshot for the open menu.
    pub(crate) fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(ctx) {
            return None;
        }
        let TuiPromptHistoryMenuState::Open { list, query, .. } = &self.state else {
            return None;
        };
        let status = list.rows().is_empty().then(|| {
            if query.trim().is_empty() {
                TuiInlineMenuStatus::Empty("No prompt history".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No matching prompts".to_owned())
            }
        });
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Prompt history".to_owned()),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: single_line_menu_title(&row.text),
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
        if let TuiPromptHistoryMenuState::Open { query, .. } = &mut self.state {
            *query = current;
        }
        self.refresh_rows(ctx);
    }

    /// Closes the menu without touching the input buffer.
    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            self.state = TuiPromptHistoryMenuState::Closed;
            self.preview_text = None;
            ctx.emit(TuiPromptHistoryMenuEvent::Updated);
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::PromptHistory, ctx);
        });
    }

    /// Rebuilds rows from the current query while preserving stable selection,
    /// defaulting to the row nearest the input on first populate.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let (query, previous_text, previous_index) = match &self.state {
            TuiPromptHistoryMenuState::Open { list, query, .. } => (
                query.clone(),
                list.selected_row().map(|row| row.text.clone()),
                list.selected_index(),
            ),
            TuiPromptHistoryMenuState::Closed => return,
        };
        let trimmed_query = query.trim();
        let rows: Vec<TuiPromptHistoryRow> =
            prompt_history_for_terminal_view(self.terminal_surface_id, ctx)
                .into_iter()
                .filter(|entry| {
                    trimmed_query.is_empty()
                        || entry
                            .query_text
                            .lines()
                            .any(|line| line.starts_with(trimmed_query))
                })
                .map(|entry| TuiPromptHistoryRow {
                    text: entry.query_text,
                })
                .collect();
        let preferred_index =
            reconciled_selection_index(&rows, previous_text.as_deref(), previous_index);
        let TuiPromptHistoryMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.replace_rows(rows, false, preferred_index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiPromptHistoryMenuEvent::Updated);
    }

    /// Writes the highlighted prompt into the input as an undo-agnostic preview.
    fn preview_selection(&mut self, ctx: &mut ModelContext<Self>) {
        let text = match &self.state {
            TuiPromptHistoryMenuState::Open { list, .. } => {
                list.selected_row().map(|row| row.text.clone())
            }
            TuiPromptHistoryMenuState::Closed => None,
        };
        let Some(text) = text else {
            return;
        };
        self.preview_text = Some(text.clone());
        self.set_input_text(&text, ctx);
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

/// Preserves selection by prompt text, falling back to the nearest previous
/// index and finally to the last (most-recent) row.
fn reconciled_selection_index(
    rows: &[TuiPromptHistoryRow],
    previous_text: Option<&str>,
    previous_index: Option<usize>,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let last = rows.len() - 1;
    if let Some(text) = previous_text
        && let Some(index) = rows.iter().position(|row| row.text == text)
    {
        return Some(index);
    }
    Some(previous_index.unwrap_or(last).min(last))
}

impl Entity for TuiPromptHistoryMenuModel {
    type Event = TuiPromptHistoryMenuEvent;
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
#[path = "prompt_history_menu_tests.rs"]
mod tests;
