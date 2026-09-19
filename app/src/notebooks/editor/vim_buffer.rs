use string_offset::CharOffset;
use vec1::Vec1;
use vim::handler::VimBufferOps;
use vim::vim::VimOperator;
use warp_editor::content::buffer::{
    AutoScrollBehavior, BufferEditAction, BufferSelectAction, EditOrigin, SelectionOffsets,
};
use warp_editor::model::{CoreEditorModel, RichTextEditorModel};
use warp_editor::selection::{TextDirection, TextUnit};
use warpui::ModelContext;

use super::NotebooksEditorModel;

impl NotebooksEditorModel {
    fn vim_set_selections(
        &mut self,
        selections: Vec1<SelectionOffsets>,
        autoscroll: AutoScrollBehavior,
        ctx: &mut ModelContext<Self>,
    ) {
        self.selection.update(ctx, |selection, ctx| {
            selection.update_selection(
                BufferSelectAction::SetSelectionOffsets { selections },
                autoscroll,
                ctx,
            );
        });
    }
}

impl VimBufferOps for NotebooksEditorModel {
    type Ctx<'a> = ModelContext<'a, Self>;

    fn snapshot(&self, ctx: &Self::Ctx<'_>) -> vim::handler::VimSnapshot {
        let text = self.content().as_ref(ctx).text().into_string();
        let carets = self
            .selections(ctx)
            .iter()
            .enumerate()
            .map(|(index, selection)| vim::handler::VimCaret {
                head: selection.head,
                tail: selection.tail,
                goal_column: self
                    .vim_goal_columns
                    .as_ref()
                    .and_then(|goals| goals.get(index).copied()),
            })
            .collect();
        vim::handler::VimSnapshot::from_plain_text(&text, carets)
    }

    fn set_selections(&mut self, carets: &[vim::handler::VimCaret], ctx: &mut Self::Ctx<'_>) {
        let Ok(selections) = Vec1::try_from_vec(
            carets
                .iter()
                .map(|caret| SelectionOffsets {
                    head: caret.head,
                    tail: caret.tail,
                })
                .collect(),
        ) else {
            return;
        };
        self.applying_vim_selections = true;
        self.vim_set_selections(selections, AutoScrollBehavior::Selection, ctx);
        self.vim_goal_columns = if carets.iter().any(|caret| caret.goal_column.is_some()) {
            Some(
                carets
                    .iter()
                    .map(|caret| caret.goal_column.unwrap_or(0))
                    .collect(),
            )
        } else {
            None
        };
        self.vim_applied_heads = Some(carets.iter().map(|caret| caret.head).collect());
        self.applying_vim_selections = false;
    }

    fn replace_ranges(
        &mut self,
        edits: &[(CharOffset, CharOffset, String)],
        ctx: &mut Self::Ctx<'_>,
    ) {
        let Ok(edits) = Vec1::try_from_vec(
            edits
                .iter()
                .map(|(start, end, text)| (text.clone(), *start..*end))
                .collect(),
        ) else {
            return;
        };
        let selection_model = self.buffer_selection_model().clone();
        self.update_content(
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

    fn indent(&mut self, dedent: bool, ctx: &mut Self::Ctx<'_>) {
        let selection_model = self.buffer_selection_model().clone();
        self.update_content(
            |mut content, ctx| {
                content.apply_edit(
                    BufferEditAction::Indent {
                        num_unit: 1,
                        shift: dedent,
                    },
                    EditOrigin::UserInitiated,
                    selection_model,
                    ctx,
                );
            },
            ctx,
        );
    }

    fn selected_text(&mut self, ctx: &mut Self::Ctx<'_>) -> String {
        self.content()
            .as_ref(ctx)
            .selected_text_as_plain_text(self.buffer_selection_model().clone(), ctx)
            .into_string()
    }

    fn delete_selection(&mut self, ctx: &mut Self::Ctx<'_>) {
        self.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
    }

    fn insert_text(&mut self, text: &str, ctx: &mut Self::Ctx<'_>) {
        self.insert(text, EditOrigin::UserInitiated, ctx);
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
                | VimOperator::Indent
                | VimOperator::Dedent
        )
    }
}
