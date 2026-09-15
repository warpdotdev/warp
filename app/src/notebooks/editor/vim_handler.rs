use vim::handler::{
    self, CaseTransform, VimBufferOps, YankedText, apply_mode_change, apply_operator,
    apply_visual_operator, apply_visual_paste,
};
use vim::vim::{
    CharacterMotion, Direction, InsertPosition, LineMotion, ModeTransition, MotionType, VimHandler,
    VimMode, VimOperand, VimOperator, VimTextObject,
};
use warp_editor::content::buffer::EditOrigin;
use warp_editor::model::{CoreEditorModel, RichTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui::{SingletonEntity, TypedActionView, ViewContext};

use super::RichTextEditorView;
use crate::notebooks::editor::find_bar::FindBarAction;
use crate::vim_registers::{RegisterContent, VimRegisters};

impl VimHandler for RichTextEditorView {
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
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.user_insert(&c.to_string(), ctx);
            });
        }
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
            if !advance && char_count > model.snapshot(ctx).remaining_on_line() {
                return;
            }
            for _ in 0..char_count.max(1) {
                model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                model.insert(&c.to_string(), EditOrigin::UserInitiated, ctx);
            }
            if !advance {
                handler::move_char(model, 1, &CharacterMotion::Left, false, ctx);
            }
        });
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
        if repeat_count > 0 {
            let repeated = text.repeat(repeat_count as usize);
            self.model.update(ctx, |model, ctx| {
                for c in repeated.chars() {
                    model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                    model.insert(&c.to_string(), EditOrigin::UserInitiated, ctx);
                }
            });
        }
        ctx.notify();
    }

    fn toggle_case(&mut self, char_count: u32, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            handler::move_char(model, char_count.max(1), &CharacterMotion::Right, true, ctx);
            model.transform_case(CaseTransform::Toggle, ctx);
        });
    }

    fn search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        self.last_search_direction = *direction;
        self.find_bar.show(ctx);
    }

    fn cycle_search(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        let effective = match (direction, self.last_search_direction) {
            (Direction::Forward, dir) => dir,
            (Direction::Backward, Direction::Backward) => Direction::Forward,
            (Direction::Backward, Direction::Forward) => Direction::Backward,
        };
        match effective {
            Direction::Forward => self.find_bar.view().clone().update(ctx, |bar, ctx| {
                bar.handle_action(&FindBarAction::FocusNextMatch, ctx)
            }),
            Direction::Backward => self.find_bar.view().clone().update(ctx, |bar, ctx| {
                bar.handle_action(&FindBarAction::FocusPreviousMatch, ctx)
            }),
        }
    }

    fn search_word_at_cursor(&mut self, direction: &Direction, ctx: &mut ViewContext<Self>) {
        self.last_search_direction = *direction;
        self.find_bar.show(ctx);
    }

    fn ex_command(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn keyword_prg(&mut self, _ctx: &mut ViewContext<Self>) {}

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

    fn visual_text_object(&mut self, _text_object: &VimTextObject, _ctx: &mut ViewContext<Self>) {}

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
        let insert = text.repeat(count.max(1) as usize);
        self.model.update(ctx, |model, ctx| {
            if motion_type == MotionType::Linewise {
                match direction {
                    Direction::Forward => {
                        handler::move_line(model, 1, &LineMotion::End, false, ctx);
                        model.insert(&format!("\n{insert}"), EditOrigin::UserInitiated, ctx);
                    }
                    Direction::Backward => {
                        handler::move_line(model, 1, &LineMotion::Start, false, ctx);
                        model.insert(&format!("{insert}\n"), EditOrigin::UserInitiated, ctx);
                    }
                }
            } else if *direction == Direction::Forward {
                handler::move_char(model, 1, &CharacterMotion::Right, false, ctx);
                model.insert(&insert, EditOrigin::UserInitiated, ctx);
            } else {
                model.insert(&insert, EditOrigin::UserInitiated, ctx);
            }
        });
    }

    fn insert_text(
        &mut self,
        text: &str,
        position: &InsertPosition,
        count: u32,
        ctx: &mut ViewContext<Self>,
    ) {
        let repeated = text.repeat(count.max(1) as usize);
        self.model.update(ctx, |model, ctx| {
            apply_mode_change(
                model,
                &VimMode::Normal,
                &ModeTransition {
                    mode: VimMode::Insert,
                    position: *position,
                },
                ctx,
            );
            model.insert(&repeated, EditOrigin::UserInitiated, ctx);
        });
    }

    fn join_line(&mut self, _count: u32, _ctx: &mut ViewContext<Self>) {}

    fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        if self.can_edit(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.undo(ctx);
            });
        }
    }

    fn change_mode(&mut self, old: &VimMode, new: &ModeTransition, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            apply_mode_change(model, old, new, ctx);
        });
        ctx.notify();
    }

    fn backspace(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_editable(ctx) {
            self.model.update(ctx, |model, ctx| {
                model.backspace(ctx);
            });
        }
    }

    fn delete_forward(&mut self, ctx: &mut ViewContext<Self>) {
        self.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
    }

    fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        if self.find_bar.is_focused(ctx) {
            self.find_bar.hide(ctx);
            return;
        }
        if matches!(self.vim_mode(ctx), Some(VimMode::Normal)) {
            ctx.notify();
            return;
        }
        self.vim_escape(ctx);
        ctx.notify();
    }
}

impl RichTextEditorView {
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
}
