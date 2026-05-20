use super::{CodeEditorEvent, CodeEditorView};
use crate::code::editor::{
    find::view::Event as FindViewEvent,
    model::{CaseTransform, CodeEditorModel, LineBound},
};
use crate::{
    view_components::find::FindDirection,
    vim_registers::{RegisterContent, VimRegisters},
};
use vim::vim::{
    BracketChar, CharacterMotion, Direction, FindCharMotion, FirstNonWhitespaceMotion,
    InsertPosition, LineMotion, ModeTransition, MotionType, TextObjectType, VimHandler, VimMode,
    VimMotion, VimOperand, VimOperator, VimTextObject, WordMotion,
};
use warp_editor::{
    content::buffer::{
        AutoScrollBehavior, BufferEditAction, EditOrigin, SelectionOffsets,
        ToBufferCharOffset as _, VimInsertPoint,
    },
    model::{CoreEditorModel, PlainTextEditorModel},
    render::model::AutoScrollMode,
    selection::{TextDirection, TextUnit},
};
use warpui::{text::point::Point, units::IntoPixels, SingletonEntity, ViewContext};

impl VimHandler for CodeEditorView {
    fn insert_char(&mut self, c: char, ctx: &mut ViewContext<Self>) {
        self.user_insert(&c.to_string(), ctx);
    }

    fn keyword_prg(&mut self, _ctx: &mut ViewContext<Self>) {
        // no-op
    }

    fn navigate_char(
        &mut self,
        count: u32,
        character_motion: &CharacterMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| match character_motion {
            CharacterMotion::Right => {
                model.vim_move_horizontal_by_offset(count, &Direction::Forward, false, true, ctx);
            }
            CharacterMotion::Up => {
                model.vim_move_vertical_by_offset(count, TextDirection::Backwards, false, ctx);
            }
            CharacterMotion::Down => {
                model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx);
            }
            CharacterMotion::Left => {
                model.vim_move_horizontal_by_offset(count, &Direction::Backward, false, true, ctx);
            }
            CharacterMotion::WrappingLeft => {
                model.vim_move_horizontal_by_offset(count, &Direction::Backward, false, false, ctx);
            }
            CharacterMotion::WrappingRight => {
                model.vim_move_horizontal_by_offset(count, &Direction::Forward, false, false, ctx);
            }
        });
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
    }

    fn navigate_line(&mut self, line_count: u32, motion: &LineMotion, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            match motion {
                LineMotion::Start => model.vim_move_to_line_bound(LineBound::Start, false, ctx),
                LineMotion::FirstNonWhitespace => model.vim_move_to_first_nonwhitespace(false, ctx),
                LineMotion::End => {
                    // Only moving to the end of the line ($) uses number-repeat (the line-count var)
                    model.vim_move_vertical_by_offset(
                        line_count.saturating_sub(1),
                        TextDirection::Forwards,
                        false,
                        ctx,
                    );
                    model.vim_move_to_line_bound(LineBound::End, false, ctx);
                }
            }
        })
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
                    model.vim_move_vertical_by_offset(count, TextDirection::Forwards, false, ctx)
                }
                FirstNonWhitespaceMotion::DownMinusOne => model.vim_move_vertical_by_offset(
                    count - 1,
                    TextDirection::Forwards,
                    false,
                    ctx,
                ),
            }

            model.vim_move_to_first_nonwhitespace(false, ctx);
        })
    }

    fn find_char(
        &mut self,
        occurrence_count: u32,
        find_char_motion: &FindCharMotion,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            model.vim_find_char(
                false, /* keep_selection */
                occurrence_count,
                find_char_motion,
                ctx,
            );
        });
    }

    fn navigate_paragraph(
        &mut self,
        count: u32,
        direction: &Direction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            model.vim_move_by_paragraph(count, direction, false, ctx);
        });
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
        // Selection logic is almost the same for all operators, so capture that in a closure first.
        let selection_change =
            |model: &mut CodeEditorModel, ctx: &mut warpui::ModelContext<CodeEditorModel>| {
                match operand {
                    VimOperand::Motion {
                        motion,
                        motion_type,
                    } => {
                        match motion {
                            VimMotion::Character(char_motion) => {
                                model.vim_select_for_char_motion(
                                    char_motion,
                                    motion_type,
                                    operator,
                                    operand_count,
                                    ctx,
                                );
                            }
                            VimMotion::Word(word_motion) => {
                                model.vim_select_for_word_motion(
                                    word_motion,
                                    operand_count,
                                    motion_type,
                                    operator,
                                    ctx,
                                );
                            }
                            VimMotion::Line(line_motion) => {
                                model.vim_select_for_line_motion(
                                    line_motion,
                                    operand_count,
                                    motion_type,
                                    operator,
                                    ctx,
                                );
                            }
                            VimMotion::FirstNonWhitespace(nonws_motion) => {
                                model.vim_select_for_first_nonwhitespace_motion(
                                    nonws_motion,
                                    motion_type,
                                    operator,
                                    operand_count,
                                    ctx,
                                );
                            }
                            VimMotion::Paragraph(direction) => {
                                model.vim_move_by_paragraph(operand_count, direction, true, ctx);
                                if *motion_type == MotionType::Linewise {
                                    let include_newline = *operator != VimOperator::Change;
                                    model.vim_extend_selection_linewise(include_newline, ctx);
                                }
                            }
                            VimMotion::JumpToLastLine => {
                                model.vim_select_to_buffer_end(ctx);
                                if *motion_type == MotionType::Linewise {
                                    model.vim_extend_selection_linewise(
                                        *operator != VimOperator::Change,
                                        ctx,
                                    );
                                }
                            }
                            VimMotion::JumpToFirstLine => {
                                model.vim_select_to_buffer_start(ctx);
                                if *motion_type == MotionType::Linewise {
                                    model.vim_extend_selection_linewise(
                                        *operator != VimOperator::Change,
                                        ctx,
                                    );
                                }
                            }
                            VimMotion::FindChar(m) => {
                                // Extend selection to the found character according to the motion
                                model.vim_find_char(
                                    true, /* keep_selection */
                                    operand_count,
                                    m,
                                    ctx,
                                );
                            }
                            VimMotion::JumpToLine(line_number) => {
                                let buffer = model.content().as_ref(ctx);
                                let selection_model = model.buffer_selection_model().as_ref(ctx);
                                let current_selections = selection_model.selection_offsets();

                                let new_selections = current_selections.mapped(|selection| {
                                    let cursor_pos = selection.head;
                                    let target_pos =
                                        Point::new(*line_number, 0).to_buffer_char_offset(buffer);

                                    SelectionOffsets {
                                        head: target_pos,
                                        tail: cursor_pos,
                                    }
                                });

                                model.vim_set_selections(
                                    new_selections,
                                    AutoScrollBehavior::Selection,
                                    ctx,
                                );

                                if *motion_type == MotionType::Linewise {
                                    let include_newline = *operator != VimOperator::Change;
                                    model.vim_extend_selection_linewise(include_newline, ctx);
                                }
                            }
                            _ => {
                                // TODO: Implement other motions (find char, brackets, etc.)
                            }
                        }
                    }
                    VimOperand::Line => {
                        // Extend selection down by count-1 lines
                        if operand_count > 1 {
                            model.vim_move_vertical_by_offset(
                                operand_count - 1,
                                TextDirection::Forwards,
                                true,
                                ctx,
                            );
                        }

                        let include_newline = operator != &VimOperator::Change
                            && operator != &VimOperator::ToggleComment;
                        model.vim_extend_selection_linewise(include_newline, ctx);
                    }
                    VimOperand::TextObject(text_object) => {
                        model.vim_select_text_object(text_object, Some(operator), ctx);
                    }
                }
            };

        let motion_type = match operand {
            VimOperand::Motion { motion_type, .. } => *motion_type,
            VimOperand::TextObject(text_object) => match text_object {
                VimTextObject {
                    object_type: TextObjectType::Paragraph,
                    ..
                } => MotionType::Linewise,
                _ => MotionType::Charwise,
            },
            VimOperand::Line => MotionType::Linewise,
        };

        match operator {
            VimOperator::Delete | VimOperator::Change => {
                self.model.update(ctx, |model, ctx| {
                    selection_change(model, ctx);

                    // Copy selection to vim register before modifying
                    let buffer = model.content().as_ref(ctx);
                    let selection_model = model.buffer_selection_model().clone();
                    let selected_text = buffer
                        .selected_text_as_plain_text(selection_model, ctx)
                        .into_string();
                    if !selected_text.is_empty() {
                        VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
                            registers.write_to_register(
                                register_name,
                                selected_text,
                                motion_type,
                                ctx,
                            );
                        });

                        if *operator == VimOperator::Change && motion_type == MotionType::Linewise {
                            // Use smart indent to position the cursor when changing the entire
                            // line.
                            model.vim_change_line_with_smart_indent(ctx);
                        } else {
                            model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                            // Insert replacement text if provided
                            if *operator == VimOperator::Change && !replacement_text.is_empty() {
                                model.insert(replacement_text, EditOrigin::UserInitiated, ctx);
                            }
                            if motion_type == MotionType::Linewise {
                                model.vim_move_to_line_bound(LineBound::Start, false, ctx);
                            }
                        }
                    }
                });
            }
            VimOperator::Yank => {
                self.model.update(ctx, |model, ctx| {
                    // Store existing selections to restore after yank
                    let existing_selections = model.selections(ctx).clone();
                    selection_change(model, ctx);

                    // Copy selection to vim register
                    let buffer = model.content().as_ref(ctx);
                    let selection_model = model.buffer_selection_model().clone();
                    let selected_text = buffer
                        .selected_text_as_plain_text(selection_model, ctx)
                        .into_string();
                    if !selected_text.is_empty() {
                        VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
                            registers.write_to_register(
                                register_name,
                                selected_text,
                                motion_type,
                                ctx,
                            );
                        });
                    }

                    match operand {
                        VimOperand::TextObject(_) => {
                            // For text objects, move to the start (min) of the selected range
                            let starts = model
                                .buffer_selection_model()
                                .as_ref(ctx)
                                .selection_offsets()
                                .mapped(|selection| {
                                    let start = selection.head.min(selection.tail);
                                    SelectionOffsets {
                                        head: start,
                                        tail: start,
                                    }
                                });
                            model.vim_set_selections(starts, AutoScrollBehavior::None, ctx);
                        }
                        _ => {
                            model.vim_set_selections(
                                existing_selections,
                                AutoScrollBehavior::None,
                                ctx,
                            );
                        }
                    }
                });
            }
            VimOperator::ToggleCase => {
                self.model.update(ctx, |model, ctx| {
                    model.apply_case_transformation_with_selection_change(
                        selection_change,
                        CaseTransform::Toggle,
                        ctx,
                    );
                });
            }
            VimOperator::Uppercase => {
                self.model.update(ctx, |model, ctx| {
                    model.apply_case_transformation_with_selection_change(
                        selection_change,
                        CaseTransform::Uppercase,
                        ctx,
                    );
                });
            }
            VimOperator::Lowercase => {
                self.model.update(ctx, |model, ctx| {
                    model.apply_case_transformation_with_selection_change(
                        selection_change,
                        CaseTransform::Lowercase,
                        ctx,
                    );
                });
            }
            VimOperator::ToggleComment => {
                self.model.update(ctx, |model, ctx| {
                    let existing_selections = model.selections(ctx).clone();
                    selection_change(model, ctx);
                    model.toggle_comments(ctx);

                    if motion_type == MotionType::Linewise {
                        model.vim_move_to_first_nonwhitespace(false, ctx);
                    } else {
                        model.vim_set_selections(
                            existing_selections,
                            AutoScrollBehavior::None,
                            ctx,
                        );
                    }
                });
            }
        }
    }

    fn replace_char(&mut self, c: char, char_count: u32, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.replace_char(c, char_count, ctx);
        });

        // Explicit call to ctx.notify() in the case that we don't make any updates to the model
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

        if let Some(word) = word_under_cursor {
            if !word.trim().is_empty() {
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
    }

    fn ex_command(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn visual_operator(
        &mut self,
        operator: &VimOperator,
        motion_type: MotionType,
        register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            // Compute the visual selection
            let include_newline = *operator != VimOperator::Change;
            model.vim_visual_selection_range(motion_type, include_newline, ctx);

            if matches!(
                operator,
                VimOperator::Delete | VimOperator::Change | VimOperator::Yank
            ) {
                let buffer = model.content().as_ref(ctx);
                let selection_model = model.buffer_selection_model().clone();
                let selected_text = buffer
                    .selected_text_as_plain_text(selection_model, ctx)
                    .into_string();
                if !selected_text.is_empty() {
                    VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
                        registers.write_to_register(register_name, selected_text, motion_type, ctx);
                    });
                }
            }

            match operator {
                VimOperator::Delete | VimOperator::Change => {
                    let selection_model = model.buffer_selection_model().clone();
                    model.update_content(
                        |mut content, ctx| {
                            content.apply_edit(
                                BufferEditAction::Backspace,
                                EditOrigin::UserInitiated,
                                selection_model,
                                ctx,
                            );
                        },
                        ctx,
                    );
                    if *operator == VimOperator::Change && motion_type == MotionType::Linewise {
                        model.vim_change_line_with_smart_indent(ctx);
                    }
                }
                VimOperator::ToggleCase | VimOperator::Lowercase | VimOperator::Uppercase => {
                    let transform = match operator {
                        VimOperator::ToggleCase => CaseTransform::Toggle,
                        VimOperator::Uppercase => CaseTransform::Uppercase,
                        VimOperator::Lowercase => CaseTransform::Lowercase,
                        _ => CaseTransform::Toggle,
                    };
                    model.transform_current_selections_case(transform, ctx);
                }
                VimOperator::Yank => {
                    model.vim_clear_selections(ctx);
                }
                VimOperator::ToggleComment => {
                    model.toggle_comments(ctx);

                    if motion_type == MotionType::Linewise {
                        model.vim_move_to_first_nonwhitespace(false, ctx);
                    } else {
                        model.vim_clear_selections(ctx);
                    }
                }
            }
        });

        // Force a re-render so that residual Visual mode highlight is cleared.
        ctx.notify();
    }

    fn visual_paste(
        &mut self,
        motion_type: MotionType,
        read_register_name: char,
        write_register_name: char,
        ctx: &mut ViewContext<Self>,
    ) {
        // Read content from the specified vim register
        let Some(RegisterContent {
            text,
            motion_type: yanked_motion_type,
        }) = VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
            registers.read_from_register(read_register_name, ctx)
        })
        else {
            return;
        };

        self.model.update(ctx, |model, ctx| {
            // Compute the visual selection
            let include_newline =
                motion_type == MotionType::Linewise && yanked_motion_type == MotionType::Linewise;
            model.vim_visual_selection_range(motion_type, include_newline, ctx);

            // Copy current selection to the write register before replacing it
            let buffer = model.content().as_ref(ctx);
            let selection_model = model.buffer_selection_model().clone();
            let selected_text = buffer
                .selected_text_as_plain_text(selection_model.clone(), ctx)
                .into_string();
            if !selected_text.is_empty() {
                VimRegisters::handle(ctx).update(ctx, |registers, ctx| {
                    registers.write_to_register(
                        write_register_name,
                        selected_text,
                        motion_type,
                        ctx,
                    );
                });
            }

            // Replace selection with yanked text
            model.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::Insert {
                            text: &text,
                            style: model.active_text_style(),
                            override_text_style: None,
                        },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );

            if motion_type == MotionType::Linewise {
                model.vim_move_to_line_bound(LineBound::Start, false, ctx);
            }
        });
    }

    fn visual_text_object(&mut self, text_object: &VimTextObject, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.vim_select_text_object(text_object, None, ctx);
        });
    }

    fn jump_to_first_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.jump_to_line_column(0, None, ctx);
        });
    }

    fn jump_to_last_line(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            let buffer = model.content().as_ref(ctx);
            let max_point = buffer.max_point();
            model.jump_to_line_column(max_point.row as usize, None, ctx);
        });
    }

    fn jump_to_line(&mut self, line_number: u32, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.jump_to_line_column(line_number as usize, None, ctx);
        });
    }

    fn jump_to_matching_bracket(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.vim_jump_to_matching_bracket(false, ctx);
        })
    }

    fn jump_to_unmatched_bracket(&mut self, bracket: &BracketChar, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.vim_jump_to_unmatched_bracket(bracket, false, ctx);
        })
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
        match new.mode {
            VimMode::Normal => {
                if *old == VimMode::Insert {
                    // When exiting insert mode, move cursor back to cover
                    // the character that was last inserted. In vim, the cursor should
                    // be ON the last inserted character, not after it.
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
                // Implement line capping for normal mode
                self.vim_maybe_enforce_cursor_line_cap(ctx);
            }
            VimMode::Insert => {
                // Apply insert position for different insert commands (i, a, o, etc.)
                self.vim_apply_insert_position(&new.position, ctx);
            }
            VimMode::Visual(_) => {
                self.model.update(ctx, |model, ctx| {
                    model.vim_set_visual_tail_to_selection_heads(ctx);
                });
            }
            _ => {}
        }
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
