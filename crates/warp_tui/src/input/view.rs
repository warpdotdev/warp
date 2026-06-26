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

use num_traits::SaturatingSub;
use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::TextUnit;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiColumn, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiParentElement, TuiRect, TuiSize, TuiStyle, TuiText,
};
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewContext};

use super::kill_buffer::KillBuffer;

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
    /// Terminal width in columns. Kept in sync with the model's `CharCellState`.
    terminal_width: u16,
    /// Maximum number of visible rows before the input scrolls.
    max_visible_rows: u32,
}

impl Entity for TuiInputView {
    type Event = TuiInputViewEvent;
}

impl TuiInputView {
    /// Construct a new `TuiInputView` backed by `model` (must be in char-cell mode).
    ///
    /// Subscribes to [`CodeEditorModelEvent::ContentChanged`] to trigger re-renders
    /// whenever the buffer changes from outside `handle_action`.
    pub fn new(
        model: ModelHandle<CodeEditorModel>,
        terminal_width: u16,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&model, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                ctx.notify();
            }
        });
        Self {
            model,
            kill_buffer: KillBuffer::default(),
            scroll_offset: 0,
            terminal_width,
            max_visible_rows: 6,
        }
    }

    /// Returns a handle to the backing [`CodeEditorModel`].
    pub fn model(&self) -> &ModelHandle<CodeEditorModel> {
        &self.model
    }

    /// Update the terminal width, resizing the char-cell layout on the model.
    pub fn set_terminal_width(&mut self, width: u16, ctx: &mut ViewContext<Self>) {
        if self.terminal_width == width {
            return;
        }
        self.terminal_width = width;
        self.model
            .update(ctx, |m, ctx| m.set_tui_terminal_width(width, ctx));
    }
}

impl TuiView for TuiInputView {
    fn ui_name() -> &'static str {
        "TuiInputView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        // ── Gather state ───────────────────────────────────────────────────────
        let text = self.plain_text(ctx);
        let terminal_width = self.terminal_width;
        let scroll_offset = self.scroll_offset;
        let visible_rows = cmp::min(self.visual_line_count(ctx), self.max_visible_rows);

        let cursor_offset = self.cursor_offset(ctx);
        let (cursor_visual_row, cursor_col) =
            char_cell_cursor_pos(&text, cursor_offset, terminal_width);
        let cursor_row_in_view = cursor_visual_row.saturating_sub(scroll_offset);

        let sel_char_range = self.selection_range(ctx).map(|r| {
            let start = r.start.as_usize().saturating_sub(1);
            let end = r.end.as_usize().saturating_sub(1);
            (start, end)
        });

        // ── Build visible rows ─────────────────────────────────────────────────
        let rows_with_offsets = build_visual_rows_with_offsets(&text, terminal_width);
        let visible_start = scroll_offset as usize;
        let visible_end =
            (scroll_offset as usize + visible_rows as usize).min(rows_with_offsets.len());
        let visible_rows_slice: Vec<(String, usize)> = if visible_start < rows_with_offsets.len() {
            rows_with_offsets[visible_start..visible_end].to_vec()
        } else {
            vec![(String::new(), 0)]
        };

        // ── Selection spans per visible row ────────────────────────────────────
        let mut selected_spans: Vec<(u16, u16, u16)> = Vec::new();
        if let Some((sel_start, sel_end)) = sel_char_range {
            if sel_start < sel_end {
                for (vis_idx, (row_text, row_char_start)) in visible_rows_slice.iter().enumerate() {
                    let row_len = row_text.chars().count();
                    let row_char_end = row_char_start + row_len;
                    if sel_end > *row_char_start && sel_start < row_char_end {
                        let span_start = sel_start.saturating_sub(*row_char_start);
                        let span_end = (sel_end - row_char_start).min(row_len);
                        selected_spans.push((vis_idx as u16, span_start as u16, span_end as u16));
                    }
                }
            }
        }

        // ── Assemble column ────────────────────────────────────────────────────
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);
        let mut column = TuiColumn::new();
        for (row_idx, (row_text, _)) in visible_rows_slice.iter().enumerate() {
            let style = if row_idx as u32 == cursor_row_in_view {
                TuiStyle::default()
            } else {
                dim
            };
            column = column.with_child(Box::new(
                TuiText::new(row_text.clone()).with_style(style).truncate(),
            ));
        }

        Box::new(TuiInputElement {
            column,
            cursor_col: cursor_col as u16,
            cursor_row_in_view: cursor_row_in_view as u16,
            selected_spans,
        })
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
        let adjusted = cursor_offset.saturating_sub(&CharOffset::from(1));
        let pt = render.offset_to_softwrap_point(adjusted);
        let cursor_row = pt.row();

        if cursor_row < self.scroll_offset {
            self.scroll_offset = cursor_row;
        } else if cursor_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = cursor_row.saturating_sub(visible_rows - 1);
        }
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

    fn range_to_visual_line_end(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let cursor = self.cursor_offset(ctx);
        let inner = self.model.as_ref(ctx);
        let render = inner.render_state().as_ref(ctx);
        let max = inner.content().as_ref(ctx).max_charoffset();
        let adjusted = cursor.saturating_sub(&CharOffset::from(1));
        let pt = render.offset_to_softwrap_point(adjusted);
        let line_end = render
            .softwrap_point_to_offset(warp_editor::render::model::SoftWrapPoint::new(
                pt.row() + 1,
                warp_editor::render::model::ColumnUnit::chars_zero(),
            ))
            .saturating_sub(&CharOffset::from(1))
            .min(max);
        if line_end <= cursor {
            None
        } else {
            Some(cursor..line_end)
        }
    }

    fn range_from_visual_line_start(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let cursor = self.cursor_offset(ctx);
        let inner = self.model.as_ref(ctx);
        let render = inner.render_state().as_ref(ctx);
        let adjusted = cursor.saturating_sub(&CharOffset::from(1));
        let pt = render.offset_to_softwrap_point(adjusted);
        let line_start =
            render.softwrap_point_to_offset(warp_editor::render::model::SoftWrapPoint::new(
                pt.row(),
                warp_editor::render::model::ColumnUnit::chars_zero(),
            ));
        if line_start >= cursor {
            None
        } else {
            Some(line_start..cursor)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TuiInputElement — element returned from render()
// ─────────────────────────────────────────────────────────────────────────────

/// The element returned from [`TuiInputView::render`].
struct TuiInputElement {
    column: TuiColumn,
    /// The cursor's 0-based column within the visible area.
    cursor_col: u16,
    /// The cursor's 0-based row within the visible area (after scroll_offset subtraction).
    cursor_row_in_view: u16,
    /// Selected spans: `(row_in_view, start_col, exclusive_end_col)`.
    selected_spans: Vec<(u16, u16, u16)>,
}

impl TuiElement for TuiInputElement {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        self.column.layout(constraint, ctx)
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
        if self.cursor_col >= area.width || self.cursor_row_in_view >= area.height {
            return None;
        }
        Some((self.cursor_col, self.cursor_row_in_view))
    }

    fn dispatch_event(
        &mut self,
        event: &warpui_core::Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        if self.column.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        if let warpui_core::Event::KeyDown {
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
pub fn build_visual_rows_with_offsets(text: &str, terminal_width: u16) -> Vec<(String, usize)> {
    let w = terminal_width as usize;
    let mut rows: Vec<(String, usize)> = Vec::new();
    let mut global_char_offset: usize = 0;

    for logical_line in text.split('\n') {
        let line_char_start = global_char_offset;
        let line_len = logical_line.chars().count();

        if logical_line.is_empty() {
            rows.push((String::new(), line_char_start));
        } else if w == 0 {
            rows.push((logical_line.to_owned(), line_char_start));
        } else {
            let chars: Vec<char> = logical_line.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let end = (start + w).min(chars.len());
                rows.push((chars[start..end].iter().collect(), line_char_start + start));
                start = end;
            }
        }
        global_char_offset += line_len + 1;
    }

    if rows.is_empty() {
        rows.push((String::new(), 0));
    }
    rows
}

/// Splits `text` into visual rows of at most `terminal_width` chars each.
pub fn build_visual_rows(text: &str, terminal_width: u16) -> Vec<String> {
    let w = terminal_width as usize;
    let mut rows = Vec::new();

    for logical_line in text.split('\n') {
        if logical_line.is_empty() {
            rows.push(String::new());
        } else if w == 0 {
            rows.push(logical_line.to_owned());
        } else {
            let chars: Vec<char> = logical_line.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let end = (start + w).min(chars.len());
                rows.push(chars[start..end].iter().collect());
                start = end;
            }
        }
    }

    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Returns `(visual_row, visual_col)` for `cursor_offset` in char-cell coordinates.
pub fn char_cell_cursor_pos(
    text: &str,
    cursor_offset: CharOffset,
    terminal_width: u16,
) -> (u32, u32) {
    let cursor_char_idx = cursor_offset.as_usize().saturating_sub(1);
    let w = terminal_width as usize;
    let mut visual_row: u32 = 0;
    let mut chars_so_far: usize = 0;

    for logical_line in text.split('\n') {
        let line_len = logical_line.chars().count();
        let line_end_exclusive = chars_so_far + line_len;

        if cursor_char_idx <= line_end_exclusive {
            let offset_in_line = cursor_char_idx.saturating_sub(chars_so_far);
            let (row_in_line, col) = if w == 0 {
                (0, offset_in_line as u32)
            } else {
                ((offset_in_line / w) as u32, (offset_in_line % w) as u32)
            };
            return (visual_row + row_in_line, col);
        }

        let line_rows = if w == 0 || line_len == 0 {
            1
        } else {
            line_len.div_ceil(w).max(1)
        };
        visual_row += line_rows as u32;
        chars_so_far = line_end_exclusive + 1;
    }

    (visual_row, 0)
}
