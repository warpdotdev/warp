//! [`TuiEditorModel`] — the editor-backed model for the TUI input view.
//!
//! This model implements [`CoreEditorModel`] from `warp_editor`, which gives it all standard
//! text-editing operations (insert, backspace, delete, word movement, undo/redo, selection)
//! for free. Cursor navigation uses the char-cell [`LayoutMode`] of [`RenderState`] so
//! soft-wrap and `move_up`/`move_down` work correctly in terminal environments.
//!
//! See `specs/tui-input-view/TECH.md` for architecture rationale.

use std::ops::Range;

use num_traits::SaturatingSub;
use string_offset::CharOffset;
use warp_editor::content::buffer::{Buffer, BufferEditAction, EditOrigin};
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::{IndentBehavior, TextStyles};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::render::model::{
    BlockSpacings, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
    ParagraphStyles, RenderState, RichTextStyles, TableStyle,
};
use warp_editor::selection::{SelectionModel, TextDirection, TextUnit};
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{AppContext, Entity, ModelAsRef, ModelContext, ModelHandle};

use super::kill_buffer::KillBuffer;

// ─────────────────────────────────────────────────────────────────────────────
// Events
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by [`TuiEditorModel`].
#[derive(Debug, Clone)]
pub enum TuiEditorModelEvent {
    /// The buffer text, cursor position, or selection changed. The view should
    /// re-render on receiving this event.
    Changed,
    /// The user pressed Enter to submit the current input. Contains the final text.
    Submit(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Model
// ─────────────────────────────────────────────────────────────────────────────

/// The editing model behind [`super::view::TuiInputView`].
///
/// ## Why a new struct instead of reusing `CodeEditorModel` / `NotebooksEditorModel`?
///
/// The **editor infrastructure IS reused**: `TuiEditorModel` implements
/// [`CoreEditorModel`] and [`PlainTextEditorModel`] from `warp_editor`, which
/// provides the full editing engine (insert, backspace, word movement, undo/redo,
/// selection, kill/yank) for free.
///
/// The two existing concrete GUI structs that also implement `CoreEditorModel` are
/// incompatible with TUI for two reasons:
///
/// 1. **LayoutMode coupling**: `CodeEditorModel` and `NotebooksEditorModel` always
///    construct [`RenderState::new`] with [`LayoutMode::Pixels`] (font-aware GPU
///    layout). The TUI path requires [`RenderState::new_tui`] with
///    [`LayoutMode::CharCell`] (monospace char-cell arithmetic, no font engine).
///    There is no way to construct the GUI models with the TUI layout mode.
///
/// 2. **GUI-specific fields**: both carry deps that are unconstructable in a
///    terminal — `CodeEditorModel` has vim mode, syntax tree, diff model, comments,
///    hidden lines, and `SessionPlatform`; `NotebooksEditorModel` has child models
///    (mermaid/command blocks), a `WindowId` binding, and viewport resize channels.
///    None of these apply to a TUI input field.
///
/// `TuiEditorModel` is therefore the minimal TUI-appropriate implementation: a buffer,
/// selection state, char-cell `RenderState`, and a kill buffer — nothing more.
pub struct TuiEditorModel {
    /// The plain-text backing store.
    buffer: ModelHandle<Buffer>,
    /// Buffer-level selection (head/tail offsets, anchors).
    buffer_selection: ModelHandle<BufferSelectionModel>,
    /// High-level selection model: cursor navigation, word movement, goal column.
    selection: ModelHandle<SelectionModel>,
    /// Char-cell [`RenderState`] — drives `offset_to_softwrap_point` and `max_line`.
    render: ModelHandle<RenderState>,
    /// Single-entry kill buffer for `Ctrl+K` / `Ctrl+U` / `Ctrl+Y`.
    kill_buffer: KillBuffer,
    /// Terminal width in character columns. Updated on resize.
    terminal_width: u16,
    /// First visible visual row (0-indexed). Updated after each cursor move.
    scroll_offset: u32,
}

impl Entity for TuiEditorModel {
    type Event = TuiEditorModelEvent;
}

// ─────────────────────────────────────────────────────────────────────────────
// CoreEditorModel implementation
// ─────────────────────────────────────────────────────────────────────────────

impl CoreEditorModel for TuiEditorModel {
    type T = TuiEditorModel;

    fn content(&self) -> &ModelHandle<Buffer> {
        &self.buffer
    }

    fn buffer_selection_model(&self) -> &ModelHandle<BufferSelectionModel> {
        &self.buffer_selection
    }

    fn selection_model(&self) -> &ModelHandle<SelectionModel> {
        &self.selection
    }

    fn render_state(&self) -> &ModelHandle<RenderState> {
        &self.render
    }

    /// No validation needed for the TUI input (plain text only, no rich-text invariants).
    fn validate(&self, _ctx: &impl ModelAsRef) {}

    /// Plain text — no styling is applied to typed characters.
    fn active_text_style(&self) -> TextStyles {
        TextStyles::default()
    }

    /// Called synchronously after every buffer edit. Updates the char-cell layout
    /// state and notifies the view.
    fn on_buffer_version_updated(
        &self,
        _version: warp_editor::content::version::BufferVersion,
        ctx: &mut ModelContext<Self::T>,
    ) {
        // Extract the plain-text buffer content and push it into the CharCell RenderState.
        let text = self.plain_text_without_trailing_sentinel(ctx);
        self.render.update(ctx, |render_state, _| {
            render_state.update_char_cell_text(&text);
        });
        ctx.emit(TuiEditorModelEvent::Changed);
    }
}

impl PlainTextEditorModel for TuiEditorModel {}

// ─────────────────────────────────────────────────────────────────────────────
// Construction
// ─────────────────────────────────────────────────────────────────────────────

impl TuiEditorModel {
    /// Create a new, empty `TuiEditorModel`.
    ///
    /// `terminal_width` is the initial terminal width in character columns.
    /// Call [`set_terminal_width`] when the terminal is resized.
    pub fn new(terminal_width: u16, ctx: &mut ModelContext<Self>) -> Self {
        let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));
        let render =
            ctx.add_model(|ctx| RenderState::new_tui(terminal_width, tui_stub_styles(), ctx));
        let selection = ctx.add_model(|ctx| {
            SelectionModel::new(
                buffer.clone(),
                render.clone(),
                buffer_selection.clone(),
                None,
                ctx,
            )
        });

        Self {
            buffer,
            buffer_selection,
            selection,
            render,
            kill_buffer: KillBuffer::default(),
            terminal_width,
            scroll_offset: 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stub RichTextStyles for TUI (styles are ignored by CharCell RenderState)
// ─────────────────────────────────────────────────────────────────────────────

/// Minimal [`RichTextStyles`] for TUI char-cell mode.
/// `RenderState::new_tui` stores styles for API compatibility but never reads them
/// during char-cell layout. Defined here (in `warp_tui`) rather than in `warp_editor`
/// since it is purely a TUI concern.
fn tui_stub_styles() -> RichTextStyles {
    use warpui_core::color::ColorU;
    use warpui_core::elements::{Border, Fill};
    use warpui_core::fonts::{FamilyId, Weight};
    const ZERO: ColorU = ColorU {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    let para = ParagraphStyles {
        font_family: FamilyId(0),
        font_size: 10.,
        font_weight: Weight::Normal,
        line_height_ratio: 1.,
        text_color: ZERO,
        baseline_ratio: 0.7,
        fixed_width_tab_size: None,
    };
    RichTextStyles {
        base_text: para,
        code_text: ParagraphStyles {
            fixed_width_tab_size: Some(4),
            ..para
        },
        code_background: Fill::None,
        embedding_background: Fill::None,
        embedding_text: para,
        code_border: Border::new(0.),
        placeholder_color: ZERO,
        selection_fill: Fill::None,
        cursor_fill: Fill::None,
        inline_code_style: InlineCodeStyle {
            font_family: FamilyId(0),
            background: ZERO,
            font_color: ZERO,
        },
        check_box_style: CheckBoxStyle {
            border_width: 0.,
            border_color: ZERO,
            icon_path: "",
            background: ZERO,
            hover_background: ZERO,
        },
        horizontal_rule_style: HorizontalRuleStyle {
            rule_height: 0.,
            color: ZERO,
        },
        broken_link_style: BrokenLinkStyle {
            icon_path: "",
            icon_color: ZERO,
        },
        block_spacings: BlockSpacings::default(),
        minimum_paragraph_height: None,
        show_placeholder_text_on_empty_block: false,
        cursor_width: 0.,
        highlight_urls: false,
        table_style: TableStyle {
            border_color: ZERO,
            header_background: ZERO,
            cell_background: ZERO,
            alternate_row_background: None,
            text_color: ZERO,
            header_text_color: ZERO,
            scrollbar_nonactive_thumb_color: ZERO,
            scrollbar_active_thumb_color: ZERO,
            font_family: FamilyId(0),
            font_size: 10.,
            cell_padding: 0.,
            outer_border: false,
            column_dividers: false,
            row_dividers: false,
        },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TUI-specific public API
// ─────────────────────────────────────────────────────────────────────────────

impl TuiEditorModel {
    // ── Cursor queries ────────────────────────────────────────────────────────

    /// Returns the total number of visual rows occupied by the current buffer
    /// content, given the current `terminal_width`.
    pub fn visual_line_count(&self, ctx: &AppContext) -> u32 {
        self.render.as_ref(ctx).max_line().as_u32().max(1)
    }

    /// Whether the cursor sits on visual row 0.
    ///
    /// Used by the view layer to decide whether `↑` should move the cursor up
    /// within the buffer or open the history menu (future: Step M2).
    pub fn is_cursor_on_first_visual_row(&self, ctx: &AppContext) -> bool {
        let render = self.render.as_ref(ctx);
        if !self.buffer_selection.as_ref(ctx).all_single_cursors() {
            return false;
        }
        let cursor = self
            .selection
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default();
        // Adjust for the 1-indexed convention used throughout the editor crate.
        let point = render.offset_to_softwrap_point(cursor.saturating_sub(&1.into()));
        point.row() == 0
    }

    // ── Scroll management ─────────────────────────────────────────────────────

    /// The first visible visual row index.
    pub fn scroll_offset(&self) -> u32 {
        self.scroll_offset
    }

    /// Update the scroll offset so that the cursor remains visible within
    /// `visible_rows` rows.
    pub fn scroll_to_cursor(&mut self, visible_rows: u32, ctx: &AppContext) {
        let render = self.render.as_ref(ctx);
        let cursor_offset = self
            .selection
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default();
        let point = render.offset_to_softwrap_point(cursor_offset.saturating_sub(&1.into()));
        let cursor_row = point.row();

        if cursor_row < self.scroll_offset {
            self.scroll_offset = cursor_row;
        } else if cursor_row >= self.scroll_offset + visible_rows {
            self.scroll_offset = cursor_row.saturating_sub(visible_rows - 1);
        }
    }

    // ── Terminal width ────────────────────────────────────────────────────────

    /// Returns the current terminal width in character columns.
    pub fn terminal_width(&self) -> u16 {
        self.terminal_width
    }

    /// Update the terminal width (called by the view on a resize event).
    /// Also re-syncs the char-cell layout and notifies the view.
    pub fn set_terminal_width(&mut self, width: u16, ctx: &mut ModelContext<Self>) {
        if self.terminal_width == width {
            return;
        }
        self.terminal_width = width;

        let text = self.plain_text_without_trailing_sentinel(ctx);
        self.render.update(ctx, |render_state, _| {
            render_state.set_char_cell_terminal_width(width);
            render_state.update_char_cell_text(&text);
        });
        ctx.emit(TuiEditorModelEvent::Changed);
    }

    // ── Kill/yank (Emacs readline) ────────────────────────────────────────────

    /// Kill from the cursor to the end of the current visual line (`Ctrl+K`).
    ///
    /// The killed text is stored in the kill buffer for later `yank()`.
    pub fn kill_to_line_end(&mut self, ctx: &mut ModelContext<Self>) {
        let range = self.range_to_visual_line_end(ctx);
        if let Some(range) = range {
            let killed = self
                .buffer
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::Delete(vec1::vec1![range]),
                        EditOrigin::UserInitiated,
                        self.buffer_selection_model().clone(),
                        ctx,
                    );
                },
                ctx,
            );
        }
    }

    /// Kill from the cursor to the start of the current visual line (`Ctrl+U`).
    pub fn kill_to_line_start(&mut self, ctx: &mut ModelContext<Self>) {
        let range = self.range_from_visual_line_start(ctx);
        if let Some(range) = range {
            let killed = self
                .buffer
                .as_ref(ctx)
                .text_in_range(range.clone())
                .into_string();
            self.kill_buffer.kill(killed);
            self.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::Delete(vec1::vec1![range]),
                        EditOrigin::UserInitiated,
                        self.buffer_selection_model().clone(),
                        ctx,
                    );
                },
                ctx,
            );
        }
    }

    /// Yank (paste) the last killed text at the cursor position (`Ctrl+Y`).
    pub fn yank(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(text) = self.kill_buffer.yank().map(str::to_owned) {
            self.user_insert(&text, ctx);
        }
    }

    // ── Submit ────────────────────────────────────────────────────────────────

    /// Submit the current input: emits a [`TuiEditorModelEvent::Submit`] with the
    /// text and resets the buffer to empty.
    pub fn submit(&mut self, ctx: &mut ModelContext<Self>) {
        let text = self.plain_text_without_trailing_sentinel(ctx);
        ctx.emit(TuiEditorModelEvent::Submit(text));
        // Clear the buffer.
        self.clear_buffer(ctx);
        self.scroll_offset = 0;
    }

    // ── Raw text access ───────────────────────────────────────────────────────

    /// Returns the plain-text content of the buffer as a `String`.
    ///
    /// Uses the same range as `Buffer::text()` — from offset 1 through
    /// `max_charoffset()` — which covers the full text without the block
    /// marker at offset 0 and without any extra sentinel.
    pub fn plain_text_without_trailing_sentinel(&self, ctx: &impl ModelAsRef) -> String {
        let buffer = self.buffer.as_ref(ctx);
        if buffer.is_empty() {
            return String::new();
        }
        buffer.text().into_string()
    }

    /// Returns the cursor's [`CharOffset`] (first selection head).
    pub fn cursor_offset(&self, ctx: &impl ModelAsRef) -> CharOffset {
        self.selection
            .as_ref(ctx)
            .cursors(ctx)
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// Returns the current selection as a `Range<CharOffset>`, or `None` if
    /// the selection is a single cursor with no extent.
    pub fn selection_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let sel = self.buffer_selection.as_ref(ctx);
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

    // ── Word movement (Ctrl+← / Alt+B, etc.) ─────────────────────────────────

    /// Delete the character after the cursor (`Delete`, `Ctrl+D`).
    pub fn delete_forward(&mut self, ctx: &mut ModelContext<Self>) {
        self.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
    }

    /// Move one word backwards (`Alt+B` / `Ctrl+←`).
    pub fn move_word_left(&mut self, ctx: &mut ModelContext<Self>) {
        self.backward_word_with_unit(false, TextUnit::Word(WordBoundariesPolicy::Default), ctx);
    }

    /// Move one word forwards (`Alt+F` / `Ctrl+→`).
    pub fn move_word_right(&mut self, ctx: &mut ModelContext<Self>) {
        self.forward_word_with_unit(false, TextUnit::Word(WordBoundariesPolicy::Default), ctx);
    }

    /// Extend selection one word backwards (`Alt+Shift+B`).
    pub fn select_word_left(&mut self, ctx: &mut ModelContext<Self>) {
        self.backward_word_with_unit(true, TextUnit::Word(WordBoundariesPolicy::Default), ctx);
    }

    /// Extend selection one word forwards (`Alt+Shift+F`).
    pub fn select_word_right(&mut self, ctx: &mut ModelContext<Self>) {
        self.forward_word_with_unit(true, TextUnit::Word(WordBoundariesPolicy::Default), ctx);
    }

    /// Delete one word backwards (`Ctrl+W` / `Alt+Backspace`).
    pub fn delete_word_backward(&mut self, ctx: &mut ModelContext<Self>) {
        self.delete(
            TextDirection::Backwards,
            TextUnit::Word(WordBoundariesPolicy::Default),
            false,
            ctx,
        );
    }

    /// Delete one word forwards (`Alt+D` / `Ctrl+Delete`).
    pub fn delete_word_forward(&mut self, ctx: &mut ModelContext<Self>) {
        self.delete(
            TextDirection::Forwards,
            TextUnit::Word(WordBoundariesPolicy::Default),
            false,
            ctx,
        );
    }

    // ── Shift+Arrow selection ─────────────────────────────────────────────────

    /// Extend selection left (`Shift+←`).
    pub fn select_left(&mut self, ctx: &mut ModelContext<Self>) {
        CoreEditorModel::select_left(self, ctx);
    }

    /// Extend selection right (`Shift+→`).
    pub fn select_right(&mut self, ctx: &mut ModelContext<Self>) {
        CoreEditorModel::select_right(self, ctx);
    }

    /// Extend selection up (`Shift+↑`).
    pub fn extend_select_up(&mut self, ctx: &mut ModelContext<Self>) {
        self.select_up(ctx);
    }

    /// Extend selection down (`Shift+↓`).
    pub fn extend_select_down(&mut self, ctx: &mut ModelContext<Self>) {
        self.select_down(ctx);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// The range from the cursor to the end of its visual line, used by `Ctrl+K`.
    ///
    /// Finds the start of the next visual row via `row+1, col=0`, subtracts 1
    /// to land at the end of the current row, then clamps to `max_charoffset()`
    /// so single-line buffers don't produce huge phantom offsets.
    fn range_to_visual_line_end(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let cursor = self.cursor_offset(ctx);
        let max = self.buffer.as_ref(ctx).max_charoffset();
        let render = self.render.as_ref(ctx);
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
            return None;
        }
        Some(cursor..line_end)
    }

    /// The range from the start of the cursor's visual line to the cursor,
    /// used by `Ctrl+U`.
    fn range_from_visual_line_start(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        let cursor = self.cursor_offset(ctx);
        let render = self.render.as_ref(ctx);
        let adjusted = cursor.saturating_sub(&CharOffset::from(1));
        let pt = render.offset_to_softwrap_point(adjusted);
        let line_start =
            render.softwrap_point_to_offset(warp_editor::render::model::SoftWrapPoint::new(
                pt.row(),
                warp_editor::render::model::ColumnUnit::chars_zero(),
            ));

        if line_start >= cursor {
            return None;
        }
        Some(line_start..cursor)
    }
}
