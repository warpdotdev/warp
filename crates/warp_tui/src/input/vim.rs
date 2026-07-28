//! Vim-mode action dispatch for [`TuiInputView`].
//!
//! This module is declared as a private submodule of `view` so that it can
//! access the private fields of [`TuiInputView`] directly, keeping the
//! sizeable vim action dispatch out of the main view file while avoiding
//! the need for extra public accessors.

use vim::vim::{InsertPosition, VimMode};
use warp_editor::model::CoreEditorModel;
use warpui_core::ViewContext;

use super::{TuiInputView, TuiInputViewEvent};
use crate::editor_interaction::TuiEditorCommand;
use crate::tui_vim_input::TuiVimAction;

impl TuiInputView {
    /// Applies a [`TuiVimAction`] — returned by the vim FSA — to the backing
    /// editor model and re-renders.
    ///
    /// `prev_vim_mode` must be captured by the caller **before** it calls the
    /// FSA (`process_char`/`process_special_key`), because the FSA advances its
    /// internal mode as part of that call.  Comparing `self.vim.mode()` inside
    /// this function would always see the post-transition mode and would never
    /// detect a change.
    pub(super) fn apply_vim_action(
        &mut self,
        action: TuiVimAction,
        prev_vim_mode: VimMode,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            TuiVimAction::InsertChar(c) => {
                // Normal character insert (insert mode or insert-mode char from FSA).
                let c_str = c.to_string();
                self.model.update(ctx, |m, ctx| m.user_insert(&c_str, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::InsertText(text) => {
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::Backspace => {
                self.model.update(ctx, |m, ctx| m.backspace(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::DeleteForward => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::DeleteForward,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::DeleteWordBackward => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::DeleteWordBackward,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::DeleteWordForward => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::DeleteWordForward,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveLeft => {
                self.model.update(ctx, |m, ctx| m.move_left(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveRight => {
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveUp => {
                self.model.update(ctx, |m, ctx| m.move_up(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveDown => {
                self.model.update(ctx, |m, ctx| m.move_down(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveWordLeft => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::MoveWordLeft,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            // Both `w` (start of next word) and `e` (end of current word) map
            // to the single `MoveWordRight` editor command; the TUI model does
            // not yet expose a separate end-of-word cursor stop.
            TuiVimAction::MoveWordRightStart | TuiVimAction::MoveWordRightEnd => {
                self.editor_state.apply_command(
                    &self.model,
                    TuiEditorCommand::MoveWordRight,
                    self.editor_behavior,
                    ctx,
                );
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToLineStart => {
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToLineEnd => {
                self.model.update(ctx, |m, ctx| m.move_to_line_end(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToFirstNonWhitespace => {
                self.model.update(ctx, |m, ctx| {
                    m.vim_move_to_first_nonwhitespace(false, ctx);
                });
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToBufferStart => {
                // `gg` — jump to the start of the buffer. Use paragraph
                // navigation which moves past all content to the very beginning.
                self.model
                    .update(ctx, |m, ctx| m.move_to_paragraph_start(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::MoveToBufferEnd => {
                // `G` — jump to the end of the buffer.
                self.model
                    .update(ctx, |m, ctx| m.move_to_paragraph_end(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::KillToLineEnd => {
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                {
                    self.vim.set_yank_buffer(killed);
                }
                self.follow_cursor(ctx);
            }
            TuiVimAction::KillToLineStart => {
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_start(ctx))
                {
                    self.vim.set_yank_buffer(killed);
                }
                self.follow_cursor(ctx);
            }
            TuiVimAction::KillLine => {
                // `dd` — delete the whole current line regardless of cursor column.
                // Move to the start of the visual row first, then kill to the end.
                self.model.update(ctx, |m, ctx| m.move_to_line_start(ctx));
                if let Some(killed) = self
                    .model
                    .update(ctx, |m, ctx| m.kill_to_char_cell_visual_row_end(ctx))
                {
                    self.vim.set_yank_buffer(killed);
                }
                self.follow_cursor(ctx);
            }
            TuiVimAction::ReplaceChar(c) => {
                // `r<char>` — replace the character at the cursor in-place.
                // `replace_char` atomically replaces without changing the cursor
                // position, matching vim's behaviour of staying on the new char.
                self.model.update(ctx, |m, ctx| m.replace_char(c, 1, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::YankToLineEnd => {
                // `y$` — yank from cursor to end of the line, non-destructively.
                // Read the buffer text directly so the undo stack is untouched;
                // the kill+re-insert approach previously broke `u` by leaving
                // the kill/re-insert pair on the undo stack.
                let cursor = self.cursor_offset(ctx);
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                // cursor is a 1-based gap offset; char index = as_usize() - 1
                let char_idx = cursor.as_usize().saturating_sub(1);
                // Yank to end of the current line ('\n' is the line separator).
                let yanked: String = buffer_text
                    .chars()
                    .skip(char_idx)
                    .take_while(|&c| c != '\n')
                    .collect();
                if !yanked.is_empty() {
                    self.vim.set_yank_buffer(yanked);
                }
                // No buffer mutation — undo stack is unchanged.
                self.follow_cursor(ctx);
            }
            TuiVimAction::YankWordForward => {
                // `yw` — yank one word forward from the cursor, non-destructively.
                // Uses vim's `w`-motion word boundary: skip the current token
                // (word chars or punctuation) then include trailing whitespace.
                let cursor = self.cursor_offset(ctx);
                let buffer_text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    buffer.text().into_string()
                };
                let char_idx = cursor.as_usize().saturating_sub(1);
                let yanked = yank_word_from_offset(&buffer_text, char_idx);
                if !yanked.is_empty() {
                    self.vim.set_yank_buffer(yanked);
                }
                // Non-destructive: no cursor or buffer mutation needed.
            }
            TuiVimAction::YankBuffer => {
                // `yy` / visual `y` — yank the full buffer content.
                let text = {
                    let inner = self.model.as_ref(ctx);
                    let buffer = inner.content().as_ref(ctx);
                    if buffer.is_empty() {
                        String::new()
                    } else {
                        buffer.text().into_string()
                    }
                };
                self.vim.set_yank_buffer(text);
                // Stay in current mode (yank is non-destructive).
            }
            TuiVimAction::PasteAfter(text) => {
                // `p` — paste after cursor.
                // `move_right` is a no-op when the cursor is already on the
                // very last character, so at end-of-line this effectively
                // inserts before the last character rather than after. This
                // edge case is a known limitation of the current editor model
                // API and is acceptable for the TUI prompt.
                self.model.update(ctx, |m, ctx| m.move_right(ctx));
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::PasteBefore(text) => {
                self.model.update(ctx, |m, ctx| m.user_insert(&text, ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::Undo => {
                self.model.update(ctx, |m, ctx| m.undo(ctx));
                self.follow_cursor(ctx);
            }
            TuiVimAction::ChangeModeToInsert(position) => {
                // Apply the cursor movement implied by the entry command
                // before handing off to Insert mode.
                match position {
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
                    // `o` / `O` (insert newline below/above) are not meaningful
                    // for TUI single-line prompts; treat as a plain mode switch.
                    InsertPosition::LineAbove | InsertPosition::LineBelow => {}
                }
                // Entering insert mode clears any visual selection anchor.
                self.visual_selection_anchor = None;
                self.follow_cursor(ctx);
            }
            TuiVimAction::ModeTransition => {
                // When entering visual mode, record the cursor position as the
                // visual selection anchor. On any other transition (Escape
                // Visual→Normal, Escape Normal→Normal, etc.), clear the anchor.
                match self.vim.mode() {
                    VimMode::Visual(_) => {
                        self.visual_selection_anchor = Some(self.cursor_offset(ctx));
                    }
                    _ => {
                        self.visual_selection_anchor = None;
                    }
                }
            }
            TuiVimAction::DeleteVisualSelection => {
                // `d`/`c` in visual mode: delete from anchor to current cursor.
                // Vim charwise visual selection is inclusive on both ends, so
                // the character under the cursor is included in the deletion.
                if let Some(anchor) = self.visual_selection_anchor.take() {
                    let cursor = self.cursor_offset(ctx);
                    let (sel_start, sel_end) = if anchor <= cursor {
                        (anchor, cursor)
                    } else {
                        (cursor, anchor)
                    };
                    // Yank [sel_start, sel_end] inclusive — convert 1-based gap
                    // offsets to 0-based char indices and include the cursor char.
                    let yank_text = {
                        let inner = self.model.as_ref(ctx);
                        let buffer = inner.content().as_ref(ctx);
                        let buffer_text = buffer.text().into_string();
                        let start_char = sel_start.as_usize().saturating_sub(1);
                        let end_char = sel_end.as_usize().saturating_sub(1);
                        // Include the character at end_char (inclusive range).
                        buffer_text
                            .chars()
                            .skip(start_char)
                            .take(end_char - start_char + 1)
                            .collect::<String>()
                    };
                    // Establish the inclusive selection in the model: sel_end + 1
                    // because the model selection head is exclusive.
                    self.model.update(ctx, |m, ctx| {
                        m.select_at(sel_start, false, ctx);
                        m.set_last_selection_head(sel_end + 1usize, ctx);
                    });
                    // Delete the selection.
                    self.model.update(ctx, |m, ctx| m.backspace(ctx));
                    if !yank_text.is_empty() {
                        self.vim.set_yank_buffer(yank_text);
                    }
                    self.follow_cursor(ctx);
                }
            }
            TuiVimAction::YankVisualSelection => {
                // `y` in visual mode: yank from anchor to cursor, non-destructively.
                // Vim charwise visual selection is inclusive on both ends.
                if let Some(anchor) = self.visual_selection_anchor.take() {
                    let cursor = self.cursor_offset(ctx);
                    let (sel_start, sel_end) = if anchor <= cursor {
                        (anchor, cursor)
                    } else {
                        (cursor, anchor)
                    };
                    // Extract the selected text from the buffer directly.
                    let buffer_text = {
                        let inner = self.model.as_ref(ctx);
                        let buffer = inner.content().as_ref(ctx);
                        buffer.text().into_string()
                    };
                    let start_char = sel_start.as_usize().saturating_sub(1);
                    let end_char = sel_end.as_usize().saturating_sub(1);
                    // Include the character at end_char (inclusive range).
                    let yank_text: String = buffer_text
                        .chars()
                        .skip(start_char)
                        .take(end_char - start_char + 1)
                        .collect();
                    if !yank_text.is_empty() {
                        self.vim.set_yank_buffer(yank_text);
                    }
                    // Non-destructive: no buffer mutation.
                }
            }
            TuiVimAction::RepeatCount { inner, count } => {
                // Execute the inner action `count` times, passing the same
                // `prev_vim_mode` so that a mode-changing inner action (e.g. a
                // count-prefixed `v` entering visual mode) is detected by the
                // shared emit check at the bottom of this function.
                for _ in 0..count {
                    self.apply_vim_action(*inner.clone(), prev_vim_mode, ctx);
                }
                // Fall through to the shared mode-change emit and ctx.notify()
                // so a mode transition inside a count-prefixed command is never
                // silently skipped.
            }
            // Pending / unhandled — no buffer edit needed.
            TuiVimAction::Pending | TuiVimAction::Unhandled => {}
        }
        // Emit a mode-change notification whenever the vim FSA transitions to a
        // different mode. This lets TuiTerminalSessionView re-render its footer
        // vim-mode indicator without the indicator living in this view's own render tree.
        if self.vim.mode() != prev_vim_mode {
            ctx.emit(TuiInputViewEvent::VimModeChanged);
        }
        ctx.notify();
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
        // Starting on whitespace: skip all whitespace.
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    } else if first.is_alphanumeric() || first == '_' {
        // Starting on a word character: skip word chars, then whitespace.
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
    } else {
        // Starting on punctuation: skip non-word, non-whitespace chars, then whitespace.
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
