//! TUI prompt Vim keybindings on the shared snapshot handler.

use vim::handler::{self, VimBufferOps, apply_mode_change, apply_operator, apply_visual_operator};
use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, InsertPosition, ModeTransition,
    MotionType, VimHandler, VimMode, VimOperand, VimOperator, VimTextObject,
};
use warp::editor::LineBound;
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui_core::ViewContext;

use super::TuiInputView;
const MAX_VIM_PASTE_BYTES: usize = 1024 * 1024;

impl VimHandler for TuiInputView {
    fn map_vim_snapshot(
        &mut self,
        ctx: &mut ViewContext<Self>,
        f: impl FnOnce(&mut vim::handler::VimSnapshot),
    ) {
        self.model.update(ctx, |model, ctx| {
            let mut snap = model.snapshot(ctx);
            f(&mut snap);
            model.set_selections(&snap.carets, ctx);
        });
    }

    fn after_vim_motion(&mut self, ctx: &mut ViewContext<Self>) {
        self.follow_cursor(ctx);
        ctx.notify();
    }

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
                handler::move_char(model, 1, &CharacterMotion::Left, false, ctx);
            }
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
        _keep_selection: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    /// Prompt-specific: `{` / `}` are no-ops — no paragraph structure.
    fn navigate_paragraph(
        &mut self,
        _count: u32,
        _direction: &Direction,
        _keep_selection: bool,
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
        if !tui_prompt_supports_operator(operator) {
            ctx.notify();
            return;
        }
        let yanked = self.model.update(ctx, |model, ctx| {
            apply_operator(
                model,
                operator,
                operand_count,
                operand,
                replacement_text,
                ctx,
            )
        });
        if let Some(yanked) = yanked {
            self.yank_buffer = yanked.text;
            self.yank_motion_type = yanked.motion_type;
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
        if !tui_prompt_supports_operator(operator) {
            ctx.notify();
            return;
        }
        let yanked = self.model.update(ctx, |model, ctx| {
            apply_visual_operator(model, operator, motion_type, ctx)
        });
        if let Some(yanked) = yanked {
            self.yank_buffer = yanked.text;
            self.yank_motion_type = yanked.motion_type;
        }
        self.follow_cursor(ctx);
        ctx.notify();
    }

    /// Prompt-specific: visual paste is a no-op for TUI; use the plain `paste` method instead.
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

    /// Prompt-specific: matching-bracket jump is a no-op.
    fn jump_to_matching_bracket(&mut self, _keep_selection: bool, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    /// Prompt-specific: unmatched-bracket jump is a no-op.
    fn jump_to_unmatched_bracket(
        &mut self,
        _bracket: &BracketChar,
        _keep_selection: bool,
        ctx: &mut ViewContext<Self>,
    ) {
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
        self.model.update(ctx, |model, ctx| {
            apply_mode_change(model, old, new, ctx);
            // Char-cell `vim_newline(false)` leaves the cursor on the original line.
            if new.mode == VimMode::Insert && new.position == InsertPosition::LineBelow {
                model.move_right(ctx);
            }
        });
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

fn tui_prompt_supports_operator(operator: &VimOperator) -> bool {
    matches!(
        operator,
        VimOperator::Delete | VimOperator::Change | VimOperator::Yank
    )
}

fn bounded_repeated_text(text: &str, count: u32) -> String {
    let max_count = (MAX_VIM_PASTE_BYTES / text.len().max(1)).max(1);
    text.repeat((count as usize).min(max_count))
}
