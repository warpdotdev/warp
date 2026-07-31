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
//! - `search`, `cycle_search`, `search_word_at_cursor` — no-op.
//! - `visual_paste` — inserts from the local yank buffer (no register system).
//! - `join_line`, `toggle_case`, `keyword_prg`, `ex_command` — no-op.
//! - Scroll helpers (`center_cursor_vertically`, `scroll_half_page_*`) — no-op.
//!

use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
    InsertPosition, LineMotion, ModeTransition, MotionType, VimHandler, VimMode, VimMotion,
    VimOperand, VimOperator, VimTextObject, WordMotion,
};
use warp::editor::{CodeEditorModel, LineBound};
use warp_editor::content::buffer::AutoScrollBehavior;
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui_core::{ModelContext, ViewContext};

use super::TuiInputView;
const MAX_VIM_PASTE_BYTES: usize = 1024 * 1024;

impl VimHandler for TuiInputView {
    // ── Character insertion ───────────────────────────────────────────────────

    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>) {
        if c == '!'
            && !self.is_shell_mode(ctx)
            && self.is_cursor_at_start(ctx)
            && !self
                .input_mode
                .as_ref(ctx)
                .is_terminal_use_active_or_pending()
        {
            self.enter_shell_mode(ctx);
            ctx.notify();
            return;
        }
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

    fn replace_text(
        &mut self,
        text: &str,
        count: u32,
        already_applied: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let repeat_count = count.saturating_sub(u32::from(already_applied));
        self.model.update(ctx, |model, ctx| {
            if repeat_count > 0 {
                model.vim_replace_text(&text.repeat(repeat_count as usize), ctx);
            }
            if !text.is_empty() {
                model.vim_move_horizontal_by_offset(1, &Direction::Backward, false, true, ctx);
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
            LineMotion::Start => model.vim_move_to_line_bound(LineBound::Start, false, ctx),
            LineMotion::FirstNonWhitespace => model.vim_move_to_first_nonwhitespace(false, ctx),
            LineMotion::End => {
                model.vim_move_vertical_by_offset(
                    line_count.saturating_sub(1),
                    TextDirection::Forwards,
                    false,
                    ctx,
                );
                model.vim_move_to_line_bound(LineBound::End, false, ctx);
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
        replacement_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if !matches!(
            operator,
            VimOperator::Delete | VimOperator::Change | VimOperator::Yank
        ) {
            ctx.notify();
            return;
        }
        let motion_type = vim_operand_motion_type(operand);
        let yanked = self.model.update(ctx, |model, ctx| {
            let existing_selections = model.selections(ctx).clone();
            select_vim_operand(model, operator, operand_count, operand, ctx);

            let selected_text = model
                .content()
                .as_ref(ctx)
                .selected_text_as_plain_text(model.buffer_selection_model().clone(), ctx)
                .into_string();

            match operator {
                VimOperator::Delete | VimOperator::Change if !selected_text.is_empty() => {
                    if *operator == VimOperator::Change && matches!(operand, VimOperand::Line) {
                        model.vim_change_line_with_smart_indent(ctx);
                    } else {
                        model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                    }
                    if *operator == VimOperator::Change && !replacement_text.is_empty() {
                        model.user_insert(replacement_text, ctx);
                    }
                }
                VimOperator::Yank => {
                    model.vim_set_selections(existing_selections, AutoScrollBehavior::None, ctx);
                }
                VimOperator::Delete
                | VimOperator::Change
                | VimOperator::ToggleCase
                | VimOperator::Uppercase
                | VimOperator::Lowercase
                | VimOperator::ToggleComment
                | VimOperator::Indent
                | VimOperator::Dedent => {}
            }
            if selected_text.is_empty() && motion_type == MotionType::Linewise {
                "\n".to_owned()
            } else {
                selected_text
            }
        });
        if !yanked.is_empty() {
            self.yank_buffer = yanked;
            self.yank_motion_type = motion_type;
        }
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn replace_char(
        &mut self,
        c: char,
        char_count: u32,
        advance: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            if advance {
                model.vim_replace_text(&c.to_string(), ctx);
            } else {
                model.replace_char(c, char_count, ctx);
            }
        });
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
        motion_type: MotionType,
        _register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let yanked = self.model.update(ctx, |model, ctx| {
            model.vim_visual_selection_range(
                motion_type,
                operator.includes_trailing_newline(),
                ctx,
            );
            let selected_text = model
                .content()
                .as_ref(ctx)
                .selected_text_as_plain_text(model.buffer_selection_model().clone(), ctx)
                .into_string();
            match operator {
                VimOperator::Delete | VimOperator::Change => {
                    model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                }
                VimOperator::Yank => model.vim_clear_selections(ctx),
                VimOperator::ToggleCase
                | VimOperator::Uppercase
                | VimOperator::Lowercase
                | VimOperator::ToggleComment
                | VimOperator::Indent
                | VimOperator::Dedent => model.vim_clear_selections(ctx),
            }
            selected_text
        });
        if !yanked.is_empty() {
            self.yank_buffer = yanked;
            self.yank_motion_type = motion_type;
        }
        self.follow_cursor(ctx);
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
        self.model
            .update(ctx, |model, ctx| model.jump_to_line_column(0, Some(0), ctx));
        self.follow_cursor(ctx);
        ctx.notify();
    }

    fn jump_to_last_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.vim_move_to_last_line(ctx);
        });
        self.follow_cursor(ctx);
        ctx.notify();
    }
    fn jump_to_line(&mut self, line_number: u32, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            let last_line = model.content().as_ref(ctx).max_point().row as usize;
            let line = line_number.max(1) as usize;
            model.jump_to_line_column(line.min(last_line), Some(0), ctx);
        });
        self.follow_cursor(ctx);
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
        count: u32,
        direction: &Direction,
        _register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.yank_buffer.is_empty() {
            ctx.notify();
            return;
        }
        let text = bounded_repeated_text(&self.yank_buffer, count);
        if self.yank_motion_type == MotionType::Linewise {
            let text = text.trim_matches('\n');
            let insertion = match direction {
                Direction::Forward => format!("\n{text}"),
                Direction::Backward => format!("{text}\n"),
            };
            self.model.update(ctx, |model, ctx| {
                model.vim_move_to_line_bound(
                    match direction {
                        Direction::Forward => LineBound::End,
                        Direction::Backward => LineBound::Start,
                    },
                    false,
                    ctx,
                );
                model.user_insert(&insertion, ctx);
            });
        } else {
            match direction {
                Direction::Forward => {
                    self.model.update(ctx, |model, ctx| model.move_right(ctx));
                    self.model
                        .update(ctx, |model, ctx| model.user_insert(&text, ctx));
                }
                Direction::Backward => {
                    self.model
                        .update(ctx, |model, ctx| model.user_insert(&text, ctx));
                }
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
        self.model.update(ctx, |model, ctx| {
            model.vim_insert_text(text, position, count, ctx);
        });
        self.follow_cursor(ctx);
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

    fn change_mode(&mut self, old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>) {
        match new.mode {
            VimMode::Normal => {
                if *old == VimMode::Insert {
                    self.model.update(ctx, |model, ctx| {
                        model.vim_move_horizontal_by_offset(
                            1,
                            &Direction::Backward,
                            false,
                            true,
                            ctx,
                        );
                    });
                }
            }
            VimMode::Insert => {
                // Apply cursor movement or newline insertion implied by the
                // entry command (i/a/A/I/o/O).
                match &new.position {
                    InsertPosition::AtCursor => {}
                    InsertPosition::AfterCursor => {
                        self.model.update(ctx, |m, ctx| m.move_right(ctx));
                    }
                    InsertPosition::LineEnd => {
                        self.model.update(ctx, |model, ctx| {
                            model.vim_move_to_line_bound(LineBound::End, false, ctx);
                        });
                    }
                    InsertPosition::LineFirstNonWhitespace => {
                        self.model.update(ctx, |m, ctx| {
                            m.vim_move_to_first_nonwhitespace(false, ctx);
                        });
                    }
                    InsertPosition::LineAbove => {
                        self.model
                            .update(ctx, |model, ctx| model.vim_newline(true, ctx));
                    }
                    InsertPosition::LineBelow => {
                        self.model.update(ctx, |model, ctx| {
                            model.vim_newline(false, ctx);
                            model.move_right(ctx);
                        });
                    }
                }
            }
            VimMode::Visual(_) => {
                self.model.update(ctx, |model, ctx| {
                    model.vim_set_visual_tail_to_selection_heads(ctx);
                });
            }
            VimMode::Replace => {
                self.model.update(ctx, |model, ctx| {
                    model.vim_enforce_cursor_line_cap(ctx);
                });
            }
        }
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
        // (e.g. clearing pending showcmd in Normal mode); the dispatch wrapper
        // observes any mode transition and notifies the footer. Nothing more
        // to do here.
        ctx.notify();
    }
}

fn vim_operand_motion_type(operand: &VimOperand) -> MotionType {
    match operand {
        VimOperand::Motion { motion_type, .. } => *motion_type,
        VimOperand::Line => MotionType::Linewise,
        VimOperand::TextObject(_) => MotionType::Charwise,
    }
}
fn bounded_repeated_text(text: &str, count: u32) -> String {
    let max_count = (MAX_VIM_PASTE_BYTES / text.len().max(1)).max(1);
    text.repeat((count as usize).min(max_count))
}

fn select_vim_operand(
    model: &mut CodeEditorModel,
    operator: &VimOperator,
    operand_count: u32,
    operand: &VimOperand,
    ctx: &mut ModelContext<CodeEditorModel>,
) {
    match operand {
        VimOperand::Motion {
            motion,
            motion_type,
        } => match motion {
            VimMotion::Character(motion) => {
                model.vim_select_for_char_motion(motion, motion_type, operator, operand_count, ctx);
            }
            VimMotion::Word(motion) => {
                model.vim_select_for_word_motion(motion, operand_count, motion_type, operator, ctx);
            }
            VimMotion::Line(motion) => {
                model.vim_select_for_line_motion(motion, operand_count, motion_type, operator, ctx);
            }
            VimMotion::FirstNonWhitespace(motion) => {
                model.vim_select_for_first_nonwhitespace_motion(
                    motion,
                    motion_type,
                    operator,
                    operand_count,
                    ctx,
                );
            }
            VimMotion::Paragraph(direction) => {
                model.vim_move_by_paragraph(operand_count, direction, true, ctx);
                if *motion_type == MotionType::Linewise {
                    model.vim_extend_selection_linewise(
                        operator.includes_trailing_newline(),
                        *operator == VimOperator::Delete,
                        ctx,
                    );
                }
            }
            VimMotion::JumpToLastLine => {
                model.vim_select_to_buffer_end(ctx);
                if *motion_type == MotionType::Linewise {
                    model.vim_extend_selection_linewise(
                        operator.includes_trailing_newline(),
                        *operator == VimOperator::Delete,
                        ctx,
                    );
                }
            }
            VimMotion::JumpToFirstLine => {
                model.vim_select_to_buffer_start(ctx);
                if *motion_type == MotionType::Linewise {
                    model.vim_extend_selection_linewise(
                        operator.includes_trailing_newline(),
                        *operator == VimOperator::Delete,
                        ctx,
                    );
                }
            }
            VimMotion::FindChar(_)
            | VimMotion::JumpToMatchingBracket
            | VimMotion::JumpToUnmatchedBracket(_) => {}
            VimMotion::JumpToLine(line_number) => {
                model.vim_select_to_line(*line_number, motion_type, operator, ctx);
            }
        },
        VimOperand::Line => {
            if operand_count > 1 {
                model.vim_move_vertical_by_offset(
                    operand_count - 1,
                    TextDirection::Forwards,
                    true,
                    ctx,
                );
            }
            model.vim_extend_selection_linewise(
                operator.includes_trailing_newline(),
                *operator == VimOperator::Delete,
                ctx,
            );
        }
        VimOperand::TextObject(text_object) => {
            model.vim_select_text_object(text_object, Some(operator), ctx);
        }
    }
}
