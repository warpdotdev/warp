//! [`TuiInputView`] — ratatui-rendered TUI prompt input.
//!
//! Implements [`TuiView`] + [`TypedActionView`]. The view:
//!
//! - Holds a [`ModelHandle<CodeEditorModel>`] constructed in `LayoutMode::CharCell`.
//! - Owns all TUI-specific session state: kill buffer, scroll offset, terminal width.
//! - Dispatches keystrokes as [`TuiInputAction`] typed actions.
//! - Emits [`TuiInputViewEvent::Submitted`] when the user presses Enter.
//!
//! # Architecture
//!
//! The view works directly with [`CodeEditorModel`] (char-cell mode) so that future
//! TUI features — vim, syntax highlighting, diff, hidden lines — come for free from
//! the shared editor infrastructure.  TUI-specific concepts (kill ring, terminal
//! viewport scroll, readline keybindings) belong to this view layer, not to the
//! underlying editor model.
//!
//! See `specs/tui-input-view/TECH.md` for the full keybinding table.

use std::cmp;
use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::render::model::{
    char_cell_display_width, char_cell_line_gap_position, char_cell_line_row_starts, ColumnUnit,
    SoftWrapPoint,
};
use warp_editor::selection::TextUnit;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiColumn, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiParentElement, TuiPoint, TuiRect, TuiRectExt, TuiSize, TuiStyle, TuiText,
};
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewContext};

use super::kill_buffer::KillBuffer;

/// Logical rows scrolled per mouse-wheel notch (matches `TuiScrollable`).
const WHEEL_STEP: isize = 2;

// ─────────────────────────────────────────────────────────────────────────────
// View events
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by [`TuiInputView`].
#[derive(Debug, Clone)]
pub enum TuiInputViewEvent {
    /// The user pressed Enter to submit the current input. Contains the final text.
    Submitted(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed action enum
// ─────────────────────────────────────────────────────────────────────────────

/// All editing operations dispatched from `TuiInputElement::dispatch_event`.
///
/// Each variant corresponds to one or more keybindings from the spec keybinding table.
#[derive(Debug, Clone)]
pub enum TuiInputAction {
    /// Insert a character (`Char(c)` key events).
    InsertChar(char),
    /// Insert a hard newline (`Shift+Enter`, `Ctrl+J`, `Alt+Enter`).
    InsertNewline,
    /// Submit the current input (`Enter`).
    Submit,
    /// Delete the character before the cursor (`Backspace`, `Ctrl+H`).
    Backspace,
    /// Delete the character after the cursor (`Delete`, `Ctrl+D`).
    DeleteForward,
    /// Move cursor left one char (`←`, `Ctrl+B`).
    MoveLeft,
    /// Move cursor right one char (`→`, `Ctrl+F`).
    MoveRight,
    /// Move cursor up one visual row (`↑`, `Ctrl+P`).
    MoveUp,
    /// Move cursor down one visual row (`↓`, `Ctrl+N`).
    MoveDown,
    /// Move cursor one word backward (`Alt+←`, `Alt+B`, `Ctrl+←`).
    MoveWordLeft,
    /// Move cursor one word forward (`Alt+→`, `Alt+F`, `Ctrl+→`).
    MoveWordRight,
    /// Move cursor to start of visual line (`Home`, `Ctrl+A`).
    MoveToLineStart,
    /// Move cursor to end of visual line (`End`, `Ctrl+E`).
    MoveToLineEnd,
    /// Extend selection left (`Shift+←`).
    SelectLeft,
    /// Extend selection right (`Shift+→`).
    SelectRight,
    /// Extend selection up (`Shift+↑`).
    SelectUp,
    /// Extend selection down (`Shift+↓`).
    SelectDown,
    /// Extend selection one word left (`Ctrl+Shift+←`, `Alt+Shift+←`).
    SelectWordLeft,
    /// Extend selection one word right (`Ctrl+Shift+→`, `Alt+Shift+→`).
    SelectWordRight,
    /// Select all text (`Ctrl+Shift+A` / `Meta+A`).
    SelectAll,
    /// Delete word backward (`Ctrl+W`, `Alt+Backspace`, `Ctrl+Backspace`).
    DeleteWordBackward,
    /// Delete word forward (`Alt+D`, `Alt+Delete`, `Ctrl+Delete`).
    DeleteWordForward,
    /// Kill from cursor to end of visual line (`Ctrl+K`).
    KillToLineEnd,
    /// Kill from cursor to start of visual line (`Ctrl+U`).
    KillToLineStart,
    /// Yank last killed text (`Ctrl+Y`).
    Yank,
    /// Undo (`Ctrl+Z`).
    Undo,
    /// Redo (`Ctrl+Shift+Z`).
    Redo,
    /// Place the cursor / begin a character selection at `offset` (single click).
    SelectionStartAt { offset: CharOffset },
    /// Extend the active selection's head to `offset` (shift-click).
    SelectionExtendTo { offset: CharOffset },
    /// Select the word at `offset` (double click).
    SelectWordAt { offset: CharOffset },
    /// Select the line at `offset` (triple click).
    SelectLineAt { offset: CharOffset },
    /// Update the in-progress drag selection to `offset` (mouse drag).
    SelectionUpdateTo { offset: CharOffset },
    /// Finish the in-progress drag selection (mouse up).
    SelectionEnd,
    /// Scroll the viewport by `rows` visual rows without moving the cursor
    /// (negative scrolls toward the top). Driven by the mouse wheel.
    Scroll { rows: isize },
}

// ─────────────────────────────────────────────────────────────────────────────
// View
// ─────────────────────────────────────────────────────────────────────────────

/// The `TuiView`-implementing entry point for the TUI prompt input.
pub struct TuiInputView {
    /// The backing code editor in char-cell (terminal) mode.
    model: ModelHandle<CodeEditorModel>,
    /// Single-entry kill buffer for `Ctrl+K` / `Ctrl+U` / `Ctrl+Y`.
    kill_buffer: KillBuffer,
    /// First visible visual row (0-indexed).
    scroll_offset: u32,
    /// Maximum number of visible rows before the input scrolls.
    max_visible_rows: u32,
    /// Whether a mouse drag-selection is in progress (set on mouse-down, cleared
    /// on mouse-up). Mirrors the GUI editor's `is_selecting`.
    is_selecting: bool,
}

impl Entity for TuiInputView {
    type Event = TuiInputViewEvent;
}

impl TuiInputView {
    /// Construct a new `TuiInputView` backed by `model` (must be in char-cell
    /// mode). The model carries the terminal width (set via
    /// [`CodeEditorModel::new_tui`]); the view does not keep its own copy.
    ///
    /// Subscribes to [`CodeEditorModelEvent::ContentChanged`] to trigger re-renders
    /// whenever the buffer changes from outside `handle_action`.
    pub fn new(model: ModelHandle<CodeEditorModel>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&model, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                ctx.notify();
            }
        });
        Self {
            model,
            kill_buffer: KillBuffer::default(),
            scroll_offset: 0,
            max_visible_rows: 6,
            is_selecting: false,
        }
    }

    /// Returns a handle to the backing [`CodeEditorModel`].
    pub fn model(&self) -> &ModelHandle<CodeEditorModel> {
        &self.model
    }

    /// Builds the concrete `TuiInputElement` for this frame. `render` wraps it in
    /// a `Box`; tests construct it directly to exercise mouse dispatch.
    ///
    /// Only width-independent state is gathered here; width-dependent layout (row
    /// wrapping, cursor placement, selection spans) happens later in
    /// `TuiInputElement::layout`, the first point that knows the terminal width.
    fn render_element(&self, ctx: &AppContext) -> TuiInputElement {
        let text = self.plain_text(ctx);
        let cursor_offset = self.cursor_offset(ctx);
        let sel_char_range = self.selection_range(ctx).map(|r| {
            let start = r.start.as_usize().saturating_sub(1);
            let end = r.end.as_usize().saturating_sub(1);
            (start, end)
        });

        TuiInputElement {
            model: self.model.clone(),
            text,
            cursor_offset,
            sel_char_range,
            scroll_offset: self.scroll_offset,
            max_visible_rows: self.max_visible_rows,
            is_selecting: self.is_selecting,
            column: TuiColumn::new(),
            cursor_col: 0,
            cursor_row_in_view: 0,
            cursor_visible: false,
            selected_spans: Vec::new(),
        }
    }
}

impl TuiView for TuiInputView {
    fn ui_name() -> &'static str {
        "TuiInputView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        Box::new(self.render_element(ctx))
    }
}

impl TypedActionView for TuiInputView {
    type Action = TuiInputAction;

    fn handle_action(&mut self, action: &TuiInputAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiInputAction::InsertChar(c) => {
                let s = c.to_string();
                self.model.update(ctx, |m, ctx| m.user_insert(&s, ctx));
            }
            TuiInputAction::InsertNewline => {
                self.model.update(ctx, |m, ctx| m.user_insert("\n", ctx));
            }
            TuiInputAction::Submit => self.submit(ctx),
            TuiInputAction::Backspace => {
                self.model.update(ctx, |m, ctx| m.backspace(ctx));
            }
            TuiInputAction::DeleteForward => {
                self.model.update(ctx, |m, ctx| {
                    m.delete(
                        warp_editor::selection::TextDirection::Forwards,
                        TextUnit::Character,
                        false,
                        ctx,
                    )
                });
            }
            TuiInputAction::MoveLeft => {
                self.model.update(ctx, |m, ctx| m.move_left(ctx));
            }
            TuiInputAction::MoveRight => {
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
            }
            TuiInputAction::MoveUp => {
                self.model.update(ctx, |m, ctx| m.move_up(ctx));
            }
            TuiInputAction::MoveDown => {
                self.model.update(ctx, |m, ctx| m.move_down(ctx));
            }
            TuiInputAction::MoveWordLeft => {
                self.model.update(ctx, |m, ctx| {
                    m.backward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::MoveWordRight => {
                self.model.update(ctx, |m, ctx| {
                    m.forward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::MoveToLineStart => {
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
            }
            TuiInputAction::MoveToLineEnd => {
                self.model.update(ctx, |m, ctx| m.move_to_line_end(ctx));
            }
            TuiInputAction::SelectLeft => {
                self.model.update(ctx, |m, ctx| m.select_left(ctx));
            }
            TuiInputAction::SelectRight => {
                self.model.update(ctx, |m, ctx| m.select_right(ctx));
            }
            TuiInputAction::SelectUp => {
                self.model.update(ctx, |m, ctx| m.select_up(ctx));
            }
            TuiInputAction::SelectDown => {
                self.model.update(ctx, |m, ctx| m.select_down(ctx));
            }
            TuiInputAction::SelectWordLeft => {
                self.model.update(ctx, |m, ctx| {
                    m.backward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::SelectWordRight => {
                self.model.update(ctx, |m, ctx| {
                    m.forward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    )
                });
            }
            TuiInputAction::SelectAll => {
                self.model.update(ctx, |m, ctx| m.select_all(ctx));
            }
            TuiInputAction::DeleteWordBackward => {
                self.model.update(ctx, |m, ctx| {
                    m.delete(
                        warp_editor::selection::TextDirection::Backwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    )
                });
            }
            TuiInputAction::DeleteWordForward => {
                self.model.update(ctx, |m, ctx| {
                    m.delete(
                        warp_editor::selection::TextDirection::Forwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    )
                });
            }
            TuiInputAction::KillToLineEnd => self.kill_to_line_end(ctx),
            TuiInputAction::KillToLineStart => self.kill_to_line_start(ctx),
            TuiInputAction::Yank => self.yank(ctx),
            TuiInputAction::Undo => {
                self.model.update(ctx, |m, ctx| m.undo(ctx));
            }
            TuiInputAction::Redo => {
                self.model.update(ctx, |m, ctx| m.redo(ctx));
            }
            TuiInputAction::SelectionStartAt { offset } => {
                self.is_selecting = true;
                self.model
                    .update(ctx, |m, ctx| m.select_at(*offset, false, ctx));
            }
            TuiInputAction::SelectionExtendTo { offset } => {
                self.model
                    .update(ctx, |m, ctx| m.set_last_selection_head(*offset, ctx));
            }
            TuiInputAction::SelectWordAt { offset } => {
                self.is_selecting = true;
                self.model
                    .update(ctx, |m, ctx| m.select_word_at(*offset, false, ctx));
            }
            TuiInputAction::SelectLineAt { offset } => {
                self.is_selecting = true;
                self.model
                    .update(ctx, |m, ctx| m.select_line_at(*offset, false, ctx));
            }
            TuiInputAction::SelectionUpdateTo { offset } => {
                if self.is_selecting {
                    self.model
                        .update(ctx, |m, ctx| m.update_pending_selection(*offset, ctx));
                }
            }
            TuiInputAction::SelectionEnd => {
                if self.is_selecting {
                    self.is_selecting = false;
                    self.model.update(ctx, |m, ctx| m.end_selection(ctx));
                }
            }
            TuiInputAction::Scroll { rows } => {
                // Wheel scrolling moves the viewport only; it must NOT snap back
                // to the cursor, so it returns early (skipping `scroll_to_cursor`).
                self.scroll_by(*rows, ctx);
                ctx.notify();
                return;
            }
        }

        let visible_rows = cmp::min(self.visual_line_count(ctx), self.max_visible_rows);
        self.scroll_to_cursor(visible_rows.max(1), ctx);
        ctx.notify();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// View-level TUI helpers
// ─────────────────────────────────────────────────────────────────────────────

impl TuiInputView {
    // ── Read helpers ──────────────────────────────────────────────────────────

    fn plain_text(&self, ctx: &AppContext) -> String {
        let inner = self.model.as_ref(ctx);
        let buffer = inner.content().as_ref(ctx);
        if buffer.is_empty() {
            return String::new();
        }
        buffer.text().into_string()
    }

    /// The terminal width (in cells) for char-cell layout, read from the backing
    /// model. The model is the single source of truth — it must hold the width
    /// for event-time navigation/scroll, so the view keeps no separate copy.
    fn terminal_width(&self, ctx: &AppContext) -> u16 {
        self.model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .char_cell()
            .map(|cc| cc.terminal_width())
            .unwrap_or(0)
    }

    fn cursor_offset(&self, ctx: &AppContext) -> CharOffset {
        self.model
            .as_ref(ctx)
            .selection_model()
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    fn selection_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let inner = self.model.as_ref(ctx);
        let sel = inner.buffer_selection_model().as_ref(ctx);
        let head = sel.first_selection_head();
        let tail = sel.first_selection_tail();
        if head == tail {
            None
        } else {
            let start = head.min(tail);
            let end = head.max(tail);
            Some(start..end)
        }
    }

    pub fn visual_line_count(&self, ctx: &AppContext) -> u32 {
        self.model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .max_line()
            .as_u32()
            .max(1)
    }

    // ── Scroll ────────────────────────────────────────────────────────────────

    fn scroll_to_cursor(&mut self, visible_rows: u32, ctx: &AppContext) {
        let cursor_offset = self.cursor_offset(ctx);
        let inner = self.model.as_ref(ctx);
        let render = inner.render_state().as_ref(ctx);
        // `offset_to_softwrap_point` is 0-based (see `char_cell_offset_to_softwrap_point`),
        // while the cursor is a 1-based `CharOffset`, so convert by subtracting 1.
        let cursor_char_index = CharOffset::from(cursor_offset.as_usize().saturating_sub(1));
        let pt = render.offset_to_softwrap_point(cursor_char_index);
        let cursor_row = pt.row();

        if cursor_row < self.scroll_offset {
            self.scroll_offset = cursor_row;
        } else if cursor_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = cursor_row.saturating_sub(visible_rows - 1);
        }
    }

    /// Scrolls the viewport by `rows` visual rows (negative scrolls toward the
    /// top), clamped to `[0, max_scroll]`. Independent of the cursor, so callers
    /// must not follow it with `scroll_to_cursor`.
    fn scroll_by(&mut self, rows: isize, ctx: &AppContext) {
        let lines = self.visual_line_count(ctx);
        let visible_rows = cmp::min(lines, self.max_visible_rows).max(1);
        let max_scroll = lines.saturating_sub(visible_rows) as isize;
        let new_offset = (self.scroll_offset as isize + rows).clamp(0, max_scroll);
        self.scroll_offset = new_offset as u32;
    }

    // ── Submit ────────────────────────────────────────────────────────────────

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.plain_text(ctx);
        ctx.emit(TuiInputViewEvent::Submitted(text));
        self.model.update(ctx, |m, ctx| m.clear_buffer(ctx));
        self.scroll_offset = 0;
    }

    // ── Kill / yank ───────────────────────────────────────────────────────────

    fn kill_to_line_end(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(range) = self.range_to_visual_line_end(ctx) {
            let killed = self
                .model
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.model.update(ctx, |inner, ctx| {
                use warp_editor::content::buffer::{BufferEditAction, EditOrigin};
                inner.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                            BufferEditAction::Delete(vec1::vec1![range]),
                            EditOrigin::UserInitiated,
                            inner.buffer_selection_model().clone(),
                            ctx,
                        );
                    },
                    ctx,
                );
            });
        }
    }

    fn kill_to_line_start(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(range) = self.range_from_visual_line_start(ctx) {
            let killed = self
                .model
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.model.update(ctx, |inner, ctx| {
                use warp_editor::content::buffer::{BufferEditAction, EditOrigin};
                inner.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                            BufferEditAction::Delete(vec1::vec1![range]),
                            EditOrigin::UserInitiated,
                            inner.buffer_selection_model().clone(),
                            ctx,
                        );
                    },
                    ctx,
                );
            });
        }
    }

    fn yank(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(text) = self.kill_buffer.yank().map(str::to_owned) {
            self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
        }
    }

    // ── Kill range helpers ────────────────────────────────────────────────────
    //
    // These compute ranges purely from the plain-text string using the same
    // char-cell soft-wrap arithmetic as `build_visual_rows`, avoiding any
    // dependency on the render-state softwrap API (which uses 0-indexed offsets
    // that differ from the buffer's 1-indexed `CharOffset` convention).

    fn range_to_visual_line_end(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let text = self.plain_text(ctx);
        // `cursor_offset` is a 1-indexed gap position (gap 1 sits before the
        // first character); the buffer's `text_in_range` / `Delete` use those
        // same coordinates, so the kill range starts exactly at `cursor_gap`.
        let cursor_gap = self.cursor_offset(ctx).as_usize();
        // 0-based index of the character at the cursor, for the pure helpers.
        let cursor_idx = cursor_gap.saturating_sub(1);
        let end_idx = visual_line_end_exclusive(&text, cursor_idx, self.terminal_width(ctx));
        if end_idx <= cursor_idx {
            return None;
        }
        // `text[i]` lives at gap `i + 1`, so the exclusive end gap is `end_idx + 1`.
        Some(CharOffset::from(cursor_gap)..CharOffset::from(end_idx + 1))
    }

    fn range_from_visual_line_start(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let text = self.plain_text(ctx);
        let cursor_gap = self.cursor_offset(ctx).as_usize();
        let cursor_idx = cursor_gap.saturating_sub(1);
        let start_idx = visual_line_start_idx(&text, cursor_idx, self.terminal_width(ctx));
        if start_idx >= cursor_idx {
            return None;
        }
        Some(CharOffset::from(start_idx + 1)..CharOffset::from(cursor_gap))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Kill range pure-text helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The `(logical_line_start, logical_line_end)` char indices bounding the
/// logical line that contains `cursor_idx` (end excludes the trailing `\n`).
fn logical_line_bounds(chars: &[char], cursor_idx: usize) -> (usize, usize) {
    let ci = cursor_idx.min(chars.len());
    let start = chars[..ci]
        .iter()
        .rposition(|&c| c == '\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let end = chars[ci..]
        .iter()
        .position(|&c| c == '\n')
        .map(|p| ci + p)
        .unwrap_or(chars.len());
    (start, end)
}

/// For the logical line containing `cursor_idx`, returns
/// `(logical_line_start, pos_in_line, row_starts, line_len)` using width-aware
/// wrapping. `row_starts` are 0-based char indices within the line where each
/// visual row begins; `line_len` is the line's char count (excluding the
/// trailing `\n`). Shared by the two kill-range helpers below.
fn visual_line_segments(
    text: &str,
    cursor_idx: usize,
    terminal_width: u16,
) -> (usize, usize, Vec<usize>, usize) {
    let chars: Vec<char> = text.chars().collect();
    let (logical_line_start, logical_line_end) = logical_line_bounds(&chars, cursor_idx);
    let line_widths: Vec<u8> = chars[logical_line_start..logical_line_end]
        .iter()
        .map(|&c| char_cell_display_width(c) as u8)
        .collect();
    let row_starts = char_cell_line_row_starts(&line_widths, terminal_width);
    let pos_in_line = cursor_idx
        .min(chars.len())
        .saturating_sub(logical_line_start);
    (
        logical_line_start,
        pos_in_line,
        row_starts,
        line_widths.len(),
    )
}

/// Returns the 0-based exclusive end index of the visual line segment at
/// `cursor_idx`. Does NOT include any trailing `\n`. Width-aware: the segment
/// boundaries follow the same display-width wrapping as the rendered rows.
fn visual_line_end_exclusive(text: &str, cursor_idx: usize, terminal_width: u16) -> usize {
    let (logical_line_start, pos_in_line, row_starts, line_len) =
        visual_line_segments(text, cursor_idx, terminal_width);
    let row = row_starts
        .partition_point(|&s| s <= pos_in_line)
        .saturating_sub(1);
    // Exclusive end = start of the next visual row, or the logical line's end
    // (excluding the '\n') for the final row.
    let seg_end_in_line = row_starts.get(row + 1).copied().unwrap_or(line_len);
    logical_line_start + seg_end_in_line
}

/// Returns the 0-based start index of the visual line segment at `cursor_idx`.
/// Width-aware (see [`visual_line_end_exclusive`]).
fn visual_line_start_idx(text: &str, cursor_idx: usize, terminal_width: u16) -> usize {
    let (logical_line_start, pos_in_line, row_starts, _line_len) =
        visual_line_segments(text, cursor_idx, terminal_width);
    let row = row_starts
        .partition_point(|&s| s <= pos_in_line)
        .saturating_sub(1);
    logical_line_start + row_starts[row]
}

// ─────────────────────────────────────────────────────────────────────────────
// TuiInputElement — element returned from render()
// ─────────────────────────────────────────────────────────────────────────────

/// The element returned from [`TuiInputView::render`].
///
/// `render` captures only width-independent state; the row wrapping, cursor
/// placement, and selection spans are computed in [`TuiElement::layout`], the
/// first point that knows the terminal width (from the layout constraint). This
/// mirrors the GUI, where the element computes geometry in `layout`.
struct TuiInputElement {
    /// Backing model: used during layout to push the terminal width and read the
    /// char-cell visual line count at that width.
    model: ModelHandle<CodeEditorModel>,
    /// Plain-text content captured at render time.
    text: String,
    /// Cursor gap offset (1-based) captured at render time.
    cursor_offset: CharOffset,
    /// Selection as 0-based `(start, end)` char indices, if any.
    sel_char_range: Option<(usize, usize)>,
    /// First visible visual row (0-indexed).
    scroll_offset: u32,
    /// Maximum number of visible rows before the input scrolls.
    max_visible_rows: u32,
    /// Whether a mouse drag-selection is in progress (captured from the view at
    /// render time); gates drag/up handling in `dispatch_event`.
    is_selecting: bool,
    /// Visible rows, built during `layout`.
    column: TuiColumn,
    /// The cursor's 0-based column within the visible area (set during `layout`).
    cursor_col: u16,
    /// The cursor's 0-based row within the visible area (set during `layout`).
    cursor_row_in_view: u16,
    /// Whether the cursor's visual row falls within the scrolled viewport; when
    /// false (e.g. after wheel-scrolling away) no terminal cursor is drawn.
    cursor_visible: bool,
    /// Selected spans `(row_in_view, start_col, exclusive_end_col)` (set during `layout`).
    selected_spans: Vec<(u16, u16, u16)>,
}

impl TuiInputElement {
    /// Builds the visible rows, cursor position, and selection spans for
    /// `terminal_width`, storing them for `render`/`cursor_position`.
    fn build(&mut self, terminal_width: u16, visible_rows: u32) {
        let (cursor_visual_row, cursor_col) =
            char_cell_cursor_pos(&self.text, self.cursor_offset, terminal_width);
        let cursor_row_in_view = cursor_visual_row.saturating_sub(self.scroll_offset);
        let cursor_visible = cursor_visual_row >= self.scroll_offset
            && cursor_visual_row < self.scroll_offset + visible_rows;

        let rows_with_offsets = build_visual_rows_with_offsets(&self.text, terminal_width);
        let visible_start = self.scroll_offset as usize;
        let visible_end =
            (self.scroll_offset as usize + visible_rows as usize).min(rows_with_offsets.len());
        let visible_rows_slice: Vec<(String, usize)> = if visible_start < rows_with_offsets.len() {
            rows_with_offsets[visible_start..visible_end].to_vec()
        } else {
            vec![(String::new(), 0)]
        };

        // Selection spans per visible row. Selection offsets are char indices;
        // terminal highlighting works in display columns, so convert via each
        // char's display width (wide chars span two columns).
        let mut selected_spans: Vec<(u16, u16, u16)> = Vec::new();
        if let Some((sel_start, sel_end)) = self.sel_char_range {
            if sel_start < sel_end {
                for (vis_idx, (row_text, row_char_start)) in visible_rows_slice.iter().enumerate() {
                    let row_chars: Vec<char> = row_text.chars().collect();
                    let row_len = row_chars.len();
                    let row_char_end = row_char_start + row_len;
                    if sel_end > *row_char_start && sel_start < row_char_end {
                        let start_char = sel_start.saturating_sub(*row_char_start);
                        let end_char = (sel_end - row_char_start).min(row_len);
                        let disp_start: usize = row_chars[..start_char]
                            .iter()
                            .map(|&c| char_cell_display_width(c))
                            .sum();
                        let disp_end: usize = row_chars[..end_char]
                            .iter()
                            .map(|&c| char_cell_display_width(c))
                            .sum();
                        selected_spans.push((vis_idx as u16, disp_start as u16, disp_end as u16));
                    }
                }
            }
        }

        let mut column = TuiColumn::new();
        for (row_text, _) in &visible_rows_slice {
            // An empty `TuiText` lays out to zero rows, which would collapse the
            // row and clip the cursor (or following rows) off the column. Render
            // a single space so every visual row keeps a height of exactly one.
            let row_display = if row_text.is_empty() {
                " ".to_string()
            } else {
                row_text.clone()
            };
            column = column.with_child(Box::new(TuiText::new(row_display).truncate()));
        }

        self.column = column;
        self.cursor_col = cursor_col as u16;
        self.cursor_row_in_view = cursor_row_in_view as u16;
        self.cursor_visible = cursor_visible;
        self.selected_spans = selected_spans;
    }

    /// Maps a terminal cell `position` to the 1-based buffer [`CharOffset`] under
    /// it (the gap the cursor should move to for a click/drag at that cell).
    ///
    /// The mouse reports an absolute terminal cell, so getting to a buffer offset
    /// crosses three coordinate spaces:
    ///   1. screen cell (`position`) -> row/col relative to the input's `area`,
    ///   2. + `scroll_offset` -> the buffer's *visual* row (undoes scrolling),
    ///   3. (visual row, col) -> char offset via the char-cell soft-wrap map.
    ///
    /// Points outside the input's vertical bounds are intentionally *not* clamped
    /// to the viewport: a point above the input maps toward row 0 and a point
    /// below it maps past the last visible row (bounded by the buffer's last
    /// visual row), so a drag that leaves the input drives auto-scroll.
    fn offset_at(&self, position: TuiPoint, area: TuiRect, app: &AppContext) -> CharOffset {
        let inner = self.model.as_ref(app);
        let render = inner.render_state().as_ref(app);

        // Step 1: row of the pointer within the input, where 0 is the input's top
        // row. Signed so a point *above* the input (`position.y < area.y`) stays
        // negative instead of wrapping around at 0.
        let row_in_view = i64::from(position.y) - i64::from(area.y);
        // Step 2: the input shows buffer visual rows starting at `scroll_offset`,
        // so the buffer row under the pointer is `scroll_offset + row_in_view`
        // (floored at the first row).
        let visual_row = (i64::from(self.scroll_offset) + row_in_view).max(0) as u32;
        // ...and capped at the last real visual row, so a drag below the text
        // resolves to the buffer's end rather than past it.
        let last_row = render.max_line().as_u32().max(1).saturating_sub(1);
        let visual_row = visual_row.min(last_row);

        // Column within that row, in display cells (0 is the input's left edge).
        let col = position.x.saturating_sub(area.x);

        // Step 3: resolve (visual_row, col) to a char offset. The soft-wrap map
        // clamps the column to the row's end and is 0-based, while the buffer
        // uses 1-based gap offsets (see the kill-range helpers), so re-add 1.
        let point = SoftWrapPoint::new(visual_row, ColumnUnit::Chars(col));
        let zero_based = render.softwrap_point_to_offset(point);
        CharOffset::from(zero_based.as_usize() + 1)
    }

    /// Maps a mouse `event` to the [`TuiInputAction`] it should dispatch, or
    /// `None` when the event should be ignored (a press outside the input, a
    /// drag/up with no selection in progress, or a non-mouse event).
    ///
    /// Mirrors the GUI's `left_mouse_down`/`dragged`/`up` mapping: click count 1
    /// starts a selection (shift extends), 2 selects a word, 3 selects a line;
    /// drag updates the pending selection and up ends it.
    fn mouse_action(
        &self,
        event: &TuiEvent,
        area: TuiRect,
        app: &AppContext,
    ) -> Option<TuiInputAction> {
        match event {
            TuiEvent::LeftMouseDown {
                position,
                modifiers,
                click_count,
                is_first_mouse,
            } => {
                // The focus-bringing first click has no matching mouse-up, and a
                // press outside the input must not start a selection.
                if *is_first_mouse || !area.contains_point(*position) {
                    return None;
                }
                let offset = self.offset_at(*position, area, app);
                Some(match *click_count {
                    0 | 1 if modifiers.shift => TuiInputAction::SelectionExtendTo { offset },
                    0 | 1 => TuiInputAction::SelectionStartAt { offset },
                    2 => TuiInputAction::SelectWordAt { offset },
                    _ => TuiInputAction::SelectLineAt { offset },
                })
            }
            // Drags continue even outside the input's bounds (drag-to-scroll),
            // but only while a selection that began inside it is active.
            TuiEvent::LeftMouseDragged { position, .. } if self.is_selecting => {
                Some(TuiInputAction::SelectionUpdateTo {
                    offset: self.offset_at(*position, area, app),
                })
            }
            TuiEvent::LeftMouseUp { .. } if self.is_selecting => Some(TuiInputAction::SelectionEnd),
            // Mouse wheel over the input scrolls the viewport (cursor unmoved).
            TuiEvent::ScrollWheel {
                position, delta, ..
            } if area.contains_point(*position) => Some(TuiInputAction::Scroll {
                // crossterm reports ScrollUp as +1 row / ScrollDown as -1; negate
                // so wheel-up scrolls toward the top (matches `TuiScrollable`).
                rows: -(delta.1 * WHEEL_STEP),
            }),
            _ => None,
        }
    }
}

impl TuiElement for TuiInputElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        // The layout constraint is the first place the real terminal width is
        // known. Push it onto the model (interior-mutable) so event-time
        // navigation/scroll read the right width, then build the rows at that
        // width — mirroring how the GUI computes geometry during layout.
        let terminal_width = constraint.constrain_width(constraint.max.width);
        let render_state = self.model.as_ref(app).render_state().clone();
        if let Some(cc) = render_state.as_ref(app).char_cell() {
            cc.set_terminal_width(terminal_width);
        }
        let visual_line_count = render_state.as_ref(app).max_line().as_u32().max(1);
        let visible_rows = cmp::min(visual_line_count, self.max_visible_rows);

        self.build(terminal_width, visible_rows);
        self.column.layout(constraint, ctx, app)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        self.column.render(area, buffer, ctx);
        if !self.selected_spans.is_empty() {
            let reversed = TuiStyle::default().add_modifier(Modifier::REVERSED);
            for &(row_in_view, start_col, end_col) in &self.selected_spans {
                let y = area.y.saturating_add(row_in_view);
                let x = area.x.saturating_add(start_col);
                let width = end_col.saturating_sub(start_col);
                if y < area.y + area.height && width > 0 {
                    let sel_rect =
                        TuiRect::new(x, y, width.min(area.width.saturating_sub(start_col)), 1);
                    buffer.set_style(sel_rect, reversed);
                }
            }
        }
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        if !self.cursor_visible
            || self.cursor_col >= area.width
            || self.cursor_row_in_view >= area.height
        {
            return None;
        }
        Some((self.cursor_col, self.cursor_row_in_view))
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if self.column.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        if let Some(action) = self.mouse_action(event, area, app) {
            event_ctx.dispatch_typed_action(action);
            return true;
        }

        if let TuiEvent::KeyDown {
            keystroke, chars, ..
        } = event
        {
            let ctrl = keystroke.ctrl;
            let alt = keystroke.alt;
            let shift = keystroke.shift;
            let key = keystroke.key.as_str();

            let action: Option<TuiInputAction> = match (ctrl, alt, shift, key) {
                // ── Submit / newline ─────────────────────────────────────────────
                (false, false, false, "enter") => Some(TuiInputAction::Submit),
                (false, false, true, "enter") => Some(TuiInputAction::InsertNewline),
                (true, false, false, "j") => Some(TuiInputAction::InsertNewline),
                (false, true, false, "enter") => Some(TuiInputAction::InsertNewline),
                // ── Deletion ─────────────────────────────────────────────────────
                (false, false, _, "backspace") => Some(TuiInputAction::Backspace),
                (true, false, false, "h") => Some(TuiInputAction::Backspace),
                (false, false, false, "delete") => Some(TuiInputAction::DeleteForward),
                (true, false, false, "d") => Some(TuiInputAction::DeleteForward),
                (true, false, false, "w") => Some(TuiInputAction::DeleteWordBackward),
                (true, false, false, "backspace") => Some(TuiInputAction::DeleteWordBackward),
                (false, true, false, "backspace") => Some(TuiInputAction::DeleteWordBackward),
                (false, true, false, "d") => Some(TuiInputAction::DeleteWordForward),
                (false, true, false, "delete") => Some(TuiInputAction::DeleteWordForward),
                (true, false, false, "delete") => Some(TuiInputAction::DeleteWordForward),
                // ── Cursor movement ───────────────────────────────────────────────
                (false, false, false, "left") | (true, false, false, "b") => {
                    Some(TuiInputAction::MoveLeft)
                }
                (false, false, false, "right") | (true, false, false, "f") => {
                    Some(TuiInputAction::MoveRight)
                }
                (false, false, false, "up") | (true, false, false, "p") => {
                    Some(TuiInputAction::MoveUp)
                }
                (false, false, false, "down") | (true, false, false, "n") => {
                    Some(TuiInputAction::MoveDown)
                }
                (false, true, false, "left")
                | (false, true, false, "b")
                | (true, false, false, "left") => Some(TuiInputAction::MoveWordLeft),
                (false, true, false, "right")
                | (false, true, false, "f")
                | (true, false, false, "right") => Some(TuiInputAction::MoveWordRight),
                (false, false, false, "home") | (true, false, false, "a") => {
                    Some(TuiInputAction::MoveToLineStart)
                }
                (false, false, false, "end") | (true, false, false, "e") => {
                    Some(TuiInputAction::MoveToLineEnd)
                }
                // ── Selection ────────────────────────────────────────────────────
                (false, false, true, "left") => Some(TuiInputAction::SelectLeft),
                (false, false, true, "right") => Some(TuiInputAction::SelectRight),
                (false, false, true, "up") => Some(TuiInputAction::SelectUp),
                (false, false, true, "down") => Some(TuiInputAction::SelectDown),
                (true, false, true, "a") => Some(TuiInputAction::SelectAll),
                // Word-wise selection: Ctrl+Shift+Arrow (and Alt+Shift+Arrow).
                (true, false, true, "left") | (false, true, true, "left") => {
                    Some(TuiInputAction::SelectWordLeft)
                }
                (true, false, true, "right") | (false, true, true, "right") => {
                    Some(TuiInputAction::SelectWordRight)
                }
                // ── Kill / yank ───────────────────────────────────────────────────
                (true, false, false, "k") => Some(TuiInputAction::KillToLineEnd),
                (true, false, false, "u") => Some(TuiInputAction::KillToLineStart),
                (true, false, false, "y") => Some(TuiInputAction::Yank),
                // ── Undo / redo ───────────────────────────────────────────────────
                (true, false, false, "z") => Some(TuiInputAction::Undo),
                (true, false, true, "z") => Some(TuiInputAction::Redo),
                // ── Printable character ───────────────────────────────────────────
                (false, false, _, _) if !chars.is_empty() && !ctrl && !alt => {
                    chars.chars().next().map(TuiInputAction::InsertChar)
                }
                _ => None,
            };

            if let Some(action) = action {
                event_ctx.dispatch_typed_action(action);
                return true;
            }
        }

        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Char-cell helpers (pure functions, no model dependency)
// ─────────────────────────────────────────────────────────────────────────────

/// Splits `text` into visual rows, returning `(row_text, char_start_offset)` pairs.
///
/// Wrapping is display-width aware (wide CJK/emoji span two columns, combining
/// marks zero), sharing the same wrapping rule as the editor's char-cell layout
/// via [`char_cell_line_row_starts`].
pub fn build_visual_rows_with_offsets(text: &str, terminal_width: u16) -> Vec<(String, usize)> {
    let mut rows: Vec<(String, usize)> = Vec::new();
    let mut global_char_offset: usize = 0;

    for logical_line in text.split('\n') {
        let line_char_start = global_char_offset;
        let chars: Vec<char> = logical_line.chars().collect();
        let line_len = chars.len();
        let widths: Vec<u8> = chars
            .iter()
            .map(|&c| char_cell_display_width(c) as u8)
            .collect();
        let row_starts = char_cell_line_row_starts(&widths, terminal_width);
        for r in 0..row_starts.len() {
            let start = row_starts[r];
            let end = row_starts.get(r + 1).copied().unwrap_or(line_len);
            rows.push((chars[start..end].iter().collect(), line_char_start + start));
        }
        global_char_offset += line_len + 1;
    }

    if rows.is_empty() {
        rows.push((String::new(), 0));
    }
    rows
}

/// Splits `text` into visual rows, display-width aware. Thin wrapper over
/// [`build_visual_rows_with_offsets`] that drops the per-row char offsets.
pub fn build_visual_rows(text: &str, terminal_width: u16) -> Vec<String> {
    build_visual_rows_with_offsets(text, terminal_width)
        .into_iter()
        .map(|(row_text, _)| row_text)
        .collect()
}

/// Returns `(visual_row, visual_col)` for `cursor_offset` in char-cell
/// coordinates, where `visual_col` is a display column (wide chars span two).
pub fn char_cell_cursor_pos(
    text: &str,
    cursor_offset: CharOffset,
    terminal_width: u16,
) -> (u32, u32) {
    let cursor_char_idx = cursor_offset.as_usize().saturating_sub(1);
    let mut visual_row: u32 = 0;
    let mut chars_so_far: usize = 0;

    for logical_line in text.split('\n') {
        let chars: Vec<char> = logical_line.chars().collect();
        let line_len = chars.len();
        let widths: Vec<u8> = chars
            .iter()
            .map(|&c| char_cell_display_width(c) as u8)
            .collect();
        let line_end_exclusive = chars_so_far + line_len;

        if cursor_char_idx <= line_end_exclusive {
            let char_in_line = cursor_char_idx.saturating_sub(chars_so_far);
            let (row_in_line, col) =
                char_cell_line_gap_position(&widths, terminal_width, char_in_line);
            return (visual_row + row_in_line, col as u32);
        }

        visual_row += char_cell_line_row_starts(&widths, terminal_width).len() as u32;
        chars_so_far = line_end_exclusive + 1;
    }

    (visual_row, 0)
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
