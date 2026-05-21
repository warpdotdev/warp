use num_traits::SaturatingSub;
use std::ops::Range;
use vec1::Vec1;
use warpui::{AppContext, Entity, ModelAsRef, ModelContext, ModelHandle, units::Pixels};

use crate::{
    content::{
        buffer::{
            AutoScrollBehavior, Buffer, BufferEvent, BufferSelectAction, SelectionOffsets,
            ToBufferCharOffset, ToBufferPoint,
        },
        hidden_lines_model::HiddenLinesModel,
        selection_model::BufferSelectionModel,
        text::{BlockType, BufferBlockStyle, CodeBlockType},
    },
    render::model::{RenderState, SoftWrapPoint},
};
use string_offset::CharOffset;
use warpui::text::{TextBuffer, point::Point, word_boundaries::WordBoundariesPolicy};

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;

/// A generic selection and navigation model for text editors.
pub struct SelectionModel {
    render: ModelHandle<RenderState>,
    content: ModelHandle<Buffer>,
    selection_model: ModelHandle<BufferSelectionModel>,
    hidden_lines: Option<ModelHandle<HiddenLinesModel>>,

    /// The goal x-coordinate in pixels. When moving between lines, the desired column
    /// might not exist on the new line (because it's shorter than the previous line). Storing
    /// the goal column lets us move back to that column when changing to a longer line.
    ///
    /// This is stored in pixels instead of characters so that, like other editors, we can
    /// match the visual column, accounting for any differences in padding and character width.
    pub goal_xs: Option<Vec1<Pixels>>,

    /// The in-progress selection.
    pending_selection: Option<PendingSelection>,

    /// Whether navigation through hidden sections is allowed.
    /// When false, navigation operations that would pass through hidden sections
    /// will be prevented, keeping the selection at its current position.
    allow_hidden_navigation: bool,
}

/// A unit for text movement.
#[derive(Debug, Clone)]
pub enum TextUnit {
    /// Move by a single character.
    Character,
    /// Move by a single word, as defined by the given policy.
    Word(WordBoundariesPolicy),
    /// Move to line boundaries (the start or end of a soft-wrapped line).
    LineBoundary,
    /// Move by lines. Up is backwards and down is forwards.
    Line,
    /// Move to the paragraph boundaries (the start or end of a hard-wrapped line).
    ParagraphBoundary,
}

/// The mode for selection dragging.
#[derive(Debug, Clone)]
pub enum SelectionMode {
    /// Dragging extends the selection by a single character.
    Character,
    /// Dragging extends the selection by words.
    Word(WordBoundariesPolicy),
    /// Dragging extends the selection by entire lines.
    Line,
}

/// State for an in-progress selection. A selection is in-progress after the mouse is pressed and
/// before it's released, and may be extended by dragging.
#[derive(Debug)]
struct PendingSelection {
    mode: SelectionMode,
    head: CharOffset,
    tail: CharOffset,
}

/// A direction that a text-editing operation may apply over.
///
/// For example, the Backspace key deletes backwards, while the Delete key deletes forwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDirection {
    Forwards,
    Backwards,
}

#[derive(Debug, Clone, Copy)]
pub struct NavigationResult {
    /// The resulting character offset from text navigation.
    pub offset: CharOffset,
    /// The goal column based on the original offset, if it's different from the character offset.
    pub goal_x: Option<Pixels>,
}

impl SelectionModel {
    pub fn new(
        content: ModelHandle<Buffer>,
        render: ModelHandle<RenderState>,
        selection_model: ModelHandle<BufferSelectionModel>,
        hidden_lines: Option<ModelHandle<HiddenLinesModel>>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&content, Self::handle_buffer_event);

        Self {
            content,
            render,
            selection_model,
            goal_xs: None,
            pending_selection: None,
            hidden_lines,
            allow_hidden_navigation: true,
        }
    }

    pub fn with_disable_hidden_navigation(mut self) -> Self {
        self.allow_hidden_navigation = false;
        self
    }

    pub fn cursors(&self, ctx: &impl ModelAsRef) -> Vec1<CharOffset> {
        self.selection_model.as_ref(ctx).selection_heads()
    }

    /// Get whether navigation through hidden sections is allowed.
    pub fn allow_hidden_navigation(&self) -> bool {
        self.allow_hidden_navigation
    }

    fn rendered_mermaid_ranges(&self, ctx: &impl ModelAsRef) -> Vec<Range<CharOffset>> {
        self.render.as_ref(ctx).content().mermaid_block_ranges()
    }

    fn has_rendered_mermaid(&self, ctx: &impl ModelAsRef) -> bool {
        !self.rendered_mermaid_ranges(ctx).is_empty()
    }

    fn is_mermaid_code_block_offset(&self, offset: CharOffset, ctx: &impl ModelAsRef) -> bool {
        matches!(
            self.content.as_ref(ctx).block_type_at_point(offset),
            BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Mermaid,
            })
        )
    }

    fn normalize_line_navigation_offset(
        &self,
        _start: CharOffset,
        direction: TextDirection,
        offset: CharOffset,
        ctx: &impl ModelAsRef,
    ) -> CharOffset {
        if !self.has_rendered_mermaid(ctx) {
            return offset;
        }

        let content = self.content.as_ref(ctx);
        let max_offset = content.max_charoffset();

        match direction {
            TextDirection::Forwards
                if offset < max_offset && self.is_mermaid_code_block_offset(offset, ctx) =>
            {
                content.block_or_line_end(offset).min(max_offset)
            }
            TextDirection::Backwards
                if offset > CharOffset::from(1)
                    && self.is_mermaid_code_block_offset(offset, ctx) =>
            {
                content.block_or_line_start(offset)
            }
            _ => offset,
        }
    }

    fn normalize_selection_offsets(
        &self,
        selection: SelectionOffsets,
        direction: Option<TextDirection>,
        mermaid_ranges: &[Range<CharOffset>],
    ) -> SelectionOffsets {
        if mermaid_ranges.is_empty() {
            return selection;
        }

        if selection.head == selection.tail {
            let offset = Self::normalize_cursor_offset(selection.head, direction, mermaid_ranges);
            return SelectionOffsets {
                head: offset,
                tail: offset,
            };
        }

        let head_is_end = selection.head > selection.tail;
        let mut start = selection.head.min(selection.tail);
        let mut end = selection.head.max(selection.tail);
        let first_intersecting = mermaid_ranges.partition_point(|range| range.end <= start);
        for range in &mermaid_ranges[first_intersecting..] {
            if range.start >= end {
                break;
            }
            if start < range.end && range.start < end {
                start = start.min(range.start);
                end = end.max(range.end);
            }
        }

        if head_is_end {
            SelectionOffsets {
                head: end,
                tail: start,
            }
        } else {
            SelectionOffsets {
                head: start,
                tail: end,
            }
        }
    }

    fn normalize_cursor_offset(
        offset: CharOffset,
        direction: Option<TextDirection>,
        mermaid_ranges: &[Range<CharOffset>],
    ) -> CharOffset {
        let containing_range = mermaid_ranges.partition_point(|range| range.end <= offset);
        let Some(range) = mermaid_ranges
            .get(containing_range)
            .filter(|range| range.start < offset)
        else {
            return offset;
        };

        match direction {
            Some(TextDirection::Backwards) => range.start,
            Some(TextDirection::Forwards) => range.end,
            None => {
                let distance_to_start = offset - range.start;
                let distance_to_end = range.end - offset;
                if distance_to_start <= distance_to_end {
                    range.start
                } else {
                    range.end
                }
            }
        }
    }

    fn normalize_selections(
        &self,
        selections: Vec1<SelectionOffsets>,
        direction: Option<TextDirection>,
        ctx: &impl ModelAsRef,
    ) -> Vec1<SelectionOffsets> {
        let mermaid_ranges = self.rendered_mermaid_ranges(ctx);
        selections.mapped(|selection| {
            self.normalize_selection_offsets(selection, direction, &mermaid_ranges)
        })
    }

    fn normalize_character_selection_offsets(
        &self,
        selection: SelectionOffsets,
        action: &BufferSelectAction,
        direction: TextDirection,
        max_offset: CharOffset,
        mermaid_ranges: &[Range<CharOffset>],
    ) -> SelectionOffsets {
        match action {
            BufferSelectAction::MoveLeft => {
                let offset = if selection.head == selection.tail {
                    selection
                        .head
                        .saturating_sub(&CharOffset::from(1))
                        .max(1.into())
                } else {
                    selection.head.min(selection.tail)
                };
                let offset = Self::normalize_cursor_offset(offset, Some(direction), mermaid_ranges);
                SelectionOffsets {
                    head: offset,
                    tail: offset,
                }
            }
            BufferSelectAction::MoveRight => {
                let offset = if selection.head == selection.tail {
                    (selection.head + CharOffset::from(1)).min(max_offset)
                } else {
                    selection.head.max(selection.tail)
                };
                let offset = Self::normalize_cursor_offset(offset, Some(direction), mermaid_ranges);
                SelectionOffsets {
                    head: offset,
                    tail: offset,
                }
            }
            BufferSelectAction::ExtendLeft => {
                let head = selection
                    .head
                    .saturating_sub(&CharOffset::from(1))
                    .max(1.into());
                let head = Self::normalize_cursor_offset(head, Some(direction), mermaid_ranges);
                SelectionOffsets {
                    head,
                    tail: selection.tail,
                }
            }
            BufferSelectAction::ExtendRight => {
                let head = (selection.head + CharOffset::from(1)).min(max_offset);
                let head = Self::normalize_cursor_offset(head, Some(direction), mermaid_ranges);
                SelectionOffsets {
                    head,
                    tail: selection.tail,
                }
            }
            _ => unreachable!(),
        }
    }
    fn normalized_character_action(
        &self,
        action: &BufferSelectAction,
        ctx: &impl ModelAsRef,
    ) -> Option<BufferSelectAction> {
        let direction = match action {
            BufferSelectAction::MoveLeft | BufferSelectAction::ExtendLeft => {
                TextDirection::Backwards
            }
            BufferSelectAction::MoveRight | BufferSelectAction::ExtendRight => {
                TextDirection::Forwards
            }
            _ => return None,
        };

        let max_offset = self.content.as_ref(ctx).max_charoffset();
        let selections = self.selections(ctx);
        let mermaid_ranges = self.rendered_mermaid_ranges(ctx);
        let raw = selections.clone().mapped(|selection| match action {
            BufferSelectAction::MoveLeft => {
                let offset = if selection.head == selection.tail {
                    selection
                        .head
                        .saturating_sub(&CharOffset::from(1))
                        .max(1.into())
                } else {
                    selection.head.min(selection.tail)
                };
                SelectionOffsets {
                    head: offset,
                    tail: offset,
                }
            }
            BufferSelectAction::MoveRight => {
                let offset = if selection.head == selection.tail {
                    (selection.head + CharOffset::from(1)).min(max_offset)
                } else {
                    selection.head.max(selection.tail)
                };
                SelectionOffsets {
                    head: offset,
                    tail: offset,
                }
            }
            BufferSelectAction::ExtendLeft => SelectionOffsets {
                head: selection
                    .head
                    .saturating_sub(&CharOffset::from(1))
                    .max(1.into()),
                tail: selection.tail,
            },
            BufferSelectAction::ExtendRight => SelectionOffsets {
                head: (selection.head + CharOffset::from(1)).min(max_offset),
                tail: selection.tail,
            },
            _ => unreachable!(),
        });
        let normalized = selections.mapped(|selection| {
            self.normalize_character_selection_offsets(
                selection,
                action,
                direction,
                max_offset,
                &mermaid_ranges,
            )
        });

        (normalized != raw).then_some(BufferSelectAction::UpdateSelectionOffsets {
            selections: normalized,
        })
    }

    fn normalize_select_action(
        &self,
        action: BufferSelectAction,
        ctx: &impl ModelAsRef,
    ) -> BufferSelectAction {
        if let Some(action) = self.normalized_character_action(&action, ctx) {
            return action;
        }
        let current_selections = self.selections(ctx);
        if let Some(selections) = action.selection_offsets(Some(&current_selections)) {
            action.with_selection_offsets(self.normalize_selections(selections, None, ctx))
        } else {
            action
        }
    }

    fn validate_select_action(&self, action: &BufferSelectAction, ctx: &AppContext) -> bool {
        if self.allow_hidden_navigation() {
            return true;
        }

        let selection_model = self.selection_model.as_ref(ctx);
        let hidden_lines = self.hidden_lines.as_ref().map(|hl| hl.as_ref(ctx));
        match action {
            BufferSelectAction::MoveLeft => {
                !(selection_model.all_single_cursors()
                    && hidden_lines
                        .map(|hl| hl.after_hidden_section(ctx))
                        .unwrap_or(false))
            }
            BufferSelectAction::MoveRight => {
                !(selection_model.all_single_cursors()
                    && hidden_lines
                        .map(|hl| hl.before_hidden_section(ctx))
                        .unwrap_or(false))
            }
            BufferSelectAction::ExtendLeft => !hidden_lines
                .map(|hl| hl.after_hidden_section(ctx))
                .unwrap_or(false),
            BufferSelectAction::ExtendRight => !hidden_lines
                .map(|hl| hl.before_hidden_section(ctx))
                .unwrap_or(false),
            _ => action
                .selection_offsets(Some(&selection_model.selection_offsets()))
                .is_none_or(|selections| {
                    selections.iter().all(|selection| {
                        let range = if selection.head < selection.tail {
                            selection.head..selection.tail
                        } else {
                            selection.tail..selection.head
                        };
                        !hidden_lines
                            .map(|hl| hl.contains_hidden_section(&range, ctx))
                            .unwrap_or(false)
                    })
                }),
        }
    }

    /// Updates the buffer-level selection. Editors **must** use this instead of calling
    /// [`Buffer::update_selection`] directly, unless they are updating the selection in
    /// order to edit.
    /// Returns true if the selection action proposed adheres to invariants around hidden sections.
    pub fn update_selection(
        &mut self,
        action: BufferSelectAction,
        autoscroll: AutoScrollBehavior,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let action = self.normalize_select_action(action, ctx);
        if !self.validate_select_action(&action, ctx) {
            return false;
        }
        // Horizontal movement resets the goal column. If this is called by a vertical movement
        // action, it will save the new goal afterwards.
        self.goal_xs = None;
        let selection_model = self.selection_model.clone();
        self.content.update(ctx, |content, ctx| {
            content.update_selection(selection_model, action, autoscroll, ctx)
        });
        true
    }

    /// End any ongoing selection.
    pub fn end_selection(&mut self, _ctx: &mut ModelContext<Self>) {
        self.pending_selection = None;
    }

    /// Set a single cursor at the offset.
    pub fn set_cursor(&mut self, offset: CharOffset, ctx: &mut ModelContext<Self>) {
        self.update_selection(
            BufferSelectAction::AddCursorAt {
                offset,
                clear_selections: true,
            },
            AutoScrollBehavior::Selection,
            ctx,
        );
    }

    // Add a new cursor at the offset.
    pub fn add_cursor(&mut self, offset: CharOffset, ctx: &mut ModelContext<Self>) {
        self.update_selection(
            BufferSelectAction::AddCursorAt {
                offset,
                clear_selections: false,
            },
            AutoScrollBehavior::Selection,
            ctx,
        );
    }

    /// Update the head and tail offsets for the currently active selections.
    fn update_selections(
        &mut self,
        selections: Vec1<SelectionOffsets>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.update_selection(
            BufferSelectAction::UpdateSelectionOffsets { selections },
            AutoScrollBehavior::Selection,
            ctx,
        );
    }

    pub fn set_last_head(&mut self, offset: CharOffset, ctx: &mut ModelContext<Self>) {
        self.update_selection(
            BufferSelectAction::SetLastHead { offset },
            AutoScrollBehavior::Selection,
            ctx,
        );
    }

    /// Begin a new selection, starting from the given offset.
    pub fn begin_selection(
        &mut self,
        offset: CharOffset,
        mode: SelectionMode,
        clear_selections: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // TODO(INT-266): Support multiselect with semantic selection.
        let offsets = match &mode {
            SelectionMode::Character => SelectionOffsets {
                head: offset,
                tail: offset,
            },
            SelectionMode::Word(policy) => {
                let content = self.content.as_ref(ctx);
                let start = content.word_start(offset, policy);
                let end = content.word_end(offset, policy);
                SelectionOffsets {
                    head: end,
                    tail: start,
                }
            }
            SelectionMode::Line => {
                let content = self.content.as_ref(ctx);
                let point = offset.to_buffer_point(content);
                let line_start = Point::new(point.row, 0);
                let line_end = Point::new(point.row, content.line_len(point.row));
                let start = line_start.to_buffer_char_offset(content);
                let end = line_end.to_buffer_char_offset(content);
                SelectionOffsets {
                    head: end,
                    tail: start,
                }
            }
        };
        let offsets = self
            .normalize_selections(Vec1::new(offsets), None, ctx)
            .into_iter()
            .next()
            .expect("single selection should exist");

        self.pending_selection = Some(PendingSelection {
            mode,
            head: offsets.head,
            tail: offsets.tail,
        });
        self.update_selection(
            BufferSelectAction::AddSelection {
                head: offsets.head,
                tail: offsets.tail,
                clear_selections,
            },
            AutoScrollBehavior::Selection,
            ctx,
        );
    }

    /// Update the pending selection. Pending selections are started when the mouse is pressed, and
    /// active as it is dragged. If the original selection were semantic (selecting by word or
    /// line), that selection mode is kept until the mouse is released.
    pub fn update_pending_selection(&mut self, offset: CharOffset, ctx: &mut ModelContext<Self>) {
        if let Some(pending_selection) = &self.pending_selection {
            let mut head = pending_selection.head;
            let mut tail = pending_selection.tail;

            match &pending_selection.mode {
                SelectionMode::Character => {
                    head = offset;
                }
                SelectionMode::Word(policy) => {
                    let content = self.content.as_ref(ctx);
                    if offset > head {
                        // If the cursor is after the pending selection, extend to the next word's
                        // end.
                        head = content.word_end(offset, policy);
                    } else if offset < tail {
                        // If the cursor is before the pending selection, extend to the previous
                        // word's start.
                        tail = head;
                        head = content.word_start(offset, policy);
                    }
                }
                SelectionMode::Line => {
                    let content = self.content.as_ref(ctx);
                    let point = offset.to_buffer_point(content);

                    if offset > head {
                        // If the cursor is after the pending selection, extend to the next lines.
                        let end_column = content.line_len(point.row);
                        head = Point::new(point.row, end_column).to_buffer_char_offset(content);
                    } else if offset < tail {
                        // If the cursor is before the pending selection, extend to the previous
                        // lines.
                        tail = head;
                        head = Point::new(point.row, 0).to_buffer_char_offset(content);
                    }
                }
            }

            self.update_selection(
                BufferSelectAction::SetLastSelection { head, tail },
                AutoScrollBehavior::Selection,
                ctx,
            );
        }
    }

    /// The selected offset ranges.
    ///  This will always return the range from the lowest offset to highest.
    ///  The head or tail may be the lowest offset.
    pub fn selections(&self, ctx: &impl ModelAsRef) -> Vec1<SelectionOffsets> {
        self.selection_model.as_ref(ctx).selection_offsets()
    }

    /// The starting point of the selection.
    ///
    /// This may be the head or tail, depending on the selection's direction.
    pub fn selection_start(&self, ctx: &impl ModelAsRef) -> CharOffset {
        let selection_model = self.selection_model.as_ref(ctx);
        selection_model
            .first_selection_head()
            .min(selection_model.first_selection_tail())
    }

    /// The end point of the selection.
    ///
    /// This may be the head or tail, depending on the selection's direction.
    pub fn selection_end(&self, ctx: &impl ModelAsRef) -> CharOffset {
        let selection_model = self.selection_model.as_ref(ctx);
        selection_model
            .first_selection_head()
            .max(selection_model.first_selection_tail())
    }

    /// Modify all of the selections in a given way.
    /// The currently active selections are looped though, along with any current x-pixel goal values,
    /// and a new head and tail offsets and a new x-pixel goal value are calculated for each selection.
    ///
    /// selection_update: A function that takes the current selection, the current goal x, and the current
    /// selection offsets, and returns the new goal x and the new selection offsets.
    fn update_selections_internal<T>(&mut self, selection_update: T, ctx: &mut ModelContext<Self>)
    where
        T: Fn(
            &SelectionModel,
            &mut ModelContext<SelectionModel>,
            &SelectionOffsets,
            &Option<Pixels>,
        ) -> (Option<Pixels>, SelectionOffsets),
    {
        // Before we take action, merge any overlapping selections.
        self.selection_model.update(ctx, |selection_model, _ctx| {
            selection_model.merge_overlapping_selections();
        });

        let selections = self.selections(ctx);
        let goal_xs = match self.goal_xs.take() {
            Some(goals) => goals.mapped(Some),
            None => selections.mapped_ref(|_| None),
        };

        // Compute the new head location and new goal x, given the old head position and the old goal x.
        let (new_goal_xs, new_selections): (Vec<_>, Vec<_>) = selections
            .iter()
            .zip(goal_xs.iter())
            .map(|(selection, old_goal_x)| selection_update(self, ctx, selection, old_goal_x))
            .unzip();

        // Convert from Vec<Option> to <Option<Vec1>>.
        // Assumptions:
        // For the same requested navigation direction and unit, the function will always return the a
        // new goal x OR it will never return a new goal x.  Therefore, the Vec is either full of Somes
        // or full of Nones.
        // This assumption is correct as of the current implementation, but is not checked by the compiler.
        //
        // Assumption: The goal x vector is always at least length 1, because the initial cursor list is a Vec1.
        debug_assert!(
            new_goal_xs.iter().all(|c| c.is_some()) || !new_goal_xs.iter().any(|c| c.is_some()),
            "All goal_xs should be Some or all should be None."
        );

        // Set the new selections.
        self.update_selections(
            Vec1::try_from_vec(new_selections)
                .expect("Should not be empty because original was a vec1"),
            ctx,
        );

        // Note: This must be set after set_selections, because that clears goal_xs.
        self.goal_xs = new_goal_xs
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .map(|goal| {
                Vec1::try_from_vec(goal).expect("Should not be empty because original was a vec1")
            });
    }

    /// Extend the current selection by moving its head.
    pub fn extend_selection(
        &mut self,
        direction: TextDirection,
        unit: TextUnit,
        ctx: &mut ModelContext<Self>,
    ) {
        // Update the selections by moving the head.
        self.update_selections_internal(
            |model, ctx, selection, goal_x| {
                let result = model.navigate(
                    selection.head,
                    direction,
                    unit.clone(),
                    1, // extend_selection always moves by 1
                    *goal_x,
                    ctx,
                );
                // Only move the head, leave the tail where it is.
                let new_selection = SelectionOffsets {
                    head: result.offset,
                    tail: selection.tail,
                };
                let new_goal_x = result.goal_x;
                let new_selection = model
                    .normalize_selections(Vec1::new(new_selection), Some(direction), ctx)
                    .into_iter()
                    .next()
                    .expect("single selection should exist");
                (new_goal_x, new_selection)
            },
            ctx,
        );
    }

    /// Move in the given direction, switching to a single cursor.
    pub fn move_selection(
        &mut self,
        direction: TextDirection,
        unit: TextUnit,
        ctx: &mut ModelContext<Self>,
    ) {
        self.move_selection_by(direction, unit, 1, ctx);
    }

    /// Move in the given direction by the given step size, switching to a single cursor.
    pub fn move_selection_by(
        &mut self,
        direction: TextDirection,
        unit: TextUnit,
        step_size: u32,
        ctx: &mut ModelContext<Self>,
    ) {
        // Update the selections by moving the head and tail, navigating starting on the head.
        self.update_selections_internal(
            |model, ctx, selection, goal_x| {
                let result = model.navigate(
                    selection.head,
                    direction,
                    unit.clone(),
                    step_size,
                    *goal_x,
                    ctx,
                );
                // Move both head and tail to the same position (single cursor)
                let new_selection = SelectionOffsets {
                    head: result.offset,
                    tail: result.offset,
                };
                let new_goal_x = result.goal_x;
                let new_selection = model
                    .normalize_selections(Vec1::new(new_selection), Some(direction), ctx)
                    .into_iter()
                    .next()
                    .expect("single selection should exist");
                (new_goal_x, new_selection)
            },
            ctx,
        );
    }

    /// From a starting character offset, calculate the ending character offset to move by `unit`
    /// in `direction`.
    pub fn navigate(
        &self,
        start: CharOffset,
        direction: TextDirection,
        unit: TextUnit,
        step_size: u32,
        goal_x: Option<Pixels>,
        ctx: &impl ModelAsRef,
    ) -> NavigationResult {
        match unit {
            TextUnit::Character => self.navigate_character(start, direction, ctx),
            TextUnit::Word(policy) => self.navigate_word(start, policy, direction, ctx),
            TextUnit::Line => self.navigate_line(start, direction, step_size, goal_x, ctx),
            TextUnit::LineBoundary => self.navigate_line_boundary(start, direction, ctx),
            TextUnit::ParagraphBoundary => self.navigate_paragraph_boundary(start, direction, ctx),
        }
    }

    /*
    Navigation implementations for specific units, split out for readability.
    */

    pub fn navigate_character(
        &self,
        start: CharOffset,
        direction: TextDirection,
        ctx: &impl ModelAsRef,
    ) -> NavigationResult {
        NavigationResult::for_offset(match direction {
            // TODO: navigate by grapheme clusters, not `char`s
            TextDirection::Backwards => start.saturating_sub(&1.into()).max(1.into()),
            TextDirection::Forwards => (start + 1).min(self.content.as_ref(ctx).max_charoffset()),
        })
    }

    pub fn navigate_word(
        &self,
        start: CharOffset,
        policy: WordBoundariesPolicy,
        direction: TextDirection,
        ctx: &impl ModelAsRef,
    ) -> NavigationResult {
        let content = self.content.as_ref(ctx);

        // We need to special-case the scenario when the starting offset is at the end of
        // a block-item. The block item itself should be considered as a "word" unit.
        if matches!(direction, TextDirection::Backwards)
            && matches!(content.block_type_at_point(start), BlockType::Item(_))
        {
            return NavigationResult::for_offset(start.saturating_sub(&1.into()));
        }
        let end_offset = match direction {
            TextDirection::Backwards => content
                .word_starts_backward_from_offset_exclusive(start)
                .ok()
                .and_then(|word_ends| word_ends.with_policy(policy).next())
                .and_then(|point| content.to_offset(point).ok())
                .unwrap_or(start),
            TextDirection::Forwards => content
                .word_ends_from_offset_exclusive(start)
                .ok()
                .and_then(|word_ends| word_ends.with_policy(policy).next())
                .and_then(|point| content.to_offset(point).ok())
                .unwrap_or(start),
        };
        NavigationResult::for_offset(end_offset)
    }

    pub fn navigate_line(
        &self,
        start: CharOffset,
        direction: TextDirection,
        step_size: u32,
        goal_x: Option<Pixels>,
        ctx: &impl ModelAsRef,
    ) -> NavigationResult {
        let render = self.render.as_ref(ctx);
        // TODO(CLD-558): This shouldn't need the +/- 1
        let point = render.offset_to_softwrap_point(start.saturating_sub(&1.into()));

        let next_point = match direction {
            TextDirection::Backwards => {
                let mut current_point = point;
                for _ in 0..step_size {
                    current_point = current_point.previous_row().unwrap_or_else(|| {
                        SoftWrapPoint::new(current_point.row(), current_point.column())
                    });
                }
                current_point
            }
            TextDirection::Forwards => {
                let mut current_point = point;
                for _ in 0..step_size {
                    match current_point.next_row(render.max_line()) {
                        Some(next) => current_point = next,
                        None => {
                            // If moving down on the last line, snap to the end of the buffer.
                            return NavigationResult::for_offset_and_goal(
                                self.content.as_ref(ctx).max_charoffset(),
                                Some(current_point.column()),
                            );
                        }
                    }
                }
                current_point
            }
        };

        // Take the max of the goal column and the next point so that, if we're moving from a short
        // column to a long one, we keep the (possible) further-right goal column. This is generally
        // equivalent to self.goal_x.unwrap_or(next_point.column()), but captures the intent that we
        // want to stick to the rightmost point along the path, especially with proportional fonts.
        let goal_column = match goal_x {
            Some(x) => x.max(next_point.column()),
            None => next_point.column(),
        };
        let goal_point = SoftWrapPoint::new(next_point.row(), goal_column);
        let next_offset = render.softwrap_point_to_offset(goal_point) + 1;
        let next_offset = self.normalize_line_navigation_offset(start, direction, next_offset, ctx);
        NavigationResult::for_offset_and_goal(next_offset, Some(goal_column))
    }

    pub fn navigate_line_boundary(
        &self,
        start: CharOffset,
        direction: TextDirection,
        ctx: &impl ModelAsRef,
    ) -> NavigationResult {
        let render = self.render.as_ref(ctx);

        // We need to special-case the scenario when the starting offset is at the end of
        // a block-item. From the buffer perspective, the offset is at the start of the line
        // given block items are represented as linebreaks. But from the user perspective, the
        // starting offset is visually on the same line as the block item. To handle this,
        // we simply return the starting offset - 1.
        let content = self.content.as_ref(ctx);
        let block_type = content.block_type_at_point(start);
        if matches!(direction, TextDirection::Backwards) && matches!(block_type, BlockType::Item(_))
        {
            return NavigationResult::for_offset(start.saturating_sub(&1.into()));
        }

        // TODO(CLD-558): This shouldn't need the +/- 1
        let start_point = render.offset_to_softwrap_point(start.saturating_sub(&1.into()));
        let end_offset = match direction {
            TextDirection::Backwards => {
                let row_start = SoftWrapPoint::new(start_point.row(), Pixels::zero());
                let soft_wrapped_start = render.softwrap_point_to_offset(row_start);

                match content.indented_line_start(start) {
                    // Like code editors, we can move between the line boundaries and the first
                    // non-whitespace character of the line.
                    Some(indented_start)
                        if indented_start > soft_wrapped_start && indented_start < start =>
                    {
                        indented_start
                    }
                    _ => soft_wrapped_start + 1,
                }
            }
            TextDirection::Forwards => {
                // To find the end of the row, find the start of the next row and subtract 1. The render
                // model's soft-wrapping state doesn't track line length.
                // If we're at the last row, the next-row-minus-1 approach won't work. Instead, we can
                // clamp to the end of the buffer.
                if start_point.row() >= render.max_line().as_u32().saturating_sub(1) {
                    self.content.as_ref(ctx).max_charoffset()
                } else {
                    match content.indented_line_start(start) {
                        Some(indented_start) if indented_start > start => indented_start,
                        _ => {
                            let next_row_start =
                                SoftWrapPoint::new(start_point.row() + 1, Pixels::zero());
                            // TODO(CLD-558): This should have a -1.
                            render.softwrap_point_to_offset(next_row_start)
                        }
                    }
                }
            }
        };
        NavigationResult::for_offset(end_offset)
    }

    fn navigate_paragraph_boundary(
        &self,
        start: CharOffset,
        direction: TextDirection,
        ctx: &impl ModelAsRef,
    ) -> NavigationResult {
        let content = self.content.as_ref(ctx);
        match direction {
            TextDirection::Backwards => {
                if matches!(content.block_type_at_point(start), BlockType::Item(_)) {
                    // See `navigate_line_boundary` on why block items are special-cased.
                    NavigationResult::for_offset(start.saturating_sub(&1.into()))
                } else {
                    NavigationResult::for_offset(content.containing_line_start(start))
                }
            }
            TextDirection::Forwards => NavigationResult::for_offset(
                // containing_line_end returns the offset of the first character after the ending
                // newline/marker, so subtract 1 to get the last character within the line.
                content.containing_line_end(start).saturating_sub(&1.into()),
            ),
        }
    }

    fn handle_buffer_event(&mut self, event: &BufferEvent, ctx: &mut ModelContext<Self>) {
        match event {
            BufferEvent::ContentChanged { origin, .. } if origin.from_user() => self.goal_xs = None,
            BufferEvent::AnchorUpdated {
                update,
                excluding_model,
            } if *excluding_model != Some(self.selection_model.id()) => {
                self.selection_model.update(ctx, |selection_model, _| {
                    selection_model.update_anchors(update.clone());
                })
            }
            _ => {}
        }
    }
}

impl Entity for SelectionModel {
    type Event = ();
}

impl NavigationResult {
    /// Creates a `NavigationResult` with both a new offset and an updated goal column.
    /// If the goal column does not exist on the new line, it may not correspond to the
    /// actual offset.
    pub fn for_offset_and_goal(offset: CharOffset, goal_x: Option<Pixels>) -> Self {
        Self { offset, goal_x }
    }

    /// Creates a `NavigationResult` with just an offset. This will clear out the goal column.
    pub fn for_offset(offset: CharOffset) -> Self {
        Self {
            offset,
            goal_x: None,
        }
    }
}
