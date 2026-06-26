//! [`TuiInputView`] — the ratatui-rendered view for the TUI input.
//!
//! Implements [`TuiView`] + [`TypedActionView`]. The view's `render()` produces a
//! [`TuiInputElement`] that:
//!
//! - Lays out and paints visible text rows from the model buffer.
//! - Returns the cursor's cell position via `cursor_position()`.
//! - Dispatches keystrokes as [`TuiInputAction`] typed actions via `dispatch_event()`.
//!
//! `handle_action()` receives the action and calls the corresponding model method.
//!
//! See `specs/tui-input-view/TECH.md` for the full keybinding table and architecture.

use std::cmp;

use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{
    Modifier, TuiBuffer, TuiColumn, TuiConstraint, TuiElement, TuiEventContext, TuiLayoutContext,
    TuiParentElement, TuiRect, TuiSize, TuiStyle, TuiText,
};
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewContext};

use super::model::TuiInputModel;

// ─────────────────────────────────────────────────────────────────────────────
// Typed action enum
// ─────────────────────────────────────────────────────────────────────────────

/// All editing operations that can be dispatched from `TuiInputElement::dispatch_event`.
///
/// Each variant corresponds to one or more keybindings from the spec's keybinding table.
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
    /// Redo (`Ctrl+Shift+Z`, `Ctrl+Y` after redo — future).
    Redo,
}

// `TuiInputAction` satisfies the `Action` blanket impl (Any + Debug + Send + Sync).

// ─────────────────────────────────────────────────────────────────────────────
// View
// ─────────────────────────────────────────────────────────────────────────────

/// The `TuiView`-implementing entry point for the TUI prompt input.
pub struct TuiInputView {
    model: ModelHandle<TuiInputModel>,
    /// Maximum number of visible rows before the input scrolls (matches spec: 6).
    max_visible_rows: u32,
}

impl Entity for TuiInputView {
    type Event = ();
}

impl TuiInputView {
    /// Construct a new `TuiInputView` wrapping `model`.
    pub fn new(model: ModelHandle<TuiInputModel>) -> Self {
        Self {
            model,
            max_visible_rows: 6,
        }
    }
}

impl TuiView for TuiInputView {
    fn ui_name() -> &'static str {
        "TuiInputView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let model = self.model.as_ref(ctx);

        // ── Gather model state ─────────────────────────────────────────────────
        let text = model.plain_text_without_trailing_sentinel(ctx);
        let terminal_width = model.terminal_width();
        let scroll_offset = model.scroll_offset();
        let visible_rows = cmp::min(model.visual_line_count(ctx), self.max_visible_rows);

        // Cursor position in visual coordinates.
        let cursor_offset = model.cursor_offset(ctx);
        let (cursor_visual_row, cursor_col) =
            char_cell_cursor_pos(&text, cursor_offset, terminal_width);

        // Cursor row relative to the scroll offset.
        let cursor_row_in_view = cursor_visual_row.saturating_sub(scroll_offset);

        // ── Build visual rows ──────────────────────────────────────────────────
        let all_rows = build_visual_rows(&text, terminal_width);

        // Take the visible slice.
        let visible_start = scroll_offset as usize;
        let visible_end = (scroll_offset as usize + visible_rows as usize).min(all_rows.len());
        let visible_row_strings: Vec<String> = if visible_start < all_rows.len() {
            all_rows[visible_start..visible_end].to_vec()
        } else {
            vec![String::new()]
        };

        // ── Assemble TuiColumn ─────────────────────────────────────────────────
        let dim = TuiStyle::default().add_modifier(Modifier::DIM);
        let mut column = TuiColumn::new();

        for (row_idx, row_text) in visible_row_strings.iter().enumerate() {
            let is_cursor_row = row_idx as u32 == cursor_row_in_view;
            // Highlight the cursor row slightly (DIM everything else).
            // Full selection highlighting is a follow-up (Step M2).
            let style = if is_cursor_row {
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
        })
    }
}

impl TypedActionView for TuiInputView {
    type Action = TuiInputAction;

    fn handle_action(&mut self, action: &TuiInputAction, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| match action {
            TuiInputAction::InsertChar(c) => {
                let s = c.to_string();
                model.user_insert(&s, ctx);
            }
            TuiInputAction::InsertNewline => model.user_insert("\n", ctx),
            TuiInputAction::Submit => model.submit(ctx),
            TuiInputAction::Backspace => model.backspace(ctx),
            TuiInputAction::DeleteForward => model.delete_forward(ctx),
            TuiInputAction::MoveLeft => model.move_left(ctx),
            TuiInputAction::MoveRight => model.move_right(ctx),
            TuiInputAction::MoveUp => model.move_up(ctx),
            TuiInputAction::MoveDown => model.move_down(ctx),
            TuiInputAction::MoveWordLeft => model.move_word_left(ctx),
            TuiInputAction::MoveWordRight => model.move_word_right(ctx),
            TuiInputAction::MoveToLineStart => model.move_to_line_start(ctx),
            TuiInputAction::MoveToLineEnd => model.move_to_line_end(ctx),
            TuiInputAction::SelectLeft => model.select_left(ctx),
            TuiInputAction::SelectRight => model.select_right(ctx),
            TuiInputAction::SelectUp => model.extend_select_up(ctx),
            TuiInputAction::SelectDown => model.extend_select_down(ctx),
            TuiInputAction::SelectAll => model.select_all(ctx),
            TuiInputAction::DeleteWordBackward => model.delete_word_backward(ctx),
            TuiInputAction::DeleteWordForward => model.delete_word_forward(ctx),
            TuiInputAction::KillToLineEnd => model.kill_to_line_end(ctx),
            TuiInputAction::KillToLineStart => model.kill_to_line_start(ctx),
            TuiInputAction::Yank => model.yank(ctx),
            TuiInputAction::Undo => model.undo(ctx),
            TuiInputAction::Redo => model.redo(ctx),
        });

        // After any action, scroll to keep the cursor visible and re-render.
        let visible_rows = {
            let m = self.model.as_ref(ctx);
            cmp::min(m.visual_line_count(ctx), self.max_visible_rows)
        };
        self.model.update(ctx, |model, app| {
            model.scroll_to_cursor(visible_rows.max(1), app);
        });
        ctx.notify();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TuiInputElement — the element returned from render()
// ─────────────────────────────────────────────────────────────────────────────

/// The element returned from [`TuiInputView::render`].
///
/// Wraps a [`TuiColumn`] of [`TuiText`] rows and overrides `cursor_position` and
/// `dispatch_event`. Layout and paint are fully delegated to the inner column.
struct TuiInputElement {
    column: TuiColumn,
    /// The cursor's 0-based column within the visible area.
    cursor_col: u16,
    /// The cursor's 0-based row within the visible area (after scroll_offset subtraction).
    cursor_row_in_view: u16,
}

impl TuiElement for TuiInputElement {
    fn layout(&mut self, constraint: TuiConstraint, ctx: &mut TuiLayoutContext) -> TuiSize {
        self.column.layout(constraint, ctx)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiLayoutContext) {
        self.column.render(area, buffer, ctx);
    }

    fn cursor_position(&self, area: TuiRect, _ctx: &mut TuiLayoutContext) -> Option<(u16, u16)> {
        let x = area.x.saturating_add(self.cursor_col);
        let y = area.y.saturating_add(self.cursor_row_in_view);
        // Clamp to the allocated area.
        if x >= area.x + area.width || y >= area.y + area.height {
            // Cursor is out of the visible area — don't show it.
            return None;
        }
        Some((x, y))
    }

    fn dispatch_event(
        &mut self,
        event: &warpui_core::Event,
        area: TuiRect,
        event_ctx: &mut TuiEventContext,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> bool {
        // First offer the event to the child column (currently a no-op since
        // TuiText and TuiColumn don't handle events themselves).
        if self.column.dispatch_event(event, area, event_ctx, ctx, app) {
            return true;
        }

        // Decode the keystroke and dispatch a typed action.
        if let warpui_core::Event::KeyDown {
            keystroke, chars, ..
        } = event
        {
            let ctrl = keystroke.ctrl;
            let alt = keystroke.alt;
            let shift = keystroke.shift;
            let key = keystroke.key.as_str();

            let action: Option<TuiInputAction> = match (ctrl, alt, shift, key) {
                // ── Submit / Newline ─────────────────────────────────────────────
                (false, false, false, "enter") => Some(TuiInputAction::Submit),
                (false, false, true, "enter") => Some(TuiInputAction::InsertNewline),
                (true, false, false, "j") => Some(TuiInputAction::InsertNewline),
                (false, true, false, "enter") => Some(TuiInputAction::InsertNewline),
                // ── Deletion ─────────────────────────────────────────────────────
                (false, false, _, "backspace") => Some(TuiInputAction::Backspace),
                (true, false, false, "h") => Some(TuiInputAction::Backspace),
                (false, false, false, "delete") => Some(TuiInputAction::DeleteForward),
                (true, false, false, "d") => Some(TuiInputAction::DeleteForward),
                // Word delete backward: Ctrl+W, Ctrl+Backspace, Alt+Backspace
                (true, false, false, "w") => Some(TuiInputAction::DeleteWordBackward),
                (true, false, false, "backspace") => Some(TuiInputAction::DeleteWordBackward),
                (false, true, false, "backspace") => Some(TuiInputAction::DeleteWordBackward),
                // Word delete forward: Alt+D, Alt+Delete, Ctrl+Delete
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
                // Word movement
                (false, true, false, "left")
                | (false, true, false, "b")
                | (true, false, false, "left") => Some(TuiInputAction::MoveWordLeft),
                (false, true, false, "right")
                | (false, true, false, "f")
                | (true, false, false, "right") => Some(TuiInputAction::MoveWordRight),
                // Line start/end
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
                    // `chars` is set by the runtime for printable character keys.
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

/// Splits `text` into visual rows of at most `terminal_width` chars each.
/// Each `\n` starts a new logical line; empty logical lines produce one empty row.
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

/// Returns the `(visual_row, visual_col)` of `cursor_offset` in char-cell coordinates.
///
/// `cursor_offset` is a 1-indexed [`CharOffset`] (the editor convention).
/// The result uses 0-based row and column indices.
pub fn char_cell_cursor_pos(
    text: &str,
    cursor_offset: string_offset::CharOffset,
    terminal_width: u16,
) -> (u32, u32) {
    // Convert to 0-based char index (buffer is 1-indexed).
    let cursor_char_idx = cursor_offset.as_usize().saturating_sub(1);

    let w = terminal_width as usize;
    let mut visual_row: u32 = 0;
    let mut chars_so_far: usize = 0;

    for logical_line in text.split('\n') {
        let line_len = logical_line.chars().count();
        let line_end_exclusive = chars_so_far + line_len;

        if cursor_char_idx <= line_end_exclusive {
            // Cursor is within this logical line (or at the end of it).
            let offset_in_line = cursor_char_idx.saturating_sub(chars_so_far);
            let (row_in_line, col) = if w == 0 {
                (0, offset_in_line as u32)
            } else {
                ((offset_in_line / w) as u32, (offset_in_line % w) as u32)
            };
            return (visual_row + row_in_line, col);
        }

        // Account for this logical line's visual rows.
        let line_rows = if w == 0 || line_len == 0 {
            1
        } else {
            line_len.div_ceil(w).max(1)
        };
        visual_row += line_rows as u32;
        // +1 for the `\n` character itself.
        chars_so_far = line_end_exclusive + 1;
    }

    // Cursor is past the end of the text.
    (visual_row, 0)
}
