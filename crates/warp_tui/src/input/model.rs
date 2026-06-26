//! [`TuiEditorModel`] — TUI editor model backed by [`CodeEditorModel`].
//!
//! `TuiEditorModel` wraps a [`ModelHandle<CodeEditorModel>`] constructed via
//! [`CodeEditorModel::new_tui`] (CharCell mode). This gives the TUI editor all of
//! `CodeEditorModel`'s features — vim, syntax highlighting, diff, hidden lines —
//! for free, while the wrapper itself holds the TUI-specific state that doesn't
//! belong on the editor: the kill buffer and scroll offset.
//!
//! See `specs/tui-input-view/TECH.md` for architecture rationale.

use std::ops::Range;

use num_traits::SaturatingSub;
use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warpui_core::{AppContext, Entity, ModelAsRef, ModelContext, ModelHandle};

use super::kill_buffer::KillBuffer;

// ─────────────────────────────────────────────────────────────────────────────
// Events
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by [`TuiEditorModel`].
#[derive(Debug, Clone)]
pub enum TuiEditorModelEvent {
    /// The buffer text, cursor position, or selection changed.
    Changed,
    /// The user submitted the current input. Contains the final text.
    Submit(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Model
// ─────────────────────────────────────────────────────────────────────────────

/// The editing model for the TUI input view.
///
/// Wraps a [`CodeEditorModel`] constructed in `LayoutMode::CharCell` so all
/// editing features (insert, backspace, word movement, undo/redo, vim, syntax,
/// diff, hidden lines) are inherited from the shared `CodeEditorModel`
/// infrastructure. TUI-specific state (kill buffer, scroll offset) lives here.
pub struct TuiEditorModel {
    /// The backing code editor in char-cell mode.
    inner: ModelHandle<CodeEditorModel>,
    /// Single-entry kill buffer for `Ctrl+K` / `Ctrl+U` / `Ctrl+Y`.
    pub kill_buffer: KillBuffer,
    /// Terminal width in columns — owned here for convenience; the actual
    /// `CharCellState` on the render state is the source of truth.
    pub terminal_width: u16,
    /// First visible visual row (0-indexed).
    pub scroll_offset: u32,
}

impl Entity for TuiEditorModel {
    type Event = TuiEditorModelEvent;
}

// ─────────────────────────────────────────────────────────────────────────────
// Construction
// ─────────────────────────────────────────────────────────────────────────────

impl TuiEditorModel {
    pub fn new(terminal_width: u16, ctx: &mut ModelContext<Self>) -> Self {
        let inner = ctx.add_model(|ctx| CodeEditorModel::new_tui(terminal_width, ctx));

        // Re-emit content changes as TuiEditorModelEvent::Changed so the view
        // can subscribe to a single, stable event type.
        ctx.subscribe_to_model(&inner, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                ctx.emit(TuiEditorModelEvent::Changed);
            }
        });

        Self {
            inner,
            kill_buffer: KillBuffer::default(),
            terminal_width,
            scroll_offset: 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TUI-specific API (delegates to inner CodeEditorModel)
// ─────────────────────────────────────────────────────────────────────────────

impl TuiEditorModel {
    // ── Cursor / visual queries ───────────────────────────────────────────────

    pub fn visual_line_count(&self, ctx: &AppContext) -> u32 {
        self.inner
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .max_line()
            .as_u32()
            .max(1)
    }

    pub fn is_cursor_on_first_visual_row(&self, ctx: &AppContext) -> bool {
        let inner = self.inner.as_ref(ctx);
        if !inner
            .buffer_selection_model()
            .as_ref(ctx)
            .all_single_cursors()
        {
            return false;
        }
        let cursor = inner
            .selection_model()
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default();
        let render = inner.render_state().as_ref(ctx);
        let point = render.offset_to_softwrap_point(cursor.saturating_sub(&1.into()));
        point.row() == 0
    }

    pub fn cursor_offset(&self, ctx: &impl ModelAsRef) -> CharOffset {
        self.inner
            .as_ref(ctx)
            .selection_model()
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    pub fn selection_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let inner = self.inner.as_ref(ctx);
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

    pub fn plain_text(&self, ctx: &impl ModelAsRef) -> String {
        let inner = self.inner.as_ref(ctx);
        let buffer = inner.content().as_ref(ctx);
        if buffer.is_empty() {
            return String::new();
        }
        buffer.text().into_string()
    }

    // ── Scroll ───────────────────────────────────────────────────────────────

    pub fn scroll_offset(&self) -> u32 {
        self.scroll_offset
    }

    pub fn scroll_to_cursor(&mut self, visible_rows: u32, ctx: &AppContext) {
        let inner = self.inner.as_ref(ctx);
        let cursor_offset = self.cursor_offset(ctx);
        let render = inner.render_state().as_ref(ctx);
        let point = render.offset_to_softwrap_point(cursor_offset.saturating_sub(&1.into()));
        let cursor_row = point.row();

        if cursor_row < self.scroll_offset {
            self.scroll_offset = cursor_row;
        } else if cursor_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = cursor_row.saturating_sub(visible_rows - 1);
        }
    }

    // ── Terminal resize ───────────────────────────────────────────────────────

    pub fn set_terminal_width(&mut self, width: u16, ctx: &mut ModelContext<Self>) {
        if self.terminal_width == width {
            return;
        }
        self.terminal_width = width;
        self.inner
            .update(ctx, |inner, ctx| inner.set_tui_terminal_width(width, ctx));
        ctx.emit(TuiEditorModelEvent::Changed);
    }

    // ── Kill / yank ───────────────────────────────────────────────────────────

    pub fn kill_to_line_end(&mut self, ctx: &mut ModelContext<Self>) {
        let range = self.range_to_visual_line_end(ctx);
        if let Some(range) = range {
            let killed = self
                .inner
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.inner.update(ctx, |inner, ctx| {
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

    pub fn kill_to_line_start(&mut self, ctx: &mut ModelContext<Self>) {
        let range = self.range_from_visual_line_start(ctx);
        if let Some(range) = range {
            let killed = self
                .inner
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.inner.update(ctx, |inner, ctx| {
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

    pub fn yank(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(text) = self.kill_buffer.yank().map(str::to_owned) {
            self.inner.update(ctx, |inner, ctx| {
                inner.user_insert(&text, ctx);
            });
        }
    }

    // ── Submit ────────────────────────────────────────────────────────────────

    pub fn submit(&mut self, ctx: &mut ModelContext<Self>) {
        let text = self.plain_text(ctx);
        ctx.emit(TuiEditorModelEvent::Submit(text));
        self.inner.update(ctx, |inner, ctx| inner.clear_buffer(ctx));
        self.scroll_offset = 0;
    }

    // ── Editing passthrough ───────────────────────────────────────────────────
    // The view calls these via model.update(ctx, |m, ctx| m.insert_char(c, ctx)).

    pub fn insert_char(&mut self, c: char, ctx: &mut ModelContext<Self>) {
        let s = c.to_string();
        self.inner
            .update(ctx, |inner, ctx| inner.user_insert(&s, ctx));
    }

    pub fn insert_newline(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner
            .update(ctx, |inner, ctx| inner.user_insert("\n", ctx));
    }

    pub fn backspace(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.backspace(ctx));
    }

    pub fn delete_forward(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_editor::selection::TextUnit;
        self.inner.update(ctx, |inner, ctx| {
            inner.delete(
                warp_editor::selection::TextDirection::Forwards,
                TextUnit::Character,
                false,
                ctx,
            )
        });
    }

    pub fn move_left(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.move_left(ctx));
    }

    pub fn move_right(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.move_right(ctx));
    }

    pub fn move_up(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.move_up(ctx));
    }

    pub fn move_down(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.move_down(ctx));
    }

    pub fn move_word_left(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_editor::selection::TextUnit;
        use warpui_core::text::word_boundaries::WordBoundariesPolicy;
        self.inner.update(ctx, |inner, ctx| {
            inner.backward_word_with_unit(false, TextUnit::Word(WordBoundariesPolicy::Default), ctx)
        });
    }

    pub fn move_word_right(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_editor::selection::TextUnit;
        use warpui_core::text::word_boundaries::WordBoundariesPolicy;
        self.inner.update(ctx, |inner, ctx| {
            inner.forward_word_with_unit(false, TextUnit::Word(WordBoundariesPolicy::Default), ctx)
        });
    }

    pub fn move_to_line_start(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner
            .update(ctx, |inner, ctx| inner.move_to_line_start(ctx));
    }

    pub fn move_to_line_end(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner
            .update(ctx, |inner, ctx| inner.move_to_line_end(ctx));
    }

    pub fn select_left(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.select_left(ctx));
    }

    pub fn select_right(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.select_right(ctx));
    }

    pub fn select_up(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.select_up(ctx));
    }

    pub fn select_down(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.select_down(ctx));
    }

    pub fn select_all(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.select_all(ctx));
    }

    pub fn delete_word_backward(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_editor::selection::TextUnit;
        use warpui_core::text::word_boundaries::WordBoundariesPolicy;
        self.inner.update(ctx, |inner, ctx| {
            inner.delete(
                warp_editor::selection::TextDirection::Backwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                false,
                ctx,
            )
        });
    }

    pub fn delete_word_forward(&mut self, ctx: &mut ModelContext<Self>) {
        use warp_editor::selection::TextUnit;
        use warpui_core::text::word_boundaries::WordBoundariesPolicy;
        self.inner.update(ctx, |inner, ctx| {
            inner.delete(
                warp_editor::selection::TextDirection::Forwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                false,
                ctx,
            )
        });
    }

    pub fn undo(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.undo(ctx));
    }

    pub fn redo(&mut self, ctx: &mut ModelContext<Self>) {
        self.inner.update(ctx, |inner, ctx| inner.redo(ctx));
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn range_to_visual_line_end(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let cursor = self.cursor_offset(ctx);
        let inner = self.inner.as_ref(ctx);
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
        let inner = self.inner.as_ref(ctx);
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
