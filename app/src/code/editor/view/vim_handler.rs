use vim::handler::{
    self, VimBufferOps, YankedText, apply_mode_change, apply_operator, apply_visual_operator,
    apply_visual_paste,
};
use vim::vim::{
    CharacterMotion, Direction, InsertPosition, ModeTransition, MotionType, VimHandler, VimMode,
    VimOperand, VimOperator,
};
use warp_editor::content::buffer::{BufferEditAction, EditOrigin, VimInsertPoint};
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::render::model::AutoScrollMode;
use warp_editor::selection::{TextDirection, TextUnit};
use warpui::units::IntoPixels;
use warpui::{SingletonEntity, ViewContext};

use super::{CodeEditorEvent, CodeEditorView};
use crate::code::editor::find::view::Event as FindViewEvent;
use crate::view_components::find::FindDirection;
use crate::vim_registers::{RegisterContent, VimRegisters};

impl VimHandler for CodeEditorView {
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

    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>) {
        self.user_insert(&c.to_string(), ctx);
    }

    fn keyword_prg(&mut self, _ctx: &mut ViewContext<Self>) {
        // no-op
    }

    fn operation(
        &mut self,
        operator: &VimOperator,
        operand_count: u32,
        operand: &VimOperand,
        register_name: char,
        replacement_text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
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
        Self::write_yanked_register(register_name, yanked, ctx);
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

        // Explicit call to ctx.notify() in the case that we don't make any updates to the model
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
        ctx.notify();
    }

    fn search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        self.last_search_direction = *direction;
        self.show_find_bar(ctx);
    }

    fn cycle_search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        let Some(find_bar) = &self.find_bar else {
            return;
        };

        if !self.searcher.as_ref(ctx).has_query() {
            return;
        }

        if !find_bar.as_ref(ctx).is_open() {
            find_bar.update(ctx, |find_bar, _| find_bar.set_open(true));
        }

        // Vim-like behavior:
        // 'n' (Forward) repeats in the same direction
        // 'N' (Backward) reverses the last direction
        let effective_dir = match (direction, self.last_search_direction) {
            (Direction::Forward, dir) => dir,
            (Direction::Backward, Direction::Backward) => Direction::Forward,
            (Direction::Backward, Direction::Forward) => Direction::Backward,
        };

        // Map vim::Direction to a FindDirection
        let find_dir = match effective_dir {
            Direction::Forward => FindDirection::Down,
            Direction::Backward => FindDirection::Up,
        };

        find_bar.update(ctx, |_find_bar, ctx| {
            ctx.emit(FindViewEvent::NextMatch {
                direction: find_dir,
            })
        });
    }

    fn search_word_at_cursor(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        self.last_search_direction = *direction;
        let Some(find_bar) = &self.find_bar else {
            return;
        };

        let word_under_cursor = self.model.as_ref(ctx).word_under_cursor_for_search(ctx);

        if let Some(word) = word_under_cursor
            && !word.trim().is_empty()
        {
            find_bar.update(ctx, |find_bar, ctx| {
                find_bar.set_find_query(ctx, &word);
                find_bar.set_open(true);
                // Disable the find input; the search is already defined.
                find_bar.set_find_input_editable(ctx, false);
            });

            self.searcher
                .update(ctx, |searcher, _| searcher.set_auto_select(true));
            self.run_find(&word, ctx);
            ctx.notify();
        }
    }

    fn ex_command(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn visual_operator(
        &mut self,
        operator: &VimOperator,
        motion_type: MotionType,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let yanked = self.model.update(ctx, |model, ctx| {
            apply_visual_operator(model, operator, motion_type, ctx)
        });
        Self::write_yanked_register(register_name, yanked, ctx);
        ctx.notify();
    }

    fn visual_paste(
        &mut self,
        motion_type: MotionType,
        read_register_name: char,
        write_register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(RegisterContent {
            text,
            motion_type: yanked_motion_type,
        }) = VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
            registers.read_from_register(read_register_name, ctx)
        })
        else {
            return;
        };

        let yanked = self.model.update(ctx, |model, ctx| {
            apply_visual_paste(model, motion_type, &text, yanked_motion_type, ctx)
        });
        Self::write_yanked_register(write_register_name, yanked, ctx);
    }

    fn paste(
        &mut self,
        count: u32,
        direction: &Direction,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(RegisterContent { text, motion_type }) = VimRegisters::handle(ctx)
            .update(ctx, |registers, ctx| {
                registers.read_from_register(register_name, ctx)
            })
        else {
            return;
        };

        // For linewise cursor positioning, compute how many leading whitespace characters are at
        // the start of the first inserted line.
        let leading_ws = if motion_type == MotionType::Linewise {
            text.chars()
                .take_while(|c| c.is_whitespace() && *c != '\n')
                .count()
        } else {
            0
        };

        let text = match motion_type {
            MotionType::Charwise => text,
            MotionType::Linewise => match direction {
                Direction::Backward => {
                    // 'P' - paste above current line
                    // Insert the text followed by a newline to push current line down
                    trim_one_end_match(&text, '\n').to_owned() + "\n"
                }
                Direction::Forward => {
                    // 'p' - paste below current line
                    "\n".to_owned() + trim_one_end_match(&text, '\n')
                }
            },
        };

        let insert_text = text.repeat(count as usize);

        let (insert_point, cursor_offset_len) = match motion_type {
            MotionType::Charwise => match direction {
                Direction::Backward => (VimInsertPoint::BeforeCursor, insert_text.len() - 1),
                Direction::Forward => (VimInsertPoint::AtCursor, insert_text.len() - 1),
            },
            MotionType::Linewise => match direction {
                Direction::Backward => (VimInsertPoint::LineStart, leading_ws),
                // For linewise "p", offset the cursor by 1 to get onto the new line, then by the line's leading whitespace.
                Direction::Forward => (VimInsertPoint::LineEnd, 1 + leading_ws),
            },
        };

        self.model.update(ctx, |model, ctx| {
            let selection_model = model.buffer_selection_model().clone();
            model.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::VimEvent {
                            text: insert_text,
                            insert_point,
                            cursor_offset_len,
                        },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        });
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
    }

    fn toggle_case(&mut self, char_count: u32, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.vim_toggle_case_chars(char_count, ctx);
        });
    }

    fn join_line(&mut self, mut count: u32, ctx: &mut ViewContext<Self>) {
        // 1J joins two lines, which is the same as 2J.
        if count == 1 {
            count = 2;
        }

        self.model.update(ctx, |model, ctx| {
            let buffer = model.content().as_ref(ctx);
            let current_selections = model.selections(ctx);
            let mut replacement_ranges = Vec::new();

            // For each selection, find `count` newlines to replace with spaces
            for selection in current_selections.iter() {
                let start_offset = selection.head;
                let mut current_offset = start_offset;
                let mut newlines_found = 0;

                while newlines_found < count.saturating_sub(1) {
                    let Some(ch) = buffer.char_at(current_offset) else {
                        break;
                    };

                    if ch == '\n' {
                        newlines_found += 1;
                        let mut range_end = current_offset + 1;

                        // Trim whitespace from the start of the next line
                        while range_end < buffer.max_charoffset() {
                            match buffer.char_at(range_end) {
                                Some(ch) if ch.is_whitespace() && ch != '\n' => range_end += 1,
                                _ => break,
                            }
                        }

                        replacement_ranges.push((current_offset, range_end));
                        current_offset = range_end;
                    } else {
                        current_offset += 1;
                    }
                }
            }

            // If we have edits, update the model
            if let Ok(edits) = vec1::Vec1::try_from_vec(
                replacement_ranges
                    .into_iter()
                    .map(|(start, end)| (" ".to_string(), start..end))
                    .collect(),
            ) {
                let selection_model = model.buffer_selection_model().clone();
                model.update_content(
                    |mut content, ctx| {
                        content.apply_edit(
                            BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                            EditOrigin::UserInitiated,
                            selection_model,
                            ctx,
                        );
                    },
                    ctx,
                );
            }
        });
    }

    fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.undo(ctx);

            // Clear selections after undo, for things like delete/change operations which
            // modify the editor state by changing selections and then making an insert/delete.
            //
            // TODO(liliwilson): this only works for the vim undo: cmd+Z and cmd+shift+z will undo
            // the operation but not the selection. Need a deeper change to the buffer model
            // undostack to support this.
            model.vim_clear_selections(ctx);
        });
    }

    fn change_mode(&mut self, old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            apply_mode_change(model, old, new, ctx);
        });
        ctx.notify();
    }

    fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.backspace(ctx);
        });
    }

    fn delete_forward(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
        });
    }

    fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        match self.vim_mode(ctx) {
            Some(VimMode::Normal) => {
                ctx.emit(CodeEditorEvent::VimEscapeInNormalMode);
            }
            _ => {
                self.vim_escape(ctx);
            }
        }
    }

    fn goto_definition(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeEditorEvent::VimGotoDefinition);
    }

    fn find_references(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeEditorEvent::VimFindReferences);
    }

    fn show_hover(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeEditorEvent::VimShowHover);
    }

    fn center_cursor_vertically(&mut self, ctx: &mut ViewContext<Self>) {
        let cursor_offset = self
            .model
            .as_ref(ctx)
            .buffer_selection_model()
            .as_ref(ctx)
            .first_selection_head();
        self.model
            .as_ref(ctx)
            .render_state()
            .clone()
            .update(ctx, |render_state, _ctx| {
                render_state.request_autoscroll_to(AutoScrollMode::PositionOffsetInViewportCenter(
                    cursor_offset,
                ));
            });
    }

    fn scroll_half_page_down(&mut self, count: u32, ctx: &mut ViewContext<Self>) {
        self.scroll_half_page(count, TextDirection::Forwards, ctx);
    }

    fn scroll_half_page_up(&mut self, count: u32, ctx: &mut ViewContext<Self>) {
        self.scroll_half_page(count, TextDirection::Backwards, ctx);
    }
}

impl CodeEditorView {
    fn write_yanked_register(
        register_name: char,
        yanked: Option<YankedText>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(yanked) = yanked else {
            return;
        };
        VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
            registers.write_to_register(register_name, yanked.text, yanked.motion_type, ctx);
        });
    }

    /// Implements `<C-d>` and `<C-u>`. Without a count, scrolls by half the
    /// viewport; with a count > 1, scrolls by that many lines (matching vim's
    /// `n<C-d>` / `n<C-u>` behavior).
    fn scroll_half_page(
        &mut self,
        count: u32,
        direction: TextDirection,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = self.model.as_ref(ctx);
        let lines = if count > 1 {
            count as usize
        } else {
            (model.lines_in_viewport(ctx) / 2).max(1)
        };
        let signed_lines = match direction {
            TextDirection::Forwards => -(lines as f32),
            TextDirection::Backwards => lines as f32,
        };
        let scroll_pixels = (signed_lines * model.line_height(ctx)).into_pixels();
        self.model.update(ctx, |model, ctx| {
            model.vim_move_vertical_by_offset(lines as u32, direction, false, ctx);
            model.render_state().update(ctx, |render_state, ctx| {
                render_state.scroll(scroll_pixels, ctx);
            });
        });
    }
}

/// Like [`str::trim_end_matches`] except that it only trims up to a single instance.
fn trim_one_end_match(s: &str, ch: char) -> &str {
    if s.ends_with(ch) {
        &s[..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
#[path = "vim_handler_tests.rs"]
mod tests;
