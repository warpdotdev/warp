use std::{cmp::max, fmt::Debug, mem, ops::RangeInclusive};

use sum_tree::SeekBias;
use vec1::{vec1, Vec1};
use warp_core::semantic_selection::SemanticSelection;
use warp_terminal::model::grid::CellType;
use warpui::{
    text::{IsRect, SelectionType},
    units::{IntoLines as _, Lines},
    AppContext, EntityId, ViewAsRef as _,
};

use crate::{
    ai::blocklist::{block::PendingUserQueryBlock, AIBlock},
    env_vars::env_var_collection_block::EnvVarCollectionBlock,
    terminal::{
        event::Event as TerminalEvent,
        model::{
            block::BlockSection,
            index::{Direction, Point, Side},
            selection::{ExpandedSelectionRange, Selection, SelectionDirection},
            terminal_model::{BlockIndex, WithinBlock},
        },
        warpify::success_block::WarpifySuccessBlock,
        GridType,
    },
};

use super::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, BlockListPoint, RichContentItem,
};

/// A selection that can span multiple blocks (and thus grids). Here row is the number of lines from
/// the top of all blocks.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd)]
pub struct BlockAnchor {
    point: BlockListPoint,
    side: Side,
}

impl BlockAnchor {
    pub fn new(point: BlockListPoint, side: Side) -> Self {
        BlockAnchor { point, side }
    }
}

#[derive(Debug, Clone)]
pub struct BlockListSelection {
    head: BlockAnchor,
    tail: BlockAnchor,
    selection_type: SelectionType,
    /// If this is Some, and if smart-select is enabled, double-clicking within this range will
    /// select this range instead of the normal smart-select logic. The purpose of this is to
    /// allow double-click selection to work on the TerminalView::highlighted_link even when it
    /// contains spaces. Smart-select never traverses across whitespace.
    smart_select_override: Option<RangeInclusive<WithinBlock<Point>>>,
}

impl BlockListSelection {
    pub fn new(point: BlockListPoint, selection_type: SelectionType, side: Side) -> Self {
        BlockListSelection {
            head: BlockAnchor::new(point, side),
            tail: BlockAnchor::new(point, side),
            selection_type,
            smart_select_override: None,
        }
    }

    /// Bring start and end points in the correct order.
    pub fn points_need_swap(start: BlockListPoint, end: BlockListPoint) -> bool {
        start.row > end.row || start.row == end.row && start.column > end.column
    }

    pub fn is_empty(&self) -> bool {
        match self.selection_type {
            SelectionType::Simple | SelectionType::Rect => {
                let (start, end) = if Self::points_need_swap(self.head.point, self.tail.point) {
                    (self.tail, self.head)
                } else {
                    (self.head, self.tail)
                };

                // Simple selection is empty when the points are identical
                // or two adjacent cells have the sides right -> left.
                start == end
                    || (start.side == Side::Right
                        && end.side == Side::Left
                        && (start.point.row == end.point.row)
                        && start.point.column + 1 == end.point.column)
            }
            SelectionType::Semantic | SelectionType::Lines => false,
        }
    }

    /// The start anchor of the selection, regardless of whether the selection is reversed.
    pub fn start_anchor(&mut self) -> &mut BlockAnchor {
        let (head, tail) = (&mut self.head, &mut self.tail);
        if Self::points_need_swap(head.point, tail.point) {
            tail
        } else {
            head
        }
    }

    /// The end anchor of the selection, regardless of whether the selection is reversed.
    pub fn end_anchor(&mut self) -> &mut BlockAnchor {
        let (head, tail) = (&mut self.head, &mut self.tail);
        if Self::points_need_swap(head.point, tail.point) {
            head
        } else {
            tail
        }
    }

    /// Given a block list position (offset from the top-left corner of the
    /// block list), returns the specific cell (row/column index within a
    /// particular grid within a particular block) closest to the given point.
    /// May return None if the position is not within a block.
    pub fn clamp_block_list_point_to_grid(
        block_list_point: BlockListPoint,
        block_list: &BlockList,
    ) -> Option<WithinBlock<Point>> {
        let mut block_heights_cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        block_heights_cursor.seek(&BlockHeight::from(block_list_point.row), SeekBias::Right);

        let block_index = match block_heights_cursor.item() {
            Some(BlockHeightItem::Block(_)) => block_heights_cursor.start().block_count.into(),
            _ => return None,
        };

        // Determine whether it is in the command grid, output grid or none of them.
        let block = block_list.block_at(block_index)?;
        let (point, grid_type) =
            match block.find(block_list_point.row - block_heights_cursor.start().height) {
                BlockSection::BlockBanner | BlockSection::PaddingTop if block.honor_ps1() => {
                    (Point::new(0, 0), GridType::Prompt)
                }
                BlockSection::BlockBanner
                | BlockSection::PaddingTop
                | BlockSection::CommandPaddingTop => (Point::new(0, 0), GridType::PromptAndCommand),
                BlockSection::PromptAndCommandGrid(row) => (
                    Point::new(row, block_list_point.column),
                    GridType::PromptAndCommand,
                ),
                BlockSection::PaddingMiddle => (
                    Point::new(
                        block.prompt_and_command_number_of_rows().saturating_sub(1),
                        block_list.size().columns().saturating_sub(1),
                    ),
                    GridType::PromptAndCommand,
                ),
                BlockSection::PaddingBottom if block.output_grid().is_empty() => (
                    Point::new(
                        block.prompt_and_command_number_of_rows().saturating_sub(1),
                        block_list.size().columns().saturating_sub(1),
                    ),
                    GridType::PromptAndCommand,
                ),
                BlockSection::OutputGrid(row) => {
                    (Point::new(row, block_list_point.column), GridType::Output)
                }
                BlockSection::PaddingBottom
                | BlockSection::EndOfBlock
                | BlockSection::NotContained => (
                    Point::new(
                        block.output_grid().len_displayed().saturating_sub(1),
                        block_list_point.column,
                    ),
                    GridType::Output,
                ),
            };

        Some(WithinBlock::new(point, block_index, grid_type))
    }

    fn max_point_within_block(
        &self,
        block_list: &BlockList,
        block_index: BlockIndex,
    ) -> Option<WithinBlock<Point>> {
        let block = block_list.block_at(block_index);

        if let Some(block_at_index) = block {
            return Some(WithinBlock::new(
                Point::new(
                    block_at_index
                        .output_grid()
                        .len_displayed()
                        .saturating_sub(1),
                    block_list.size().columns().saturating_sub(1),
                ),
                block_index,
                GridType::Output,
            ));
        }
        None
    }

    /// Clamp the selection start and end two points within grids.
    pub fn clamp_to_grid_points(
        &mut self,
        block_list: &BlockList,
    ) -> Option<(WithinBlock<Point>, WithinBlock<Point>)> {
        // Enforce that the head is above the tail to make the logic below consistent.
        let head_is_below_tail = (self.head.point.row > self.tail.point.row
            && !block_list.is_inverted)
            || (self.head.point.row < self.tail.point.row && block_list.is_inverted);
        let (mut head, mut tail) = if head_is_below_tail {
            (self.tail, self.head)
        } else {
            (self.head, self.tail)
        };

        let mut head_block_heights_cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        let mut tail_block_heights_cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        head_block_heights_cursor.seek(&BlockHeight::from(head.point.row), SeekBias::Right);
        tail_block_heights_cursor.seek(&BlockHeight::from(tail.point.row), SeekBias::Right);
        let original_head_cursor_total_index = head_block_heights_cursor.start().total_count;
        let original_tail_cursor_total_index = tail_block_heights_cursor.start().total_count;

        // Move the head cursor forward until we reach a command block or the item at the tail.
        // We have to check based on total count since rich and command blocks can share the same index.
        while !matches!(
            head_block_heights_cursor.item(),
            Some(BlockHeightItem::Block(height)) if height.as_f64() > 0.
        ) && original_tail_cursor_total_index != head_block_heights_cursor.start().total_count
        {
            if !block_list.is_inverted {
                head_block_heights_cursor.next();
            } else {
                head_block_heights_cursor.prev();
            }
        }

        // Move the tail cursor backwards until we reach a command block or the item at the head.
        while !matches!(
            tail_block_heights_cursor.item(),
            Some(BlockHeightItem::Block(height)) if height.as_f64() > 0.
        ) && original_head_cursor_total_index != tail_block_heights_cursor.start().total_count
        {
            if !block_list.is_inverted {
                tail_block_heights_cursor.prev();
            } else {
                tail_block_heights_cursor.next();
            }
        }

        // If the head and tail aren't both on command blocks, there weren't any command blocks in the selection.
        if !matches!(
            (
                head_block_heights_cursor.item(),
                tail_block_heights_cursor.item()
            ),
            (
                Some(BlockHeightItem::Block(_)),
                Some(BlockHeightItem::Block(_))
            )
        ) {
            return None;
        }

        let adjusted_head_block_index = head_block_heights_cursor.start().block_count.into();
        let adjusted_tail_block_index = tail_block_heights_cursor.start().block_count.into();
        let adjusted_head_cursor_total_index = head_block_heights_cursor.start().total_count;
        let adjusted_tail_cursor_total_index = tail_block_heights_cursor.start().total_count;

        // Both head and tail cursors are on a command block at this point. Clamp the head and tail points as necessary.
        let mut block_start_point = Self::clamp_block_list_point_to_grid(head.point, block_list);
        let mut block_end_point = Self::clamp_block_list_point_to_grid(tail.point, block_list);
        if adjusted_head_cursor_total_index != original_head_cursor_total_index {
            let grid_type = GridType::PromptAndCommand;
            block_start_point = Some(WithinBlock::new(
                Point::new(0, 0),
                adjusted_head_block_index,
                grid_type,
            ));
            if !matches!(self.selection_type, SelectionType::Rect) {
                // If we are snapping to the max point, make sure the head is at the left side.
                head.side = Side::Left;
            }
        }
        if adjusted_tail_cursor_total_index != original_tail_cursor_total_index {
            block_end_point = self.max_point_within_block(block_list, adjusted_tail_block_index);
            if !matches!(self.selection_type, SelectionType::Rect) {
                // If we are snapping to the max point, make sure the tail is at the right side.
                tail.side = Side::Right;
            }
        }

        // Undo the 'swap' we did at the beginning of the function.
        if let (Some(mut head_point), Some(mut tail_point)) = (block_start_point, block_end_point) {
            // If the active selection mode is rect selection, do not clamp the head and tail columns.
            if matches!(self.selection_type, SelectionType::Rect) {
                head_point.inner.col = head.point.column;
                tail_point.inner.col = tail.point.column;
            }

            if head_is_below_tail {
                self.head = tail;
                self.tail = head;
                return Some((tail_point, head_point));
            } else {
                self.head = head;
                self.tail = tail;
                return Some((head_point, tail_point));
            }
        }

        None
    }
}

#[derive(Debug)]
struct ExpandedSelection {
    absolute_point: BlockListPoint,
    within_grid_point: WithinBlock<Point>,
}

impl ExpandedSelection {
    pub fn new(absolute_point: BlockListPoint, within_grid_point: WithinBlock<Point>) -> Self {
        ExpandedSelection {
            absolute_point,
            within_grid_point,
        }
    }
}

/// A renderable selection that includes ranges expanded for various selection types.
/// Start is always before end.
#[derive(Debug)]
pub struct SelectionRange {
    pub start: BlockListPoint,
    pub end: BlockListPoint,
}

impl SelectionRange {
    pub fn new(start: BlockListPoint, end: BlockListPoint) -> Self {
        SelectionRange { start, end }
    }
}

impl BlockList {
    /// Exposes the underlying selection, which is stored as two BlockListPoints.
    /// This selection either has a renderable range OR it is empty.
    pub fn selection(&self) -> Option<&BlockListSelection> {
        self.selection.as_ref()
    }

    /// Returns the start and end points of the current text selection in the block list, if any,
    /// and whether the selection was reversed.
    /// Note that the `start` is always before `end`.
    pub fn text_selection_range(
        &self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<(WithinBlock<Point>, WithinBlock<Point>, bool)> {
        let selection_range = self.expand_selection(semantic_selection, inverted_blocklist)?;
        Some((
            selection_range.start().within_grid_point,
            selection_range.end().within_grid_point,
            selection_range.is_reversed(),
        ))
    }

    /// Returns the range of cells that the selection spans, which represent what is rendered
    /// as a highlighted selection
    pub fn renderable_selection(
        &self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<Vec1<SelectionRange>> {
        self.expand_selection(semantic_selection, inverted_blocklist)
            .as_ref()
            .map(|selection| match selection {
                ExpandedSelectionRange::Regular { start, end, .. } => {
                    vec1![SelectionRange::new(
                        self.clip_selection_start(start),
                        self.clip_selection_end(end),
                    )]
                }
                ExpandedSelectionRange::Rect { rows } => rows.mapped_ref(|(start, end)| {
                    SelectionRange::new(
                        self.clip_selection_start(start),
                        self.clip_selection_end(end),
                    )
                }),
            })
    }

    /// Initializes a selection of the specified type/side AT the given point (head/tail are both defined to be the given point).
    /// However, if smart_select_override is specified, the selection may be overridden, if the override wraps around the cursor.
    /// Generally, one would use update_selection to expand the selection after initializing it using start_selection.
    pub fn start_selection(
        &mut self,
        point: BlockListPoint,
        selection_type: SelectionType,
        side: Side,
    ) {
        let mut selection = BlockListSelection::new(point, selection_type, side);
        if let Some(smart_select_override) = &self.smart_select_override {
            let (override_start, override_end) = smart_select_override.unfold_range();
            // convert the WithinBlock values to BlockListPoint so they can be compared to the
            // point where the click occurred
            let override_start_absolute =
                BlockListPoint::from_within_block_point(&override_start, self);
            let override_end_absolute =
                BlockListPoint::from_within_block_point(&override_end, self);
            // We only want to accept this override if it actually wraps around the cursor, so we
            // do this comparison to make sure the point of the cursor is between the ends of the
            // range of the potential override text. Since BlockListPoint rows are floats, the
            // comparison isn't as straightforward as you would intuit.
            // BlockListPoint::from_within_block_point will return a point where the row value is
            // the top of the row. The cursor will be somewhere in between the top and bottom of
            // the cell, so we need to add 1.0 to override_end_absolute.row for the comparison to
            // be correct
            if override_start_absolute.row <= point.row
                && point.row <= override_end_absolute.row + 1.into_lines()
                // the column only needs to be checked if they are in the same row
                && (point.row <= override_end_absolute.row || point.column <= override_end_absolute.column)
                && (override_start_absolute.row + 1.into_lines() <= point.row || override_start_absolute.column <= point.column)
            {
                selection.smart_select_override = Some(override_start..=override_end)
            }
        }
        self.set_selection(selection);
    }

    /// Used to update an existing selection's tail to a new BlockListPoint (and an associated `side`).
    /// Must have an existing selection to update - otherwise, this is a no-op (use start_selection first).
    pub fn update_selection(&mut self, point: BlockListPoint, side: Side) {
        let Some(mut selection) = self.selection.take() else {
            return;
        };

        let block_anchor = BlockAnchor::new(point, side);

        selection.tail = block_anchor;

        self.set_selection(selection);
    }

    /// Coordinates an update to the tail of the block-text selection. Returns the BlockListPoint
    /// of the new tail, if an update was made.
    pub fn move_selection_tail(
        &mut self,
        direction: &SelectionDirection,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<BlockListPoint> {
        self.standardize_text_selection(semantic_selection, inverted_blocklist);

        match direction {
            SelectionDirection::Left => {
                self.select_text_left(semantic_selection, inverted_blocklist)
            }
            SelectionDirection::Right => {
                self.select_text_right(semantic_selection, inverted_blocklist)
            }
            SelectionDirection::Up => self.select_text_up(semantic_selection, inverted_blocklist),
            SelectionDirection::Down => {
                self.select_text_down(semantic_selection, inverted_blocklist)
            }
        }
    }

    /// Extends or reduces the tail of the selection by one cell leftwards. This function expands
    /// the underlying selection into a WithinBlock<Point>, operates on the tail's coordinates,
    /// and then converts the tail back into the coordinate space of the blocklist.
    /// Returns the BlockListPoint of the new tail, if an update was made.
    pub fn select_text_left(
        &mut self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<BlockListPoint> {
        let tail = match self.expand_selection(semantic_selection, inverted_blocklist) {
            Some(selection) => selection.tail().within_grid_point,
            None => match self.selection.take() {
                None => return None,
                Some(mut selection) => {
                    // In this case, there's an underlying selection that spans an empty range
                    // and has no rendering.
                    //
                    // The selection is in one of two formats:
                    // (row: x, col: y, side: R) --> (row: x, col: y + 1, side: L)
                    // (row: x, col: y + 1, side: L) --> (row: x, col: y, side: R)
                    // And we should update the selection to this:
                    // (row: x, col: y, side: R) --> (row: x, col: y, side: L)
                    let start_anchor = selection.start_anchor().point;

                    selection.tail.point = start_anchor;
                    selection.tail.side = Side::Left;
                    selection.head.point = start_anchor;
                    selection.head.side = Side::Right;
                    self.set_selection(selection);
                    return Some(start_anchor);
                }
            },
        };

        let mut selection = self.selection.take()?;

        let tail_point = tail.get();
        let grid = match self.block_at(tail.block_index) {
            None => return None,
            Some(block) => match tail.grid {
                GridType::Prompt => block.prompt_grid(),
                GridType::PromptAndCommand => block.prompt_and_command_grid(),
                GridType::Rprompt => block.rprompt_grid(),
                GridType::Output => block.output_grid(),
            },
        };

        // There are two cases:
        // (1) If the tail is at the top-left corner of a grid, search upwards to find
        //     the nearest Command or Output grid. If no grids are found, the selection remains
        //     the same.
        // (2) If there's still space in the grid above or to the left, we use a grid iterator
        //     to move leftwards one cell.
        let new_tail = match (tail_point.row, tail_point.col) {
            (0, 0) => {
                match self.seek_up_to_next_grid(
                    tail.block_index,
                    tail.grid,
                    self.size.columns - 1,
                    inverted_blocklist,
                ) {
                    // If None, the tail is already at the top-most grid at row=0, col=0.
                    None => tail,
                    Some(block_point) => block_point,
                }
            }
            _ => {
                let mut selection_cursor = grid.grid_handler.selection_cursor_from(*tail_point);
                selection_cursor.move_backward();
                let point = selection_cursor.position()?;
                WithinBlock::new(point, tail.block_index, tail.grid)
            }
        };

        let mut new_tail_blocklist = BlockListPoint::from_within_block_point(&new_tail, self);
        new_tail_blocklist.row += 0.5.into_lines(); // Endpoints are positioned in the row's vertical center.
        selection.tail.point = new_tail_blocklist;
        self.set_selection(selection);
        Some(new_tail_blocklist)
    }

    /// Extends or reduces the tail of the selection by one cell rightwards. This function expands
    /// the underlying selection into a WithinBlock<Point>, operates on the tail's coordinates,
    /// and then converts the tail back into the coordinate space of the blocklist.
    /// Returns the BlockListPoint of the new tail, if an update was made.
    pub fn select_text_right(
        &mut self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<BlockListPoint> {
        let tail = match self.expand_selection(semantic_selection, inverted_blocklist) {
            Some(selection) => selection.tail().within_grid_point,
            None => match self.selection.take() {
                None => return None,
                Some(mut selection) => {
                    // In this case, there's an underlying selection that spans an empty range
                    // and has no rendering.
                    //
                    // The selection is in one of two formats:
                    // (row: x, col: y, side: R) --> (row: x, col: y + 1, side: L)
                    // (row: x, col: y + 1, side: L) --> (row: x, col: y, side: R)
                    // And we should update the selection to this:
                    // (row: x, col: y + 1, side: L) --> (row: x, col: y + 1, side: R)
                    let end_anchor = selection.end_anchor().point;

                    selection.head.point = end_anchor;
                    selection.head.side = Side::Left;
                    selection.tail.point = end_anchor;
                    selection.tail.side = Side::Right;
                    self.set_selection(selection);
                    return Some(end_anchor);
                }
            },
        };

        let mut selection = self.selection.take()?;

        let tail_point = tail.get();
        let grid = match self.block_at(tail.block_index) {
            None => return None,
            Some(block) => match tail.grid {
                GridType::Prompt => block.prompt_grid(),
                GridType::PromptAndCommand => block.prompt_and_command_grid(),
                GridType::Rprompt => block.rprompt_grid(),
                GridType::Output => block.output_grid(),
            },
        };
        let num_rows_in_grid = grid.len_displayed();

        // There are two cases:
        // (1) If the tail is at the bottom-right corner of a grid, search downwards to find
        //     the nearest Command or Output grid. If no grids are found, the selection remains the
        //     same.
        // (2) If there's still space in the grid below or to the right, we use a grid iterator
        //     to move rightwards one cell.
        let new_tail = match (tail_point.row, tail_point.col) {
            (row, col) if row == num_rows_in_grid - 1 && col == self.size.columns - 1 => {
                match self.seek_down_to_next_grid(
                    tail.block_index,
                    tail.grid,
                    0,
                    inverted_blocklist,
                ) {
                    // If None, the tail is already at the bottom-most grid at the maximum row and col.
                    None => tail,
                    Some(block_point) => block_point,
                }
            }
            (_, _) => {
                let mut selection_cursor = grid.grid_handler.selection_cursor_from(*tail_point);
                selection_cursor.move_forward();
                let point = selection_cursor.position()?;
                WithinBlock::new(point, tail.block_index, tail.grid)
            }
        };

        let mut new_tail_blocklist = BlockListPoint::from_within_block_point(&new_tail, self);
        new_tail_blocklist.row += 0.5.into_lines(); // Endpoints are positioned in the row's vertical center.
        selection.tail.point = new_tail_blocklist;
        self.set_selection(selection);
        Some(new_tail_blocklist)
    }

    /// Extends or reduces the tail of the selection by one cell upwards. This function expands
    /// the underlying selection into a WithinBlock<Point>, operates on the tail's coordinates,
    /// and then converts the tail back into the coordinate space of the blocklist.
    /// Returns the BlockListPoint of the new tail, if an update was made.
    pub fn select_text_up(
        &mut self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<BlockListPoint> {
        let (mut head, mut tail) =
            match self.expand_selection(semantic_selection, inverted_blocklist) {
                Some(selection) => (
                    selection.head().within_grid_point,
                    selection.tail().within_grid_point,
                ),
                // select-up and select-down do nothing when there's no rendered selection.
                None => return None,
            };

        let mut selection = self.selection.take()?;

        // If the selection spans exactly one line, ensure the tail is at the beginning
        // This is custom logic to make sure that shift-down on a selected line grabs
        // an additional line instead of erasing the selection.
        if selection.head.point.row == selection.tail.point.row
            && selection.head.point.column == 0
            && selection.tail.point.column == self.size.columns - 1
        {
            mem::swap(&mut selection.head, &mut selection.tail);
            mem::swap(&mut head, &mut tail);
        }

        let tail_point = tail.get();
        let grid = match self.block_at(tail.block_index) {
            None => return None,
            Some(block) => match tail.grid {
                GridType::Prompt => block.prompt_grid(),
                GridType::PromptAndCommand => block.prompt_and_command_grid(),
                GridType::Rprompt => block.rprompt_grid(),
                GridType::Output => block.output_grid(),
            },
        };

        // There are two cases:
        // (1) If the tail is at the topmost row of a grid, search upwards at the current
        //     column to find the next Command or Output grid. If no grid is found, move
        //     the tail all the way to the left of the current row.
        // (2) If the tail is in a grid that has more rows above, jump one row upwards.
        let new_tail = match (tail_point.row, tail_point.col) {
            (0, col) => match self.seek_up_to_next_grid(
                tail.block_index,
                tail.grid,
                col,
                inverted_blocklist,
            ) {
                None => {
                    // If there's no grid above, moves the selection tail to span the top row.
                    selection.tail.side = Side::Left;
                    let top_left_corner = Point::new(0, 0);
                    WithinBlock::new(top_left_corner, tail.block_index, tail.grid)
                }
                Some(block_point) => block_point,
            },
            _ => {
                let mut selection_cursor = grid.grid_handler.selection_cursor_from(*tail_point);
                selection_cursor.move_up();
                let point = selection_cursor.position()?;
                WithinBlock::new(point, tail.block_index, tail.grid)
            }
        };

        let mut new_tail_blocklist = BlockListPoint::from_within_block_point(&new_tail, self);
        new_tail_blocklist.row += 0.5.into_lines(); // Endpoints are positioned in the row's vertical center.
        selection.tail.point = new_tail_blocklist;

        self.set_selection(selection);

        Some(new_tail_blocklist)
    }

    /// Extends or reduces the tail of the selection by one cell downwards. This function expands
    /// the underlying selection into a WithinBlock<Point>, operates on the tail's coordinates,
    /// and then converts the tail back into the coordinate space of the blocklist.
    /// Returns the BlockListPoint of the new tail, if an update was made.
    pub fn select_text_down(
        &mut self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<BlockListPoint> {
        let (mut head, mut tail) =
            match self.expand_selection(semantic_selection, inverted_blocklist) {
                Some(selection) => (
                    selection.head().within_grid_point,
                    selection.tail().within_grid_point,
                ),
                // select-up and select-down do nothing when there's no rendered selection.
                None => return None,
            };

        let mut selection = self.selection.take()?;

        // If the selection spans exactly one line, ensure the tail is at the end.
        // This is custom logic to make sure that shift-down on a selected line grabs
        // an additional line instead of erasing the selection.
        if selection.head.point.row == selection.tail.point.row
            && selection.head.point.column == self.size.columns - 1
            && selection.tail.point.column == 0
        {
            mem::swap(&mut selection.head, &mut selection.tail);
            mem::swap(&mut head, &mut tail);
        }

        let (new_tail, is_at_bottom) = self.move_point_down(tail, inverted_blocklist)?;
        if is_at_bottom {
            // If there's no grid below, pin the selection to the end of the row since there is no more lines below.
            selection.tail.side = Side::Right;
        }

        let mut new_tail_blocklist = BlockListPoint::from_within_block_point(&new_tail, self);
        new_tail_blocklist.row += 0.5.into_lines(); // Endpoints are positioned in the row's vertical center.
        selection.tail.point = new_tail_blocklist;

        self.set_selection(selection);

        Some(new_tail_blocklist)
    }

    /// Given a point in the blocklist, move it down one column. If the input point does not exist in
    /// the grid, return None. Otherwise, return a (point, bool) pair where the point is the result point
    /// after the move and bool indicates whether the input point is already at the bottom of grid.
    fn move_point_down(
        &self,
        tail: WithinBlock<Point>,
        inverted_blocklist: bool,
    ) -> Option<(WithinBlock<Point>, bool)> {
        let tail_point = tail.get();
        let grid = match self.block_at(tail.block_index) {
            None => return None,
            Some(block) => match tail.grid {
                GridType::Prompt => block.prompt_grid(),
                GridType::PromptAndCommand => block.prompt_and_command_grid(),
                GridType::Rprompt => block.rprompt_grid(),
                GridType::Output => block.output_grid(),
            },
        };
        let num_rows_in_grid = grid.len_displayed();
        let mut is_at_bottom = false;

        // There are two cases:
        // (1) If the tail is at the bottom row of a grid, search downwards at the current
        //     column to find the next Command or Output grid. If no grid is found, move
        //     the tail all the way to the right of the current row.
        // (2) If the tail is in a grid that has more rows below, jump one row downwards.
        Some((
            match (tail_point.row, tail_point.col) {
                (row, col) if row == num_rows_in_grid - 1 => {
                    match self.seek_down_to_next_grid(
                        tail.block_index,
                        tail.grid,
                        col,
                        inverted_blocklist,
                    ) {
                        None => {
                            let bottom_right_corner = Point::new(row, self.size.columns - 1);
                            is_at_bottom = true;
                            WithinBlock::new(bottom_right_corner, tail.block_index, tail.grid)
                        }
                        Some(block_point) => block_point,
                    }
                }
                _ => {
                    let mut selection_cursor = grid.grid_handler.selection_cursor_from(*tail_point);
                    selection_cursor.move_down();
                    let point = selection_cursor.position()?;
                    WithinBlock::new(point, tail.block_index, tail.grid)
                }
            },
            is_at_bottom,
        ))
    }

    /// Converts the head and tail of a block text selection to meet the following standards:
    /// (1) The left-end of a selection opens on the Left side of a cell and the right-end of a selection
    ///     terminates on the Right side of a cell. This allows us to mostly ignore the Sides of the selection
    ///     endpoints when the tail is modified. The exception is the case where select-up or select-down
    ///     causes a change in the direction of the selection.
    /// (2) After mouse-drag has completed, all selections become Simple selections (as opposed to Semantic
    ///     of Line). This means changing the selection range via keyboard has standardized behavior regardless
    ///     of how the user first created the selection.
    /// (3) Standardized endpoints fall in the exact vertical center of their rows, for easy comparison with eachother.
    ///     In addition, the vertical center makes us less liable to floating-point precision errors that can occur at
    ///     row boundaries.
    /// This function should be called only after an active selection is finished.
    pub fn standardize_text_selection(
        &mut self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) {
        let (head, tail, selection_moves_right) =
            match self.expand_selection(semantic_selection, inverted_blocklist) {
                Some(selection) => (
                    selection.head().within_grid_point,
                    selection.tail().within_grid_point,
                    !selection.is_reversed(),
                ),
                None => return,
            };
        let mut new_head_blocklist = BlockListPoint::from_within_block_point(&head, self);
        let mut new_tail_blocklist = BlockListPoint::from_within_block_point(&tail, self);

        // Endpoints are positioned in the row's vertical center. (See function comment).
        new_head_blocklist.row += 0.5.into_lines();
        new_tail_blocklist.row += 0.5.into_lines();

        if let Some(selection) = self.selection.as_mut() {
            selection.head.point = new_head_blocklist;
            selection.head.side = if selection_moves_right {
                Side::Left
            } else {
                Side::Right
            };

            selection.tail.point = new_tail_blocklist;
            selection.tail.side = if selection_moves_right {
                Side::Right
            } else {
                Side::Left
            };

            selection.selection_type = SelectionType::Simple;
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.event_proxy
            .send_terminal_event(TerminalEvent::TextSelectionChanged);
    }

    pub fn set_smart_select_override(
        &mut self,
        smart_select_override: WithinBlock<RangeInclusive<Point>>,
    ) {
        self.smart_select_override = Some(smart_select_override);
    }

    pub fn clear_smart_select_override(&mut self) {
        self.smart_select_override = None;
    }

    pub fn selection_to_string(
        &self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
        app: &AppContext,
    ) -> Option<String> {
        match self.expand_selection(semantic_selection, inverted_blocklist) {
            Some(ExpandedSelectionRange::Regular { start, end, .. }) => {
                let start_within_grid_point = start.within_grid_point;
                let end_within_grid_point = end.within_grid_point;

                let mut selected_texts: Vec<String> = vec![];
                let mut selection_start_cursor = self
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                let original_selection = self
                    .selection
                    .as_ref()
                    .expect("Selection should exist if it can be expanded");
                let mut top_row = original_selection.head.point.row;
                let mut bottom_row = original_selection.tail.point.row;

                // Ensure that top_row is always above bottom_row so we can loop based on block heights.
                if original_selection.tail.point.row < original_selection.head.point.row {
                    top_row = original_selection.tail.point.row;
                    bottom_row = original_selection.head.point.row;
                }
                selection_start_cursor.seek(&BlockHeight::from(top_row), SeekBias::Right);

                // Loop over each block, adding their contents to the output.
                let agent_view_state = self.agent_view_state();
                while bottom_row >= selection_start_cursor.start().height {
                    let Some(item) = selection_start_cursor.item() else {
                        // We reached the end of the block list.
                        break;
                    };
                    // Otherwise, accumulate selection depending on block type.
                    match item {
                        BlockHeightItem::Block { .. } => {
                            let block_index = selection_start_cursor.start().block_count.into();
                            if let Some(command_block) = self.block_at(block_index) {
                                // Don't copy hidden or empty blocks.
                                if command_block.is_empty(agent_view_state) {
                                    selection_start_cursor.next();
                                    continue;
                                }

                                let start_point =
                                    if block_index == start.within_grid_point.block_index {
                                        start_within_grid_point.into()
                                    } else {
                                        command_block.start_point()
                                    };
                                let end_point = if block_index == end.within_grid_point.block_index
                                {
                                    end_within_grid_point.into()
                                } else {
                                    command_block.end_point()
                                };

                                selected_texts
                                    .push(command_block.bounds_to_string(start_point, end_point));
                            }
                        }
                        BlockHeightItem::RichContent(RichContentItem { view_id, .. }) => {
                            if let Some(selected_text) =
                                read_selected_text_from_ai_block(*view_id, app)
                            {
                                selected_texts.push(selected_text);
                            }
                            if let Some(selected_text) =
                                read_selected_text_from_pending_user_query_block(*view_id, app)
                            {
                                selected_texts.push(selected_text);
                            }

                            if let Some(active_window_id) = app.windows().active_window() {
                                if let Some(ssh_block) = app
                                    .view_with_id::<WarpifySuccessBlock>(active_window_id, *view_id)
                                {
                                    let warpify_success_block = app.view(&ssh_block);
                                    if let Some(selected_text) =
                                        warpify_success_block.selected_text()
                                    {
                                        selected_texts.push(selected_text);
                                    }
                                }
                            }
                        }
                        BlockHeightItem::Gap(_)
                        | BlockHeightItem::RestoredBlockSeparator { .. }
                        | BlockHeightItem::InlineBanner { .. }
                        | BlockHeightItem::SubshellSeparator { .. } => {}
                    }

                    selection_start_cursor.next();
                }

                if inverted_blocklist {
                    selected_texts.reverse();
                }

                Some(selected_texts.join("\n"))
            }
            Some(ExpandedSelectionRange::Rect { rows }) => {
                let mut selected_texts: Vec<String> = vec![];

                let mut selection_start_cursor = self
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                let original_selection = self
                    .selection
                    .as_ref()
                    .expect("Selection should exist if it can be expanded");

                let head_row = original_selection.head.point.row;
                let tail_row = original_selection.tail.point.row;
                let top_row = head_row.min(tail_row);
                let bottom_row = head_row.max(tail_row);

                selection_start_cursor.seek(&BlockHeight::from(top_row), SeekBias::Right);

                // Loop over each _command block_ row in the rect selection. Add the content to the selected_texts result.
                // Note that there could be rich content blocks in between the command block rows. Therefore in each iteration
                // we need to check and append the intermediate rich content selections.
                for (start, end) in rows {
                    let current_row = start.absolute_point.row;

                    // Read rich content selected text in the intermediate rich content blocks.
                    while current_row >= selection_start_cursor.start().height {
                        if let Some(BlockHeightItem::RichContent(item)) =
                            selection_start_cursor.item()
                        {
                            if let Some(selected_text) =
                                read_selected_text_from_ai_block(item.view_id, app)
                            {
                                selected_texts.push(selected_text);
                            }
                            if let Some(selected_text) =
                                read_selected_text_from_pending_user_query_block(item.view_id, app)
                            {
                                selected_texts.push(selected_text);
                            }
                        }
                        selection_start_cursor.next();
                    }
                    let Some(command_block) = self.block_at(start.within_grid_point.block_index)
                    else {
                        continue;
                    };
                    let start_point = start.within_grid_point.into();
                    let end_point = end.within_grid_point.into();
                    selected_texts.push(command_block.bounds_to_string(start_point, end_point));
                }

                // Read AI block selected text in the trailing AI blocks.
                while bottom_row >= selection_start_cursor.start().height {
                    if let Some(BlockHeightItem::RichContent(item)) = selection_start_cursor.item()
                    {
                        if let Some(selected_text) =
                            read_selected_text_from_ai_block(item.view_id, app)
                        {
                            selected_texts.push(selected_text);
                        }
                        if let Some(selected_text) =
                            read_selected_text_from_pending_user_query_block(item.view_id, app)
                        {
                            selected_texts.push(selected_text);
                        }
                    }
                    selection_start_cursor.next();
                }

                Some(selected_texts.join("\n"))
            }
            None => {
                // Check if there are rich content blocks in the selection. This is to cover
                // an edge case when selection only spans rich content blocks, expand_selection
                // will return None.
                let ids = self.rich_content_blocks_in_selection();

                if ids.is_empty() {
                    return None;
                }

                let mut selected_texts = vec![];
                for view_id in ids {
                    if let Some(selected_text) = read_selected_text_from_ai_block(view_id, app) {
                        selected_texts.push(selected_text);
                    }

                    if let Some(active_window_id) = app.windows().active_window() {
                        if let Some(env_var_block) =
                            app.view_with_id::<EnvVarCollectionBlock>(active_window_id, view_id)
                        {
                            let block = app.view(&env_var_block);
                            if let Some(selected_text) = block.selected_text(app) {
                                selected_texts.push(selected_text);
                            }
                        }

                        if let Some(ssh_block) =
                            app.view_with_id::<WarpifySuccessBlock>(active_window_id, view_id)
                        {
                            let warpify_success_block = app.view(&ssh_block);
                            if let Some(selected_text) = warpify_success_block.selected_text() {
                                selected_texts.push(selected_text);
                            }
                        }
                    }

                    if let Some(selected_text) =
                        read_selected_text_from_pending_user_query_block(view_id, app)
                    {
                        selected_texts.push(selected_text);
                    }
                }

                // TODO: If `selected_texts` is empty, should we return `None` instead of `Some("")`?
                // As of 02/18/2025, this scenario can be reproduced by single-clicking anywhere on an AI response block.
                Some(selected_texts.join("\n"))
            }
        }
    }

    /// Whether there is a renderable selection with the current blocklist config.
    pub fn has_renderable_selection(
        &self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> bool {
        self.renderable_selection(semantic_selection, inverted_blocklist)
            .is_some()
            || !self.rich_content_blocks_in_selection().is_empty()
    }

    pub(super) fn update_selection_after_height_change(&mut self) {
        let max_height = self.block_heights.summary().height;
        if let Some(mut selection) = self.selection.take() {
            let start_point = selection.start_anchor().point;
            let mut end_point = selection.end_anchor().point;

            // Clamp the selection down to the height of the blocklist if its range is past the
            // size of the blocklist. This can happen if a block shrinks in size when it finishes
            // causing the selection within the block to be out of range.
            if end_point.row > max_height {
                end_point = BlockListPoint::new(max_height - 1.into_lines(), self.size.columns);
            }

            // Only keep the selection if it still fits in the block list.
            // Replace the end point of the selection with the end of the blocklist, which
            // could be either the head or the tail of the selection
            if selection.head.point <= selection.tail.point {
                let block_anchor = BlockAnchor::new(end_point, selection.tail.side);
                selection.tail = block_anchor;
            } else {
                let block_anchor = BlockAnchor::new(end_point, selection.head.side);
                selection.head = block_anchor;
            }

            if start_point.row <= max_height {
                self.set_selection(selection);
            }
        }
    }

    /// Update the selection after a grid's content was truncated because it hit the maximum number
    /// of lines in the grid.
    pub(super) fn update_selection_after_grid_truncation(&mut self) {
        let active_block_location = WithinBlock::new(
            (),
            self.active_block_index(),
            self.active_block().active_grid_type(),
        );

        let within_block_points = self
            .selection
            .as_ref()
            .and_then(|selection| selection.clone().clamp_to_grid_points(self));

        if let Some((mut start, mut end)) = within_block_points {
            if start > end {
                mem::swap(&mut start, &mut end);
            }
            if start.in_same_block_and_grid(&active_block_location) {
                if let Some(selection) = self.selection.as_mut() {
                    // If the start of the selection is at the first row of the grid, clamp the
                    // selection to the first point in the grid so that that a previous grid
                    // (which wasn't previously selected) is not selected.
                    if start.get().row == 0 {
                        selection.start_anchor().point.column = 0;
                    } else {
                        selection.start_anchor().point.row = max(
                            selection.start_anchor().point.row - 1.into_lines(),
                            Lines::zero(),
                        );
                    }
                }
            }

            if end.in_same_block_and_grid(&active_block_location) {
                if let Some(mut selection) = self.selection.take() {
                    selection.end_anchor().point.row = max(
                        selection.end_anchor().point.row - 1.into_lines(),
                        Lines::zero(),
                    );

                    // If the selection is at the first row in the grid, clear the selection if
                    // the start was already truncated by linefeed (since the selection contents
                    // have been truncated away entirely). If the start was not truncated by
                    // linefeed already, this means the start is in a prior block, so the set
                    // the end of the selection to be at the last column of the previous block
                    // grid.
                    if end.get().row == 0 {
                        if !start.in_same_block_and_grid(&end) {
                            selection.end_anchor().point.column = self.size.columns() - 1;
                            self.set_selection(selection);
                        }
                    } else {
                        self.set_selection(selection);
                    }
                }
            }
        }
    }

    fn set_selection(&mut self, value: BlockListSelection) {
        self.selection = Some(value);
        self.event_proxy
            .send_terminal_event(TerminalEvent::TextSelectionChanged);
    }

    /// Return the list of corresponding rich content block view ids contained in the active
    /// text selection.
    fn rich_content_blocks_in_selection(&self) -> Vec<EntityId> {
        let Some(original_selection) = self.selection.as_ref() else {
            return vec![];
        };
        let mut top_row = original_selection.head.point.row;
        let mut bottom_row = original_selection.tail.point.row;

        // Ensure that top_row is always above bottom_row so we can loop based on block heights.
        if original_selection.tail.point.row < original_selection.head.point.row {
            top_row = original_selection.tail.point.row;
            bottom_row = original_selection.head.point.row;
        }

        let mut rich_block_view_ids = vec![];

        let mut selection_start_cursor = self
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        selection_start_cursor.seek(&BlockHeight::from(top_row), SeekBias::Right);
        while let Some(item) = selection_start_cursor.item() {
            if let BlockHeightItem::RichContent(RichContentItem { view_id, .. }) = item {
                rich_block_view_ids.push(*view_id);
            }

            // We reached the bottom row. We could break now.
            if selection_start_cursor.end_seek_position().into_lines() >= bottom_row {
                break;
            }
            selection_start_cursor.next();
        }

        rich_block_view_ids
    }

    /// Converts the underlying selection into a pair of ordered WithinBlock<Point>s
    /// Takes a (head, tail) pair and returns a (start, end) pair, which as an invariant
    /// of this function are now ordered.
    fn expand_selection(
        &self,
        semantic_selection: &SemanticSelection,
        inverted_blocklist: bool,
    ) -> Option<ExpandedSelectionRange<ExpandedSelection>> {
        let mut block_list_selection = self.selection.clone()?;

        if block_list_selection.is_empty() {
            return None;
        }

        let mut swapped = false;
        let is_rect = block_list_selection.selection_type.into();
        let (start, end) = block_list_selection.clamp_to_grid_points(self).and_then(
            |(mut start_grid_point, mut end_grid_point)| {
                // The selection endpoints must be ordered from start --> end
                let mut start = block_list_selection.head;
                let mut end = block_list_selection.tail;

                swapped = self.order_selection_endpoints(
                    &mut start_grid_point,
                    &mut start,
                    &mut end_grid_point,
                    &mut end,
                    inverted_blocklist,
                );

                // The start and end points are in the same grid--create a grid selection within
                // the grid and convert it to a range to determine the start and end points of
                // the expanded selection
                if start_grid_point.in_same_block_and_grid(&end_grid_point) {
                    let mut selection = Selection::new(
                        block_list_selection.selection_type,
                        *start_grid_point.get(),
                        start.side,
                    );
                    if swapped {
                        selection.set_smart_select_side(Direction::Right);
                    } else {
                        selection.set_smart_select_side(Direction::Left);
                    }
                    if let Some(smart_select_override) = &block_list_selection.smart_select_override
                    {
                        if start_grid_point.in_same_block_and_grid(smart_select_override.start())
                            && start_grid_point.in_same_block_and_grid(smart_select_override.end())
                        {
                            selection.set_smart_select_override(
                                *smart_select_override.start().get()
                                    ..=*smart_select_override.end().get(),
                            );
                        }
                    }
                    selection.update(*end_grid_point.get(), end.side);

                    let grid = self.grid_at_location(&start_grid_point);
                    let selection_range =
                        selection.to_range(&grid.grid_handler, semantic_selection)?;

                    start_grid_point.replace(selection_range.start);
                    end_grid_point.replace(selection_range.end);

                    Some((start_grid_point, end_grid_point))
                } else {
                    // The start and end points are in different grids. Create a grid selection
                    // from the starting point to the end of the grid and another selection from
                    // the start of the grid to the end point.
                    let start_point = {
                        let mut selection = Selection::new(
                            block_list_selection.selection_type,
                            *start_grid_point.get(),
                            start.side,
                        );

                        if !swapped {
                            selection.set_smart_select_side(Direction::Left);
                        }

                        if let Some(smart_select_override) =
                            &block_list_selection.smart_select_override
                        {
                            if start_grid_point
                                .in_same_block_and_grid(smart_select_override.start())
                            {
                                selection.set_smart_select_override(
                                    *smart_select_override.start().get()
                                        ..=*smart_select_override.end().get(),
                                );
                            }
                        }

                        let grid = self.grid_at_location(&start_grid_point);

                        selection.update(
                            Point::new(
                                grid.len_displayed().saturating_sub(1),
                                self.size().columns.saturating_sub(1),
                            ),
                            Side::Right,
                        );

                        let selection_range =
                            selection.to_range(&grid.grid_handler, semantic_selection)?;

                        start_grid_point.replace(selection_range.start);
                        start_grid_point
                    };

                    let end_point = {
                        let mut selection = Selection::new(
                            block_list_selection.selection_type,
                            Point::new(0, 0),
                            Side::Left,
                        );

                        selection.update(*end_grid_point.get(), end.side);
                        if swapped {
                            selection.set_smart_select_side(Direction::Right);
                        }

                        if let Some(smart_select_override) =
                            &block_list_selection.smart_select_override
                        {
                            if end_grid_point.in_same_block_and_grid(smart_select_override.end()) {
                                selection.set_smart_select_override(
                                    *smart_select_override.start().get()
                                        ..=*smart_select_override.end().get(),
                                );
                            }
                        }

                        let grid = self.grid_at_location(&end_grid_point);
                        let selection_range =
                            selection.to_range(&grid.grid_handler, semantic_selection)?;
                        end_grid_point.replace(selection_range.end);
                        end_grid_point
                    };

                    Some((start_point, end_point))
                }
            },
        )?;
        match is_rect {
            IsRect::False => {
                let start_absolute = BlockListPoint::from_within_block_point(&start, self);
                let end_absolute = BlockListPoint::from_within_block_point(&end, self);
                Some(ExpandedSelectionRange::regular(
                    ExpandedSelection::new(start_absolute, start),
                    ExpandedSelection::new(end_absolute, end),
                    swapped,
                ))
            }
            IsRect::True => {
                let mut current = start;
                let mut end = end;
                let mut selected_rows = Vec::new();

                // Make sure the end column is always larger than the start column. This prevents the edge case when we miss including the last
                // line of the rect selection when dragging from bottom left to top right or top right to bottom left.
                if end.inner.col < current.inner.col {
                    mem::swap(&mut end.inner.col, &mut current.inner.col);
                }
                let end_inner = end.get();

                // Move down current point until we cross the end point.
                while current.is_visually_before(&end, inverted_blocklist) {
                    let end_in_row = current.map(|inner| Point {
                        row: inner.row,
                        col: end_inner.col,
                    });
                    let start_in_row = current;

                    let start_absolute =
                        BlockListPoint::from_within_block_point(&start_in_row, self);
                    let end_absolute = BlockListPoint::from_within_block_point(&end_in_row, self);

                    selected_rows.push((
                        ExpandedSelection::new(start_absolute, start_in_row),
                        ExpandedSelection::new(end_absolute, end_in_row),
                    ));

                    // Sanity check -- if the current point is on the exact same row as the end, we should break.
                    if current.in_same_block_and_grid(&end) && current.get().row == end.get().row {
                        break;
                    }

                    match self.move_point_down(current, inverted_blocklist) {
                        Some((point, _)) => current = point,
                        None => break,
                    };
                }

                match Vec1::try_from_vec(selected_rows) {
                    Ok(result) => Some(ExpandedSelectionRange::rect(result)),
                    Err(_) => {
                        log::warn!("Failed to create a new rect selection");
                        None
                    }
                }
            }
        }
    }

    /// Swaps endpoints into sorted order so they become the start and end of the selection. Accounts
    /// for whether the blocklist renders as inverted (i.e., Input at Top). Returns a boolean indicating
    /// whether a swap was necessary
    fn order_selection_endpoints(
        &self,
        head_grid: &mut WithinBlock<Point>,
        head_anchor: &mut BlockAnchor,
        tail_grid: &mut WithinBlock<Point>,
        tail_anchor: &mut BlockAnchor,
        inverted_blocklist: bool,
    ) -> bool {
        let needs_swap = if head_grid.block_index == tail_grid.block_index {
            // If the points are in the same block, we can do a standard comparison
            head_grid > tail_grid
                || (head_grid == tail_grid
                    && head_anchor.side == Side::Right
                    && tail_anchor.side == Side::Left)
        } else {
            // For different blocks, it depends on whether the blocklist renders as inverted
            (head_grid.block_index > tail_grid.block_index) ^ (inverted_blocklist)
        };

        if needs_swap {
            mem::swap(head_grid, tail_grid);
            mem::swap(head_anchor, tail_anchor);
        }
        needs_swap
    }

    // Clip selection start to the right index when encountering a wide character (e.g. CJK characters)
    // A wide character is made up of a WIDE_CHAR cell followed by WIDE_CHAR_SPACER cell.
    // So when the selection starts on a WIDE_CHAR_SPACER, we should clip it to the start of WIDE_CHAR.
    fn clip_selection_start(&self, start: &ExpandedSelection) -> BlockListPoint {
        let within_grid_point = start.within_grid_point;
        let block = self
            .block_at(within_grid_point.block_index)
            .expect("Block index should be valid.");
        let point = within_grid_point.get();

        let cell_type = block
            .grid_of_type(within_grid_point.grid)
            .and_then(|grid| grid.grid_handler().cell_type(*point));

        if matches!(cell_type, Some(CellType::WideCharSpacer)) {
            return BlockListPoint {
                row: start.absolute_point.row,
                column: start.absolute_point.column.saturating_sub(1),
            };
        }
        start.absolute_point
    }

    // Clip selection end to the right index when encountering a wide character (e.g. CJK characters)
    // A wide character is made up of a WIDE_CHAR cell followed by WIDE_CHAR_SPACER cell.
    // So when the selection ends on a WIDE_CHAR, we should clip it to the end of WIDE_CHAR_SPACER.
    fn clip_selection_end(&self, end: &ExpandedSelection) -> BlockListPoint {
        let within_grid_point = end.within_grid_point;
        let block = self
            .block_at(end.within_grid_point.block_index)
            .expect("Block index should be valid.");

        let point = end.within_grid_point.get();

        let cell_type = block
            .grid_of_type(within_grid_point.grid)
            .and_then(|grid| grid.grid_handler().cell_type(*point));

        if matches!(cell_type, Some(CellType::WideChar)) {
            return BlockListPoint {
                row: end.absolute_point.row,
                column: (end.absolute_point.column + 1).min(self.size.columns - 1),
            };
        }
        end.absolute_point
    }
}

/// Given the view id of an AI block, return the active selected text in that block.
fn read_selected_text_from_ai_block(view_id: EntityId, app: &AppContext) -> Option<String> {
    let active_window_id = app.windows().active_window()?;

    let ai_block = app.view_with_id::<AIBlock>(active_window_id, view_id)?;
    let ai_block_view = app.view(&ai_block);
    ai_block_view.selected_text(app)
}

/// Given the view id of a pending user query block, return the active selected text in that block.
fn read_selected_text_from_pending_user_query_block(
    view_id: EntityId,
    app: &AppContext,
) -> Option<String> {
    let active_window_id = app.windows().active_window()?;

    let pending_user_query_block =
        app.view_with_id::<PendingUserQueryBlock>(active_window_id, view_id)?;
    let pending_user_query_block_view = app.view(&pending_user_query_block);
    pending_user_query_block_view.selected_text(app)
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
