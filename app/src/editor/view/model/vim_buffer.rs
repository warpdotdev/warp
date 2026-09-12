use string_offset::CharOffset;
use vec1::Vec1;
use vim::handler::{VimBufferOps, VimCaret, VimSnapshot};
use vim::vim::{MotionType, VimOperator};
use warpui::ModelContext;

use super::{AnchorBias, EditorModel, LocalSelection, Selection, ToCharOffset};

fn to_one_based(offset: CharOffset) -> CharOffset {
    CharOffset::from(offset.as_usize() + 1)
}

fn to_zero_based(offset: CharOffset) -> CharOffset {
    CharOffset::from(offset.as_usize().saturating_sub(1))
}

impl VimBufferOps for EditorModel {
    type Ctx<'a> = ModelContext<'a, Self>;

    fn snapshot(&self, ctx: &Self::Ctx<'_>) -> VimSnapshot {
        let buffer = self.buffer(ctx);
        let visual_tails = self.vim_visual_tails();
        let carets = self
            .selections(ctx)
            .iter()
            .enumerate()
            .map(|(index, selection)| {
                let head =
                    to_one_based(selection.head().to_char_offset(buffer).unwrap_or_default());
                let tail = visual_tails
                    .get(index)
                    .and_then(|anchor| anchor.to_char_offset(buffer).ok())
                    .map(to_one_based)
                    .unwrap_or_else(|| {
                        to_one_based(selection.tail().to_char_offset(buffer).unwrap_or_default())
                    });
                VimCaret {
                    head,
                    tail,
                    goal_column: selection.goal_end_column,
                }
            })
            .collect();
        VimSnapshot::from_plain_text(&self.buffer_text(ctx), carets)
    }

    fn set_selections(&mut self, carets: &[VimCaret], ctx: &mut Self::Ctx<'_>) {
        let buffer = self.buffer(ctx);
        let max = buffer.len();
        let new_selections: Vec<LocalSelection> = carets
            .iter()
            .filter_map(|caret| {
                let head = to_zero_based(caret.head).min(max);
                let tail = to_zero_based(caret.tail).min(max);
                let (start, end, reversed) = if head >= tail {
                    (tail, head, false)
                } else {
                    (head, tail, true)
                };
                let start_anchor = buffer.anchor_at(start, AnchorBias::Left).ok()?;
                let end_anchor = if start == end {
                    start_anchor.clone()
                } else {
                    buffer.anchor_at(end, AnchorBias::Left).ok()?
                };
                Some(LocalSelection {
                    selection: Selection {
                        start: start_anchor,
                        end: end_anchor,
                        reversed,
                    },
                    clamp_direction: Default::default(),
                    goal_start_column: caret.goal_column,
                    goal_end_column: caret.goal_column,
                })
            })
            .collect();
        let Ok(new_selections) = Vec1::try_from_vec(new_selections) else {
            return;
        };
        if !self.vim_visual_tails.is_empty() {
            let buffer = self.buffer(ctx);
            self.vim_visual_tails = carets
                .iter()
                .filter_map(|caret| {
                    buffer
                        .anchor_at(to_zero_based(caret.tail).min(max), AnchorBias::Left)
                        .ok()
                })
                .collect();
            let collapsed = new_selections.mapped(|mut selection| {
                let head = selection.head().clone();
                selection.set_selection(Selection::single_cursor(head));
                selection
            });
            self.change_selections(collapsed, ctx);
            return;
        }
        self.change_selections(new_selections, ctx);
    }

    fn replace_ranges(
        &mut self,
        edits: &[(CharOffset, CharOffset, String)],
        ctx: &mut Self::Ctx<'_>,
    ) {
        let mut converted: Vec<(CharOffset, CharOffset, String)> = edits
            .iter()
            .map(|(start, end, text)| (to_zero_based(*start), to_zero_based(*end), text.clone()))
            .collect();
        converted.sort_by_key(|(start, _, _)| start.as_usize());
        converted.reverse();
        for (start, end, text) in converted {
            let end = end.max(start);
            if let Err(error) = self.buffer_edit(std::iter::once(start..end), text, ctx) {
                warp_errors::report_error!(error.context("error applying vim replace_ranges"));
            }
        }
    }

    fn selected_text(&mut self, ctx: &mut Self::Ctx<'_>) -> String {
        EditorModel::selected_text(self, ctx)
    }

    fn delete_selection(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.insert("", None, ctx);
    }

    fn insert_text(&mut self, text: &str, ctx: &mut Self::Ctx<'_>) {
        self.insert(text, None, ctx);
    }

    fn indent(&mut self, dedent: bool, ctx: &mut Self::Ctx<'_>) {
        if dedent {
            EditorModel::unindent(self, ctx);
        } else {
            EditorModel::indent(self, ctx);
        }
    }

    fn set_visual_tails_to_heads(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_set_visual_tail_to_selection_heads(ctx);
    }

    fn expand_visual_selection(
        &mut self,
        motion_type: MotionType,
        include_newline: bool,
        ctx: &mut Self::Ctx<'_>,
    ) {
        self.vim_visual_selection_range(motion_type, include_newline, ctx);
    }

    fn enforce_normal_mode_line_cap(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.vim_visual_tails.clear();
        self.enforce_cursor_line_cap(ctx);
    }

    fn supports_operator(&self, operator: &VimOperator) -> bool {
        matches!(
            operator,
            VimOperator::Delete
                | VimOperator::Change
                | VimOperator::Yank
                | VimOperator::ToggleCase
                | VimOperator::Uppercase
                | VimOperator::Lowercase
        )
    }

    fn supports_text_objects(&self) -> bool {
        true
    }

    fn last_line_lands_on_first_nonwhitespace(&self) -> bool {
        false
    }

    fn yank_restores_original_cursor(&self, motion_type: MotionType) -> bool {
        motion_type == MotionType::Linewise
    }
}
