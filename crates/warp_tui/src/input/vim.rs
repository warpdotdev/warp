//! [`VimHandler`] implementation for [`TuiInputView`].
//!
//! Wires the TUI prompt's backing [`CodeEditorModel`] into the shared vim
//! dispatch layer (the same pattern [`CodeEditorView`] uses).  Prompt-specific
//! semantics are expressed as explicit no-ops or custom overrides in the trait
//! implementation rather than as arms in a bespoke match:
//!
//! - `find_char` — no-op (single-line prompt; `f`/`F`/`t`/`T` are skipped).
//! - `navigate_paragraph` — no-op (no paragraph structure in a prompt).
//! - `jump_to_*_bracket` — no-op.
//! - `jump_to_line` — no-op (single logical line).
//! - `operation` with `VimOperand::Line` (`dd`, `cc`, `yy`) — kills / yanks
//!   the full current line via prompt-specific kill helpers.
//! - `insert_text` / `change_mode` for `LineAbove`/`LineBelow` (`o`/`O`) —
//!   no-op new-line insertion; the mode switch to Insert still happens.
//! - `search`, `cycle_search`, `search_word_at_cursor` — no-op.
//! - `visual_paste` — inserts from the local yank buffer (no register system).
//! - `join_line`, `toggle_case`, `keyword_prg`, `ex_command` — no-op.
//! - Scroll helpers (`center_cursor_vertically`, `scroll_half_page_*`) — no-op.
//!
//! All VimModeChanged notifications are emitted from `change_mode` so the
//! parent session view can update its footer indicator.

use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
    InsertPosition, LineMotion, ModeTransition, MotionType, VimHandler, VimMode, VimMotion,
    VimOperand, VimOperator, VimTextObject, WordMotion,
};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui_core::ViewContext;

use super::{TuiInputView, TuiInputViewEvent};
use crate::editor_interaction::TuiEditorCommand;

impl VimHandler for TuiInputView {
    // ── Character insertion ───────────────────────────────────────────────────

    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>) {
        let c_str = c.to_string();
        self.model.update(ctx, |m, ctx| m.user_insert(&c_str, ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    fn navigate_char(
        &mut self,
        count: u32,
        character_motion: &CharacterMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| match character_motion {
            CharacterMotion::Right | CharacterMotion::WrappingRight => {
                model.vim_move_horizontal_by_offset(count, &Direction::Forward, false, true, ctx);
            }
            CharacterMotion::Left | CharacterMotion::WrappingLeft => {
                model.vim_move_horizontal_by_offset(count, &Direction::Backward, false, true, ctx);
            }
            CharacterMotion::Up => {
                model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
            }
            CharacterMotion::Down => {
                model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn navigate_word(&mut self, count: u32, word_motion: &WordMotion, ctx: &mut ViewContext<Self>) {
        let WordMotion {
            direction,
            bound,
            word_type,
        } = word_motion;
        self.model.update(ctx, |model, ctx| {
            model.vim_navigate_word(*direction, *bound, *word_type, count, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn navigate_line(&mut self, line_count: u32, motion: &LineMotion, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| match motion {
            // Use move_to_line_start/end (CoreEditorModel) instead of vim_move_to_line_bound
            // (which requires LineBound from app/src/code/editor) to avoid a dependency
            // on the GUI-side code module from the headless TUI crate.
            LineMotion::Start => model.move_to_line_start(ctx),
            LineMotion::FirstNonWhitespace => model.vim_move_to_first_nonwhitespace(false, ctx),
            LineMotion::End => {
                model.vim_move_vertical_by_offset(
                    line_count.saturating_sub(1),
                    TextDirection::Forwards,
                    false,
                    ctx,
                );
                model.move_to_line_end(ctx);
            }
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn first_nonwhitespace_motion(
        &mut self,
        count: u32,
        motion: &FirstNonWhitespaceMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            match motion {
                FirstNonWhitespaceMotion::Up => {
                    model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
                }
                FirstNonWhitespaceMotion::Down => {
                    model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
                }
                FirstNonWhitespaceMotion::DownMinusOne => {
                    model.vim_move_vertical_by_offset(
                        count - 1,
                        TextDirection::Forwards,
                        false,
                        ctx,
                    );
                }
            }
            model.vim_move_to_first_nonwhitespace(false, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: `f`/`F`/`t`/`T` are no-ops — single-line prompt
    /// makes find-char useful only when the cursor is at the start of a long
    /// line, and TUI's existing horizontal navigation covers that.
    fn find_char(
        &mut self,
        _occurrence_count: u32,
        _find_char_motion: &FindCharMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    /// Prompt-specific: `{` / `}` are no-ops — no paragraph structure.
    fn navigate_paragraph(
        &mut self,
        _count: u32,
        _direction: &Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    // ── Operators ─────────────────────────────────────────────────────────────

    fn operation(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        _register_name: char,
        _replacement_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        match (operator, operand) {
            // ── Linewise: dd / cc / yy ────────────────────────────────────
            (VimOperator::Delete | VimOperator::Change, VimOperand::Line) => {
                // `dd` (or `cc`): delete the whole current line, kill-buffer style.
                // Move to line start first so the entire visual row is removed
                // regardless of where the cursor is.
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                {
                    self.yank_buffer = killed;
                }
                self.follow_cursor(ctx);
            }
            (VimOperator::Yank, VimOperand::Line) => {
                // `yy`: yank full buffer content.
                let text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    if buffer.is_empty() {
                        String::new()
                    } else {
                        buffer.text().into_string()
                    }
                };
                self.yank_buffer = text;
            }

            // ── Motion-based operators ────────────────────────────────────
            (operator, VimOperand::Motion { motion, .. }) => {
                self.apply_motion_operation(operator, motion, operand_count, ctx);
            }

            // ── Text objects (word, sentence, etc.) ───────────────────────
            // TUI prompt skips text-object operators for simplicity.
            (_, VimOperand::TextObject(_)) => {}

            // ── Unsupported operator/operand combinations ─────────────────
            _ => {}
        }
        ctx.notify();
    }

    fn replace_char(&mut self, c: char, char_count: u32, ctx: &mut ViewContext<Self>) {
        self.model
            .update(ctx, |model, ctx| model.replace_char(c, char_count, ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: case-toggle is a no-op in the TUI prompt.
    fn toggle_case(&mut self, _char_count: u32, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Search (no-op in TUI prompt) ──────────────────────────────────────────

    fn search(&mut self, _direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn cycle_search(&mut self, _direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn search_word_at_cursor(&mut self, _direction: &Direction, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn ex_command(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn keyword_prg(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Visual mode operators ─────────────────────────────────────────────────

    fn visual_operator(
        &mut self,
        operator: &VimOperator,
        _motion_type: MotionType,
        _register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(anchor) = self.visual_selection_anchor.take() else {
            ctx.notify();
            return;
        };
        let cursor = self.cursor_offset(ctx);
        let (sel_start, sel_end) = if anchor <= cursor {
            (anchor, cursor)
        } else {
            (cursor, anchor)
        };

        match operator {
            VimOperator::Delete | VimOperator::Change => {
                // Yank the selection first, then delete it.
                let yank_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    let text = buffer.text().into_string();
                    let start = sel_start.as_usize().saturating_sub(1);
                    let end = sel_end.as_usize().saturating_sub(1);
                    text.chars()
                        .skip(start)
                        .take(end - start + 1)
                        .collect::<String>()
                };
                // Select the range in the model (sel_end + 1 because head is exclusive).
                self.model.update(ctx, |m, ctx| {
                    m.select_at(sel_start, false, ctx);
                    m.set_last_selection_head(sel_end + 1usize, ctx);
                });
                self.model.update(ctx, |m, ctx| m.backspace(ctx));
                if !yank_text.is_empty() {
                    self.yank_buffer = yank_text;
                }
                self.follow_cursor(ctx);
            }
            VimOperator::Yank => {
                // Non-destructive: just capture the selected range.
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                let start = sel_start.as_usize().saturating_sub(1);
                let end = sel_end.as_usize().saturating_sub(1);
                let yank_text: String = buffer_text
                    .chars()
                    .skip(start)
                    .take(end - start + 1)
                    .collect();
                if !yank_text.is_empty() {
                    self.yank_buffer = yank_text;
                }
            }
            _ => {}
        }
        ctx.notify();
    }

    /// Prompt-specific: visual paste is a no-op for TUI; use the plain
    /// `paste` method instead (the TUI has no register system).
    fn visual_paste(
        &mut self,
        _motion_type: MotionType,
        _read_register_name: char,
        _write_register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    /// Prompt-specific: visual text-object selection is a no-op.
    fn visual_text_object(&mut self, _text_object: &VimTextObject, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Jumps ─────────────────────────────────────────────────────────────────

    fn jump_to_first_line(&mut self, ctx: &mut ViewContext<Self>) {
        // `gg` — jump to the very start of the buffer.
        self.model
            .update(ctx, |m, ctx| m.move_to_paragraph_start(ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn jump_to_last_line(&mut self, ctx: &mut ViewContext<Self>) {
        // `G` — jump to the very end of the buffer.
        self.model
            .update(ctx, |m, ctx| m.move_to_paragraph_end(ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: numbered-line jump is a no-op.
    fn jump_to_line(&mut self, _line_number: u32, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    /// Prompt-specific: matching-bracket jump is a no-op.
    fn jump_to_matching_bracket(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    /// Prompt-specific: unmatched-bracket jump is a no-op.
    fn jump_to_unmatched_bracket(&mut self, _bracket: &BracketChar, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    // ── Paste ─────────────────────────────────────────────────────────────────

    fn paste(
        &mut self,
        _count: u32,
        direction: &Direction,
        _register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.yank_buffer.is_empty() {
            ctx.notify();
            return;
        }
        let text = self.yank_buffer.clone();
        match direction {
            Direction::Forward => {
                // `p` — paste after cursor.
                // `move_right` is a no-op when at end-of-line; paste lands
                // after the current character to match vim `p` semantics.
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
            }
            Direction::Backward => {
                // `P` — paste before cursor.
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
            }
        }
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn insert_text(
        &mut self,
        text: &str,
        position: &InsertPosition,
        count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        // For `o`/`O` (LineBelow/LineAbove) positions we skip newline insertion;
        // the mode already switches to Insert via change_mode. Other positions
        // (dot-repeat of plain insertions) use the shared model path.
        match position {
            InsertPosition::LineAbove | InsertPosition::LineBelow => {
                // Prompt-specific: no newline insertion on o/O in TUI.
                if !text.is_empty() {
                    self.model.update(ctx, |m, ctx| m.user_insert(text, ctx));
                    self.follow_cursor(ctx);
                }
            }
            _ => {
                self.model.update(ctx, |m, ctx| {
                    m.vim_insert_text(text, position, count, ctx);
                });
                self.follow_cursor(ctx);
            }
        }
        ctx.notify();
    }

    // ── Miscellaneous ─────────────────────────────────────────────────────────

    fn join_line(&mut self, _count: u32, ctx: &mut ViewContext<Self>) {
        // No-op in TUI prompt.
        ctx.notify();
    }

    fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |m, ctx| m.undo(ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn change_mode(&mut self, _old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>) {
        match new.mode {
            VimMode::Normal => {
                // Clear visual selection anchor when returning to Normal.
                self.visual_selection_anchor = None;
            }
            VimMode::Insert => {
                // Apply cursor movement implied by the entry command (i/a/A/I).
                // `o`/`O` (LineBelow/LineAbove) are no-ops for newline creation
                // in the TUI prompt — the mode switch still happens.
                match &new.position {
                    InsertPosition::AtCursor => {}
                    InsertPosition::AfterCursor => {
                        self.model.update(ctx, |m, ctx| m.move_right(ctx));
                    }
                    InsertPosition::LineEnd => {
                        self.model.update(ctx, |m, ctx| m.move_to_line_end(ctx));
                    }
                    InsertPosition::LineFirstNonWhitespace => {
                        self.model.update(ctx, |m, ctx| {
                            m.vim_move_to_first_nonwhitespace(false, ctx);
                        });
                    }
                    // Prompt-specific: no newline on o/O.
                    InsertPosition::LineAbove | InsertPosition::LineBelow => {}
                }
                // Entering Insert mode clears any visual selection anchor.
                self.visual_selection_anchor = None;
            }
            VimMode::Visual(_) => {
                // Record the cursor position as the visual selection anchor.
                self.visual_selection_anchor = Some(self.cursor_offset(ctx));
            }
            _ => {}
        }
        // Emit a mode-change notification so the footer indicator re-renders.
        ctx.emit(TuiInputViewEvent::VimModeChanged);
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |m, ctx| m.backspace(ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn delete_forward(&mut self, ctx: &mut ViewContext<Self>) {
        // PlainTextEditorModel::delete is in scope via the import above.
        self.model.update(ctx, |m, ctx| {
            m.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        // All escape routing (menu dismissal, shell-mode exit, etc.) is handled
        // by `handle_escape` before the keystroke reaches the vim model.
        // By the time this fires the FSA has already consumed the Escape key
        // (e.g. clearing pending showcmd in Normal mode); a mode transition
        // emits VimModeChanged via `change_mode`.  Nothing more to do here.
        ctx.notify();
    }
}

impl TuiInputView {
    /// Applies a motion-based delete/change/yank operation to the backing model.
    /// Extracted from `operation` to keep the trait impl readable.
    fn apply_motion_operation(
        &mut self,
        operator: &VimOperator,
        motion: &VimMotion,
        count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        match (operator, motion) {
            // ── Delete/change motions ─────────────────────────────────────
            (VimOperator::Delete | VimOperator::Change, VimMotion::Character(char_motion)) => {
                let cmd = match char_motion {
                    CharacterMotion::Right | CharacterMotion::WrappingRight => {
                        TuiEditorCommand::DeleteForward
                    }
                    CharacterMotion::Left | CharacterMotion::WrappingLeft => {
                        TuiEditorCommand::Backspace
                    }
                    _ => return,
                };
                for _ in 0..count.max(1) {
                    self.editor_state
                        .apply_command(&self.model, cmd, self.editor_behavior, ctx);
                }
                self.follow_cursor(ctx);
            }
            (VimOperator::Delete | VimOperator::Change, VimMotion::Word(word_motion)) => {
                let cmd = match word_motion.direction {
                    Direction::Forward => TuiEditorCommand::DeleteWordForward,
                    Direction::Backward => TuiEditorCommand::DeleteWordBackward,
                };
                for _ in 0..count.max(1) {
                    self.editor_state
                        .apply_command(&self.model, cmd, self.editor_behavior, ctx);
                }
                self.follow_cursor(ctx);
            }
            (VimOperator::Delete | VimOperator::Change, VimMotion::Line(LineMotion::End)) => {
                // `d$` / `c$` — kill from cursor to end of line.
                for _ in 0..count.max(1) {
                    if let Some(killed) = self
                        .model
                        .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                    {
                        self.yank_buffer = killed;
                    }
                }
                self.follow_cursor(ctx);
            }
            (VimOperator::Delete | VimOperator::Change, VimMotion::Line(LineMotion::Start)) => {
                // `d0` / `c0` — kill from cursor to start of line.
                for _ in 0..count.max(1) {
                    if let Some(killed) = self
                        .model
                        .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_start(ctx))
                    {
                        self.yank_buffer = killed;
                    }
                }
                self.follow_cursor(ctx);
            }

            // ── Yank motions ──────────────────────────────────────────────
            (VimOperator::Yank, VimMotion::Line(LineMotion::End)) => {
                // `y$` — yank from cursor to end of line, non-destructively.
                let cursor = self.cursor_offset(ctx);
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                let char_idx = cursor.as_usize().saturating_sub(1);
                let yanked: String = buffer_text
                    .chars()
                    .skip(char_idx)
                    .take_while(|&c| c != '\n')
                    .collect();
                if !yanked.is_empty() {
                    self.yank_buffer = yanked;
                }
                // Non-destructive; no buffer mutation.
            }
            (VimOperator::Yank, VimMotion::Word(_)) => {
                // `yw` — yank one word forward from the cursor, non-destructively.
                let cursor = self.cursor_offset(ctx);
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                let char_idx = cursor.as_usize().saturating_sub(1);
                let yanked = yank_word_from_offset(&buffer_text, char_idx);
                if !yanked.is_empty() {
                    self.yank_buffer = yanked;
                }
            }
            (VimOperator::Yank, _) => {
                // Any other yank motion: yank the full buffer.
                let text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    if buffer.is_empty() {
                        String::new()
                    } else {
                        buffer.text().into_string()
                    }
                };
                self.yank_buffer = text;
            }

            // ── Unsupported operators/motions ─────────────────────────────
            _ => {}
        }
    }
}

/// Compute the text that vim's `yw` (word-forward yank) would capture,
/// starting at character index `char_idx` (0-based) in `text`.
///
/// Matches vim's `w`-motion word definition:
/// - From a word character (alphanumeric/underscore): skip word chars, then
///   include any trailing whitespace.
/// - From punctuation: skip non-word/non-whitespace chars, then include
///   any trailing whitespace.
/// - From whitespace: skip all whitespace.
///
/// The returned string is a non-destructive yank that leaves the buffer
/// untouched, so `u` after `yw` does not delete the yanked text.
pub(super) fn yank_word_from_offset(text: &str, char_idx: usize) -> String {
    let chars: Vec<char> = text.chars().skip(char_idx).collect();
    if chars.is_empty() {
        return String::new();
    }
    let mut end = 0;
    let first = chars[0];
    if first.is_whitespace() {
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    } else if first.is_alphanumeric() || first == '_' {
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    } else {
        while end < chars.len()
            && !chars[end].is_alphanumeric()
            && chars[end] != '_'
            && !chars[end].is_whitespace()
        {
            end += 1;
        }
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    }
    chars.into_iter().take(end).collect()
}

#[cfg(test)]
mod tests {
    use super::yank_word_from_offset;

    #[test]
    fn yank_word_from_start_of_word() {
        assert_eq!(yank_word_from_offset("hello world", 0), "hello ");
    }

    #[test]
    fn yank_word_from_middle_of_word() {
        assert_eq!(yank_word_from_offset("hello world", 2), "llo ");
    }

    #[test]
    fn yank_word_from_whitespace() {
        assert_eq!(yank_word_from_offset("hello world", 5), " ");
    }

    #[test]
    fn yank_word_from_last_word() {
        assert_eq!(yank_word_from_offset("hello world", 6), "world");
    }

    #[test]
    fn yank_word_empty_input() {
        assert_eq!(yank_word_from_offset("", 0), "");
    }

    #[test]
    fn yank_word_from_punctuation() {
        assert_eq!(yank_word_from_offset("!abc", 0), "!");
    }
}
