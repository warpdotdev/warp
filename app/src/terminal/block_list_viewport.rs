use std::ops::Range;
use std::rc::Rc;
use std::sync::MutexGuard;

use pathfinder_geometry::vector::Vector2F;
use serde::{Deserialize, Serialize};
use sum_tree::{Cursor, SeekBias};
use warp_core::features::FeatureFlag;
use warpui::elements::ClippedScrollStateHandle;
use warpui::units::{IntoLines, IntoPixels, Lines, Pixels};
use warpui::{AppContext, ModelHandle};

use super::block_list_element::{
    GridType, SnackbarHeader, SnackbarHeaderState, SnackbarPoint, VisibleItem,
};
use super::model::block::{Block, BlockSection};
use super::model::blocks::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, BlockListPoint, SelectionRange,
    TotalIndex,
};
use super::model::selection::SelectionPoint;
use super::model::terminal_model::{BlockIndex, BlockSortDirection, WithinBlock};
use super::view::BlockVisibilityMode;
use super::{
    height_in_range_approx, heights_approx_gt, heights_approx_gte, heights_approx_lt,
    heights_approx_lte, SizeInfo, HEIGHT_FUDGE_FACTOR_LINES,
};
use crate::ai::blocklist::agent_view::{AgentViewDisplayMode, AgentViewState};
use crate::terminal::input::inline_menu::InlineMenuPositioner;
use crate::terminal::model::blocks::RichContentItem;
use crate::terminal::model::index::Point as IndexPoint;

/// Wraps a scroll position for the purposes of centralizing update logic.
pub struct ScrollState {
    position: ScrollPosition,
}

impl ScrollState {
    pub fn new(position: ScrollPosition) -> Self {
        Self { position }
    }

    pub fn position(&self) -> ScrollPosition {
        self.position
    }

    /// Possibly updates the scroll position, returning whether it was actually changed.
    pub fn update(
        &mut self,
        viewport: ViewportState,
        update: ScrollPositionUpdate,
        app: &AppContext,
    ) -> bool {
        let next_position = viewport.next_scroll_position(update, app);
        if next_position != self.position {
            log::debug!(
                "updating scroll position from {:?} to {:?} for update {:?}",
                self.position,
                next_position,
                update
            );
            self.position = next_position;
            return true;
        }
        false
    }
}

/// Defines whether we are tracking scroll position from the top or bottom of the
/// blocklist.  In modes where the input is below the blocklist (PinnedToBottom and
/// Waterfall), we track scroll position from the top, so that when there is a long
/// running command in those modes we can maintain scroll position.  For modes where
/// the block list is inverted (PinnedToTop), we track scroll position from the bottom
/// for the same reason.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ScrollLines {
    /// Scroll position is measured in lines from the top of the block list
    /// e.g. ScrollTop(0) == the top of the block list.
    ScrollTop(Lines),

    /// Scroll position is measured in lines from the bottom of the block list
    /// e.g. ScrollBottom(0) == the bottom of the block list.
    ScrollBottom(Lines),
}

impl ScrollLines {
    /// Convert from scroll top to ScrollLines, taking into account the input mode.
    fn from_scroll_top(
        scroll_top: Lines,
        input_mode: InputMode,
        block_list: &BlockList,
        content_element_height: Lines,
    ) -> Self {
        match input_mode {
            InputMode::PinnedToBottom | InputMode::Waterfall => ScrollLines::ScrollTop(scroll_top),
            InputMode::PinnedToTop => {
                let active_block = block_list.active_block();
                let is_long_running = active_block.is_active_and_long_running();

                // The behavior for pinned to top is somewhat subtle...
                //
                // If you are scrolled into a long-running block, we want to maintain
                // your scroll position as the block grows downward, so we need to anchor
                // using scroll top.
                //
                // Otherwise we want to use scroll bottom so that the block that is growing
                // above you doesn't make you lose your position when you are scrolled down.
                if is_long_running
                    && scroll_top
                        < active_block
                            .height(block_list.agent_view_state())
                            .into_lines()
                {
                    ScrollLines::ScrollTop(scroll_top)
                } else {
                    ScrollLines::ScrollBottom(
                        block_list.block_heights().summary().height
                            - content_element_height
                            - scroll_top,
                    )
                }
            }
        }
    }

    /// Get the scroll top from the scroll lines, taking into account the input mode.
    fn scroll_top(&self, block_list: &BlockList, content_element_height: Lines) -> Lines {
        match *self {
            ScrollLines::ScrollTop(scroll_top) => scroll_top,
            ScrollLines::ScrollBottom(scroll_bottom) => {
                block_list.block_heights().summary().height - content_element_height - scroll_bottom
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ScrollPosition {
    /// The scrolling follows the bottom of the most recently executed block.
    /// In terms of scroll_top, this implies scrolling stays locked to max_scroll_top.
    FollowsBottomOfMostRecentBlock,

    /// The scrolling follows the bottom of the most recently executed block,
    /// similar to FollowsBottomOfMostRecentBlock, but because there can be a gap
    /// below the most recent block, the scroll_top is not necessarily max_scroll_top.
    /// We store the scroll top and use that.
    WaterfallGapFollowsBottomOfMostRecentBlock { scroll_top_in_lines: Lines },

    /// Scrolling is set to a particular scroll_top (offset in lines from the top
    /// of the block list)
    FixedAtPosition { scroll_lines: ScrollLines },

    /// The scrolling follows an offset within a long-running block and
    /// adjusts for output grid truncation.
    ///
    /// In other words, this allows a user to lock their scroll position at
    /// a particular line and have the scroll follow that line even while the
    /// output grid is being truncated at head (until the line itself is truncated).
    FixedWithinLongRunningBlock {
        /// The absolute scroll offset.
        /// This is equivalent to [`ScrollPosition::FixedAtPosition::scroll_lines`].
        scroll_lines: ScrollLines,

        /// The number of lines truncated from the output grid
        /// at the time that the scroll position was set.
        num_output_lines_truncated: u64,
    },
}

/// Represents the location of a find match to be used for calculating scroll position.
#[derive(Debug, Clone, Copy)]
pub enum FindMatchScrollLocation {
    /// For matches occurring in command blocks.
    Block {
        block_index: BlockIndex,

        /// `BlockSection` describing the relative position of the match within the block.
        section: BlockSection,
    },
    /// For matches occurring in rich content views.
    RichContent {
        /// The total index of the rich content item in the blocklist sumtree.
        index: TotalIndex,
    },
}

/// An enum of all possible scroll position updates, useful for centralizing
/// scroll update logic.
#[derive(Debug, Clone, Copy)]
pub enum ScrollPositionUpdate {
    AfterCommandExecutionStarted,
    AfterRichBlockInserted,
    AfterRichBlockUpdated,
    AfterKeydownOnTerminal,
    AfterTypedCharacters,
    AfterWriteUserBytesToPty,
    AfterScrollEvent {
        scroll_delta: Lines,
    },
    AfterResize,
    AfterClear,
    AfterPageUp,
    AfterPageDown,
    AfterHome,
    AfterEnd,
    AfterFilter {
        block_index: BlockIndex,
        prev_top_of_viewport: Lines,
        prev_bottom_of_block: Lines,
        prev_first_visible_original_row: Option<usize>,
    },
    AfterFilterClear {
        block_index: BlockIndex,
        offset_from_block_top: Lines,
    },
    ScrollToTopOfBlock {
        block_index: BlockIndex,
    },
    ScrollToBottomOfBlock {
        block_index: BlockIndex,
    },
    ScrollMostRecentBlockIntoView,
    ScrollToBlocklistRowIfNotVisible {
        row: Lines,
    },
    ScrollToFindMatchIfNotVisible(FindMatchScrollLocation),
    ScrollToTopOfBlockWithBuffer {
        block_index: BlockIndex,
        buffer_lines: Lines,
    },
    ScrollToTopOfRichContent {
        index: TotalIndex,
    },
    AfterEnterAgentView,
    AfterExitAgentView {
        saved_position: ScrollPosition,
    },
}

/// The direction that blocks flow in the viewport.
#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Direction that blocks flow in the terminal viewport.",
    rename_all = "snake_case"
)]
pub enum InputMode {
    /// The most recent blocks are at the bottom of the screen and new blocks
    /// are added at the bottom as the blocklist grows
    #[default]
    PinnedToBottom,

    /// The most recent blocks are at the top of the screen and new blocks
    /// are added at the top as the blocklist grows
    PinnedToTop,

    /// The input starts at the top and gets pushed down by commands above it.
    Waterfall,
}

impl InputMode {
    pub fn is_inverted_blocklist(&self) -> bool {
        self.is_pinned_to_top()
    }

    pub fn is_pinned_to_top(&self) -> bool {
        matches!(self, InputMode::PinnedToTop)
    }

    pub fn block_sort_direction(&self) -> BlockSortDirection {
        if self.is_inverted_blocklist() {
            BlockSortDirection::MostRecentFirst
        } else {
            BlockSortDirection::MostRecentLast
        }
    }
}

pub enum ClampingMode {
    // If the point is within the block, clamp to the nearest grid point. For
    // example, if the point is within the padding to the left of the command
    // grid clamp to the nearest point within the command grid.
    ClampToGridIfWithinBlock,

    // Regardless of whether the point is in a block, clamp to the nearest grid point.
    ClampToGrid,

    // If the point isn't contained within the grid, return none.
    ReturnNoneIfNotInGrid,
}

/// A block that is partially scrolled off the bottom of the screen.
#[derive(Debug, Clone, Copy)]
pub struct OverhangingBlock {
    visible_block_height_px: Pixels,
    block_index: BlockIndex,
    is_most_recent_block: bool,
}

impl OverhangingBlock {
    /// The height of the block that is scrolled into view.
    pub fn visible_block_height_px(&self) -> Pixels {
        self.visible_block_height_px
    }

    /// The index of the block.
    pub fn block_index(&self) -> BlockIndex {
        self.block_index
    }

    /// Whether this is the most recently executed block.
    pub fn is_most_recent_block(&self) -> bool {
        self.is_most_recent_block
    }
}

/// An iterator over the block list items in the current viewport
/// that takes into account whether the block list is inverted, always returning
/// blocks from screen top to to screen bottom.  Note that the indices of all
/// items returned by this iterator are always in model coordinates, so that most
/// recently executed blocks have higher indices than older blocks.
///
/// E.g.  If you are iterating over a blocklist that is in "non-inverted mode"
/// the blocks will be returned from top to bottom visually, but the indices will
/// be lower with the topmost blocks and higher with the bottommost because the
/// bottommost are the most recent.
///
/// In "inverted "mode, the topmost blocks will have the highest model indices because
/// they are most recent.
pub struct ViewportIter<'a> {
    /// The underlying cursor into the block heights that we are iterating over
    /// For `BlockDirection::MostRecentOnBottom` this is a forward iterator.
    /// For `BlockDirection::MostRecentOnTop` this is a reverse iterator.
    block_heights_iter: Box<dyn Iterator<Item = &'a BlockHeightItem> + 'a>,

    /// The current input mode.
    input_mode: InputMode,
    agent_view_state: &'a AgentViewState,

    /// The y-offset of the current block in lines from the viewport origin.
    top_of_current_block: Lines,

    /// The y-offset of the bottom of the viewport in lines from the viewport origin.
    bottom_offset: Lines,

    /// The index of the first block being iterated over.
    start_block_index: BlockIndex,

    /// The index of the current block being iterated over.
    curr_block_index: BlockIndex,

    /// The index of the last item (block or otherwise) we have iterated over.
    curr_entry_index: TotalIndex,
}

impl ViewportIter<'_> {
    /// Returns the visible block range iterated over so far.
    pub fn visible_block_range(&self) -> Range<BlockIndex> {
        match self.input_mode {
            InputMode::PinnedToBottom | InputMode::Waterfall => {
                self.start_block_index..(self.curr_block_index + 1.into())
            }
            InputMode::PinnedToTop => self.curr_block_index..(self.start_block_index + 1.into()),
        }
    }
}

/// An item in the viewport, along with the indices it's at.  These are indices
/// into the block list, not indices representing the order the blocks are
/// rendered.
#[derive(Debug)]
pub struct ViewportIterItem {
    /// An optional block index, only defined if this is a block.
    pub block_index: Option<BlockIndex>,

    /// The index into the entire block heights list. Defined for all items.
    pub entry_index: TotalIndex,

    /// The height of the current item.
    pub block_height_item: BlockHeightItem,

    /// The top of the current block in terms of lines from the top of the first block
    /// in the viewport.  Always starts at zero for the first iterator item.
    pub top_of_current_block: Lines,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AutoscrollBehavior {
    /// Always autoscroll when an action completes.
    Always,
    /// When an action completes, only autoscroll if the scroll position is at the end of
    /// the blocklist.
    WhenScrolledToEnd,
}

/// This struct encapsulates all the data we need to do a viewport calculation.
/// Callers should not hold onto this state for long periods (and it will be
/// hard to in any case because of the lifetime params). Specifically the usage
/// pattern is to create a viewport for a paritcular blocklist state, do some
/// calculations and then discard it.
///
/// To do viewport calculations you need a reference to the blocklist (which is of
/// limited lifetime because of how model locking works), plus all
/// the info related to scroll position and screen size.
pub struct ViewportState<'a> {
    block_list: &'a BlockList,
    snackbar_header_state: SnackbarHeaderState,
    input_mode: InputMode,
    size_info: SizeInfo,
    scroll_position: ScrollPosition,
    horizontal_clipped_scroll_state: ClippedScrollStateHandle,

    /// Cached visible items to use in place of re-iterating to generate them
    visible_items: Option<Rc<Vec<VisibleItem>>>,

    /// The size of the BlocklistElement.
    blocklist_element_size: Vector2F,

    /// The size of the terminal input view, presumably from the last layout.
    input_size: Vector2F,

    /// Autoscroll behavior for rich content blocks.
    rich_block_autoscroll_behavior: AutoscrollBehavior,

    inline_menu_positioner: ModelHandle<InlineMenuPositioner>,
}

impl<'a> ViewportState<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_list: &'a BlockList,
        snackbar_header_state: SnackbarHeaderState,
        input_mode: InputMode,
        size_info: SizeInfo,
        scroll_position: ScrollPosition,
        visible_items: Option<Rc<Vec<VisibleItem>>>,
        horizontal_clipped_scroll_state: ClippedScrollStateHandle,
        blocklist_element_size: Vector2F,
        input_size: Vector2F,
        rich_block_autoscroll_behavior: AutoscrollBehavior,
        inline_menu_positioner: ModelHandle<InlineMenuPositioner>,
    ) -> Self {
        Self {
            block_list,
            snackbar_header_state,
            input_mode,
            size_info,
            scroll_position,
            horizontal_clipped_scroll_state,
            visible_items,
            blocklist_element_size,
            input_size,
            rich_block_autoscroll_behavior,
            inline_menu_positioner,
        }
    }

    fn snackbar_header_state(&self) -> MutexGuard<'_, SnackbarHeader> {
        self.snackbar_header_state
            .state_handle
            .lock()
            .expect("locking snackbar header state")
    }

    fn content_element_height_lines(&self) -> Lines {
        Pixels::new(self.blocklist_element_size.y()).to_lines(self.size_info.cell_height_px)
    }

    /// Returns a new iterator over the viewport that respects the viewport's
    /// block direction.  See the ViewportIter struct docs for more info.
    pub fn iter(&self) -> ViewportIter<'a> {
        self.iter_from(self.scroll_top_in_lines())
    }

    /// Returns a new iterator over the viewport that respects the viewport's
    /// block direction.  See the ViewportIter struct docs for more info.
    ///
    /// This will include items that are below the bottom of the viewport by the given
    /// bottom_overhang amount.
    pub fn iter_with_bottom_overhang(&self, bottom_overhang: Lines) -> ViewportIter<'a> {
        let bottom_offset =
            self.scroll_top_in_lines() + self.content_element_height_lines() + bottom_overhang;
        self.iter_range(self.scroll_top_in_lines(), bottom_offset)
    }

    // Returns a new iterator starting from the given scroll position in lines.
    fn iter_from(&self, top_offset: Lines) -> ViewportIter<'a> {
        let bottom_offset = self.scroll_top_in_lines() + self.content_element_height_lines();
        self.iter_range(top_offset, bottom_offset)
    }

    // Returns a new iterator that spans the given line range.
    fn iter_range(&self, top_offset: Lines, bottom_offset: Lines) -> ViewportIter<'a> {
        let cursor = self.block_height_cursor(top_offset);

        match self.input_mode {
            InputMode::PinnedToBottom | InputMode::Waterfall => {
                let block_index = BlockIndex::from(cursor.start().block_count);
                let curr_entry_index = cursor.start().total_count;
                let top_of_block = cursor.start().height;
                ViewportIter {
                    block_heights_iter: Box::new(cursor),
                    input_mode: self.input_mode,
                    agent_view_state: self.block_list.agent_view_state(),
                    top_of_current_block: top_of_block,
                    bottom_offset,
                    start_block_index: block_index,
                    curr_block_index: block_index,
                    curr_entry_index: curr_entry_index.into(),
                }
            }
            InputMode::PinnedToTop => {
                let total_block_height = self.block_list.block_heights().summary().height;
                // Note that we use cursor.end() in the backwards case because
                // when you are iterating in reverse, the end gives you the correct first
                // element.
                let backwards_end = cursor.end();
                let block_index = BlockIndex::from(backwards_end.block_count);
                let curr_entry_index = backwards_end.total_count;
                let top_of_block = total_block_height - backwards_end.height;
                ViewportIter {
                    block_heights_iter: Box::new(cursor.rev()),
                    input_mode: self.input_mode,
                    agent_view_state: self.block_list.agent_view_state(),
                    top_of_current_block: top_of_block,
                    bottom_offset,
                    start_block_index: block_index,
                    curr_block_index: block_index,
                    curr_entry_index: curr_entry_index.into(),
                }
            }
        }
    }

    fn block_height_cursor(
        &self,
        top_offset: Lines,
    ) -> Cursor<'a, BlockHeightItem, BlockHeight, BlockHeightSummary> {
        match self.input_mode {
            InputMode::PinnedToBottom | InputMode::Waterfall => {
                let mut forward_cursor = self
                    .block_list
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                forward_cursor.seek_clamped(&BlockHeight::from(top_offset), SeekBias::Right);
                forward_cursor
            }
            InputMode::PinnedToTop => {
                let total_block_height = self.block_list.block_heights().summary().height;
                let mut backwards_cursor = self
                    .block_list
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                // Note that we seek_clamped here because accumulated floating point errors
                // in adjusting block list heights make it possible that the total_block_height
                // we receive from the summary actually very slightly exceeds what the
                // cursor thinks is the max point to seek to.  In the case with a zero scroll top
                // this can make seeks fail without the clamp.  We want to cap the seek at the
                // end, not have the seek fail in this case.
                let seek_to = (total_block_height - top_offset).max(Lines::zero());
                backwards_cursor.seek_clamped(&BlockHeight::from(seek_to), SeekBias::Left);
                backwards_cursor
            }
        }
    }

    /// Returns where the first block starts relative to the grid origin in pixels
    /// If there is no top block, returns a zero offset
    pub fn offset_to_top_of_first_block(&self, app: &AppContext) -> Pixels {
        let total_block_height = self.block_list.block_heights().summary().height;

        let top_of_current_block = if let Some(visible_items) = &self.visible_items {
            let mut block_heights_cursor = self
                .block_list
                .block_heights()
                .cursor::<TotalIndex, BlockHeightSummary>();
            let first_index = visible_items
                .first()
                .map(|item| item.index())
                .unwrap_or_else(|| 0.into());
            block_heights_cursor.seek(&first_index, SeekBias::Right);
            match self.input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => {
                    block_heights_cursor.start().height
                }
                InputMode::PinnedToTop => total_block_height - block_heights_cursor.end().height,
            }
        } else {
            let mut viewport_iter = self.iter();
            let top_item = viewport_iter.next().expect("should be a top item to paint");
            top_item.top_of_current_block
        };
        let content_element_lines = self.content_element_height_lines();
        let top_offset = self.scroll_top_in_lines();

        // Move the grid origin upwards to start at the top of the current block
        let mut adjustment =
            -(top_offset - top_of_current_block).to_pixels(self.size_info.cell_height_px());

        if matches!(self.input_mode, InputMode::PinnedToBottom) {
            // Take into account the case where the blocks don't fill up the entire grid
            if content_element_lines > total_block_height {
                adjustment += (content_element_lines - total_block_height)
                    .to_pixels(self.size_info.cell_height_px());
            }
        }

        if self.block_list.active_gap().is_some() && !self.is_input_rendered_at_bottom_of_pane(app)
        {
            // Apply a paint-time translation to the blocklist that accounts for inline menu
            // visibility/positioning, effectively "sliding" the blocklist contents upwards to
            // preserve the current input position.
            //
            // See doc comments on blocklist_top_inset_when_in_waterfall_mode for context.
            if let Some(blocklist_inset) = self
                .inline_menu_positioner
                .as_ref(app)
                .blocklist_top_inset_when_in_waterfall_mode(app)
            {
                adjustment -= blocklist_inset;
            }
        }

        adjustment
    }

    fn scroll_lines_from_scroll_top(&self, scroll_top: Lines) -> ScrollLines {
        ScrollLines::from_scroll_top(
            scroll_top,
            self.input_mode,
            self.block_list,
            self.content_element_height_lines(),
        )
    }

    /// How far the view is scrolled from the top of all blocks in lines.
    pub fn scroll_top_in_lines(&self) -> Lines {
        match (self.input_mode, self.scroll_position) {
            (
                InputMode::PinnedToBottom,
                ScrollPosition::FollowsBottomOfMostRecentBlock
                | ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock { .. },
            ) => self.max_scroll_top_in_lines(),
            (InputMode::Waterfall, ScrollPosition::FollowsBottomOfMostRecentBlock) => {
                self.max_scroll_top_in_lines()
            }
            (
                InputMode::Waterfall,
                ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock {
                    scroll_top_in_lines,
                },
            ) => {
                if self.block_list.active_gap().is_some() {
                    let max_scroll_top_lines = self.max_scroll_top_in_lines();
                    if max_scroll_top_lines - scroll_top_in_lines
                        > self.content_element_height_lines()
                    {
                        // If you are scrolled completely above the viewport, then set the scroll
                        // position so that the bottom of the latest command is directly above
                        // the input.
                        max_scroll_top_lines - self.content_element_height_lines()
                    } else {
                        // Otherwise, if you are partially scrolled into the viewport, maintain
                        // the current scroll position.
                        scroll_top_in_lines
                    }
                } else {
                    self.max_scroll_top_in_lines()
                }
            }
            (
                InputMode::PinnedToTop,
                ScrollPosition::FollowsBottomOfMostRecentBlock
                | ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock { .. },
            ) => {
                let index = self.block_list.last_non_hidden_block_by_index();

                // If there is a visible rich content block after the last non hidden block, return the height
                // of it. Otherwise, return the height of the last non hidden block.
                let height = match self
                    .block_list
                    .last_non_hidden_rich_content_block_after_block(index)
                    .map(|(_, content)| content.last_laid_out_height)
                {
                    Some(height) => Some(height.into_lines()),
                    None => index.and_then(|last_index| {
                        self.block_list.block_at(last_index).map(|block| {
                            block
                                .height(self.block_list.agent_view_state())
                                .into_lines()
                        })
                    }),
                };

                height
                    .map(|height| height - self.content_element_height_lines())
                    .unwrap_or(Lines::zero())
            }
            (_, ScrollPosition::FixedAtPosition { scroll_lines }) => {
                scroll_lines.scroll_top(self.block_list, self.content_element_height_lines())
            }
            (
                _,
                ScrollPosition::FixedWithinLongRunningBlock {
                    scroll_lines,
                    num_output_lines_truncated,
                },
            ) => {
                // Adjust the scroll-top by the number of lines
                // truncated since we set our scroll position.
                let adjustment = self
                    .block_list
                    .active_block()
                    .output_grid()
                    .grid_handler()
                    .num_lines_truncated()
                    .saturating_sub(num_output_lines_truncated)
                    .into_lines();
                let unadjusted_scroll_top =
                    scroll_lines.scroll_top(self.block_list, self.content_element_height_lines());
                let adjusted_scroll_top = unadjusted_scroll_top - adjustment;

                // We only want to adjust the scroll position as far up as the block goes.
                let top_of_block = self.top_of_block_in_lines(self.block_list.active_block_index());
                std::cmp::max(adjusted_scroll_top, top_of_block)
            }
        }
        .max(Lines::zero())
        .min(self.max_scroll_top_in_lines())
    }

    /// How far the view is scrolled from the top of all blocks in pixels
    pub fn scroll_top_in_pixels(&self) -> Pixels {
        self.scroll_top_in_lines()
            .to_pixels(self.size_info.cell_height_px())
    }

    /// Returns the index of the topmost visible block in the viewport
    #[cfg(test)]
    pub fn topmost_visible_block(&self) -> Option<BlockIndex> {
        self.iter().next().and_then(|item| item.block_index)
    }

    pub fn next_scroll_position(
        &self,
        update: ScrollPositionUpdate,
        app: &AppContext,
    ) -> ScrollPosition {
        match update {
            ScrollPositionUpdate::AfterCommandExecutionStarted => {
                self.scroll_position_after_command_execution(app)
            }
            // When a rich block is inserted, we make the same scroll change
            // as we would when a command is executed (i.e. scroll to end).
            ScrollPositionUpdate::AfterRichBlockInserted
            | ScrollPositionUpdate::AfterRichBlockUpdated => {
                // If the user has scrolled to some fixed position, check the autoscroll behavior
                // before overriding it and scrolling to the end.
                if self.rich_block_autoscroll_behavior != AutoscrollBehavior::Always
                    && matches!(
                        self.scroll_position,
                        ScrollPosition::FixedAtPosition { .. }
                            | ScrollPosition::FixedWithinLongRunningBlock { .. }
                    )
                {
                    return self.scroll_position;
                }

                self.scroll_position_after_command_execution(app)
            }
            ScrollPositionUpdate::AfterKeydownOnTerminal
            | ScrollPositionUpdate::AfterTypedCharacters
            | ScrollPositionUpdate::AfterWriteUserBytesToPty => {
                // keydown actually has the same semantics for adjusting scroll position
                // as executing a command.
                self.scroll_position_after_command_execution(app)
            }
            ScrollPositionUpdate::AfterScrollEvent { scroll_delta } => {
                self.scroll_position_for_delta(scroll_delta)
            }
            ScrollPositionUpdate::AfterResize => {
                let max_scroll_top = self.max_scroll_top_in_lines();

                // When resizing, the number of rows might "shrink" as the wrapped-around lines
                // are rendered in one line. This changes the value of maximum scroll top and could
                // make the previous scroll position invalid. Thus we add an additional check here
                // to change the scroll position to stick to the bottom if previous scroll top is invalid.
                if let ScrollPosition::FixedAtPosition { scroll_lines } = self.scroll_position {
                    if scroll_lines.scroll_top(self.block_list, self.content_element_height_lines())
                        > max_scroll_top
                    {
                        return ScrollPosition::FollowsBottomOfMostRecentBlock;
                    }
                }
                self.scroll_position
            }
            ScrollPositionUpdate::AfterClear => self.scroll_position_after_clear(),
            ScrollPositionUpdate::ScrollToTopOfBlock { block_index } => {
                self.scroll_position_at_top_of_block(block_index)
            }
            ScrollPositionUpdate::ScrollToBottomOfBlock { block_index } => {
                self.scroll_position_at_bottom_of_block(block_index)
            }
            ScrollPositionUpdate::ScrollMostRecentBlockIntoView => {
                if self.input_mode.is_inverted_blocklist() {
                    ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(Lines::zero()),
                    }
                } else if let Some(block_index) = self.block_list.last_non_hidden_block_by_index() {
                    self.scroll_position_at_bottom_of_block(block_index)
                } else {
                    self.scroll_position
                }
            }
            ScrollPositionUpdate::AfterPageUp => {
                let new_scroll_top = (self.scroll_top_in_lines()
                    - self.content_element_height_lines()
                    + 1.0.into_lines())
                .max(Lines::zero());
                ScrollPosition::FixedAtPosition {
                    scroll_lines: self.scroll_lines_from_scroll_top(new_scroll_top),
                }
            }
            ScrollPositionUpdate::AfterPageDown => {
                let total_block_heights = self.block_list.block_heights().summary().height;
                let visible_rows = self.content_element_height_lines();
                let current_position = self.scroll_top_in_lines();
                if current_position + visible_rows - 1.0.into_lines()
                    > (total_block_heights - visible_rows).max(Lines::zero())
                {
                    ScrollPosition::FollowsBottomOfMostRecentBlock
                } else {
                    let new_scroll_top = current_position + visible_rows - 1.0.into_lines();
                    ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(new_scroll_top),
                    }
                }
            }
            ScrollPositionUpdate::AfterHome => ScrollPosition::FixedAtPosition {
                scroll_lines: self.scroll_lines_from_scroll_top(Lines::zero()),
            },
            ScrollPositionUpdate::AfterEnd | ScrollPositionUpdate::AfterEnterAgentView => {
                if matches!(
                    self.input_mode,
                    InputMode::PinnedToBottom | InputMode::Waterfall
                ) {
                    ScrollPosition::FollowsBottomOfMostRecentBlock
                } else {
                    ScrollPosition::FixedAtPosition {
                        scroll_lines: self
                            .scroll_lines_from_scroll_top(self.max_scroll_top_in_lines()),
                    }
                }
            }
            ScrollPositionUpdate::ScrollToBlocklistRowIfNotVisible { row } => {
                self.scroll_to_blocklist_row_if_not_visible(row)
            }
            ScrollPositionUpdate::ScrollToFindMatchIfNotVisible(find_match_position) => {
                self.scroll_to_match_if_not_visible(find_match_position)
            }
            ScrollPositionUpdate::AfterFilter {
                block_index,
                prev_top_of_viewport,
                prev_bottom_of_block,
                prev_first_visible_original_row,
            } => self.adjust_scroll_after_filter(
                block_index,
                prev_top_of_viewport,
                prev_bottom_of_block,
                prev_first_visible_original_row,
            ),
            ScrollPositionUpdate::AfterFilterClear {
                block_index,
                offset_from_block_top,
            } => self.adjust_scroll_after_filter_clear(block_index, offset_from_block_top),
            ScrollPositionUpdate::ScrollToTopOfBlockWithBuffer {
                block_index,
                buffer_lines,
            } => self.scroll_position_at_top_of_block_with_buffer(block_index, buffer_lines),
            ScrollPositionUpdate::ScrollToTopOfRichContent { index } => {
                let (top, _) = self.rich_content_scroll_bounds(index);
                let scroll_top = top.max(Lines::zero()).min(self.max_scroll_top_in_lines());
                ScrollPosition::FixedAtPosition {
                    scroll_lines: self.scroll_lines_from_scroll_top(scroll_top),
                }
            }
            ScrollPositionUpdate::AfterExitAgentView { saved_position } => saved_position,
        }
    }

    fn adjust_scroll_after_filter_clear(
        &self,
        block_index: BlockIndex,
        offset_from_block_top: Lines,
    ) -> ScrollPosition {
        if matches!(
            self.scroll_position,
            ScrollPosition::FollowsBottomOfMostRecentBlock
                | ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock { .. }
        ) {
            return self.scroll_position;
        }

        let block_top = self.top_of_block_in_lines(block_index);
        let new_scroll_top = block_top + offset_from_block_top;
        ScrollPosition::FixedAtPosition {
            scroll_lines: self.scroll_lines_from_scroll_top(new_scroll_top),
        }
    }

    fn anchor_block_bottom_after_filtering(
        &self,
        block_index: BlockIndex,
        prev_top_of_viewport: Lines,
        prev_bottom_of_block: Lines,
    ) -> ScrollPosition {
        let curr_bottom_of_block = self.bottom_of_block_in_lines(block_index);
        let height_delta = prev_bottom_of_block - curr_bottom_of_block;
        let new_scroll_top = prev_top_of_viewport - height_delta;
        ScrollPosition::FixedAtPosition {
            scroll_lines: self.scroll_lines_from_scroll_top(new_scroll_top),
        }
    }

    /// Attempt to anchor to the top row in the viewport before the filter was
    /// applied. The algorithm is as follows:
    ///   1. If the top row in the viewport is not filtered out, scroll so it
    ///      remains the top row in the viewport.
    ///   2. If the top row in the viewport was filtered out, look for the next
    ///      closest row and scroll so that becomes the top row in the viewport.
    ///      e.g. Suppose the user is looking at:
    ///      ---  top of viewport   ---
    ///      | row 4                  |
    ///      | row 5                  |
    ///      | row 6                  |
    ///      --- bottom of viewport ---
    ///      If rows 4 and 5 get filtered out, then row 6 is the next closest
    ///      row and so we scroll so that becomes the top row in the viewport.
    ///      ---  top of viewport   ---
    ///      | row 6                  |
    ///      | ...                    |
    ///      | ...                    |
    ///      --- bottom of viewport ---
    fn anchor_top_row_after_filtering(
        &self,
        block: &Block,
        top_of_block: Lines,
        new_first_displayed_row: usize,
    ) -> ScrollPosition {
        let header_height = self.snackbar_header_height();
        let new_scroll_top = top_of_block
            + block.output_grid_offset().into_lines()
            + (new_first_displayed_row as f32).into_lines()
            - header_height;
        ScrollPosition::FixedAtPosition {
            scroll_lines: self.scroll_lines_from_scroll_top(new_scroll_top),
        }
    }

    fn adjust_scroll_after_filter(
        &self,
        block_index: BlockIndex,
        prev_top_of_viewport: Lines,
        prev_bottom_of_block: Lines,
        prev_first_visible_original_row: Option<usize>,
    ) -> ScrollPosition {
        if matches!(
            self.scroll_position,
            ScrollPosition::FollowsBottomOfMostRecentBlock
                | ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock { .. }
        ) {
            return self.scroll_position;
        }

        let Some(block) = self.block_list.block_at(block_index) else {
            log::warn!("Could not find block when adjusting scroll after filter");
            return self.scroll_position;
        };

        // The top of block is always in the same position before and after filtering.
        let top_of_block = self.top_of_block_in_lines(block_index);
        let top_of_output_grid = top_of_block + block.output_grid_offset().into_lines();
        let prev_bottom_of_viewport = prev_top_of_viewport + self.content_element_height_lines();

        // Scroll to the top/bottom of the block if it was not in the viewport at all.
        let was_viewport_before_block = heights_approx_lte(prev_bottom_of_viewport, top_of_block);
        if was_viewport_before_block {
            return self.scroll_position_at_top_of_block(block_index);
        }
        let was_viewport_after_block = heights_approx_lte(
            // Adjust so that the bottom padding counts as "after" the block. The
            // end result here is if the top of the viewport is in the padding,
            // we will scroll to the bottom of the block.
            prev_bottom_of_block - block.padding_bottom().into_lines(),
            prev_top_of_viewport,
        );
        if was_viewport_after_block {
            return self.scroll_position_at_bottom_of_block(block_index);
        }

        let showing_snackbar = self.snackbar_header_state().header_position.is_some();
        // If the prompt or command are visible (not via the snackbar), the
        // top of the block will be anchored.
        let was_top_of_block_visible = height_in_range_approx(
            if showing_snackbar {
                top_of_block
            } else {
                top_of_output_grid
            },
            prev_top_of_viewport,
            prev_bottom_of_viewport,
        );
        let was_bottom_of_block_visible = height_in_range_approx(
            prev_bottom_of_block,
            prev_top_of_viewport,
            prev_bottom_of_viewport,
        );

        if was_top_of_block_visible {
            // Keep the top of the block in the same position. This is a no-op
            // when the input mode is not `PinnedToTop`.
            ScrollPosition::FixedAtPosition {
                scroll_lines: self.scroll_lines_from_scroll_top(prev_top_of_viewport),
            }
        } else {
            // The top of the viewport was somewhere in the output grid pre-filter.
            let Some(prev_first_visible_original_row) = prev_first_visible_original_row else {
                log::warn!("No previous row in viewport found from before filtering");
                return self.scroll_position;
            };

            if let Some(new_first_displayed_row) = block
                .output_grid()
                .grid_handler()
                .get_exact_or_next_displayed_row(prev_first_visible_original_row)
            {
                self.anchor_top_row_after_filtering(block, top_of_block, new_first_displayed_row)
            } else if was_bottom_of_block_visible {
                self.anchor_block_bottom_after_filtering(
                    block_index,
                    prev_top_of_viewport,
                    prev_bottom_of_block,
                )
            } else {
                self.scroll_position_at_bottom_of_block(block_index)
            }
        }
    }

    /// Returns the optimal scroll position for the given `find_match_location`.
    fn scroll_to_match_if_not_visible(
        &self,
        find_match_location: FindMatchScrollLocation,
    ) -> ScrollPosition {
        let current_scroll_top = self.scroll_top_in_lines();
        let viewport_height = self.content_element_height_lines();
        let current_scroll_bottom = current_scroll_top + viewport_height;

        let mut new_scroll_top = match find_match_location {
            FindMatchScrollLocation::Block {
                block_index,
                section,
            } => {
                let block_section_offset = self
                    .block_list
                    .block_at(block_index)
                    .map(|block| block.block_section_offset_from_top(section))
                    .unwrap_or(Lines::zero());
                let match_position =
                    self.top_of_block_in_lines(block_index) + block_section_offset.into_lines();
                let buffer = {
                    // This is used as a row offset from the top/bottom of the view in order to ensure that
                    // the match is fully visible, not hidden by the find bar or snackbar.
                    const SCROLL_OFFSET: Lines = Lines::new(5.0);

                    if SCROLL_OFFSET < viewport_height {
                        SCROLL_OFFSET
                    } else {
                        Lines::zero()
                    }
                };

                // If the match position is above the viewport, scroll up until the match
                // is at the bottom of the viewport (with some buffer from the bottom).
                if match_position < current_scroll_top + buffer {
                    match_position - viewport_height + buffer
                } else if match_position > current_scroll_bottom - buffer {
                    // Otherwise, if the match position is below the viewport, scroll down until the
                    // match is at the top of the viewport (with some buffer from the top).
                    // We calculate the bottom of the viewport by adding the height of the viewport to
                    // the current top scroll position, subtracting a buffer to ensure the match is fully visible.
                    match_position - buffer
                } else {
                    // The match is in view so just set the current scroll position to unmovable.
                    // A new finished block will set scroll_position to FixedToBottom, scrolling away from the focused match.
                    // An active, running block doesn't change scroll_position, so the focused match will always remain in view
                    // if it is in an active, running block.
                    return ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(current_scroll_top),
                    };
                }
            }
            FindMatchScrollLocation::RichContent { index } => {
                // Scrolls to the rich content block containing the match, but does not yet support
                // scrolling to the exact match location since rich content UI is not laid out along
                // sum tree `Line`s.
                //
                // As a result, we determine a scroll position relative to the top or bottom of the rich
                // content block depending on which direction we need to scroll to make the match visible.
                let (rich_content_top, rich_content_bottom) =
                    self.rich_content_scroll_bounds(index);

                if (rich_content_top >= current_scroll_top
                    && rich_content_bottom <= current_scroll_bottom)
                    || (rich_content_top <= current_scroll_top
                        && rich_content_bottom >= current_scroll_bottom)
                {
                    // If the AI block is either completely in view or is larger than the viewport
                    // and spans the entire viewport, fix the scroll position.
                    return ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(current_scroll_top),
                    };
                } else if rich_content_top < current_scroll_top {
                    // If the AI block is above the viewport, scroll such that the block is at the
                    // bottom of the viewport.
                    rich_content_bottom - viewport_height
                } else {
                    // Else, the block must be below the viewport, scroll such that the block is at
                    // the top of the viewport.
                    rich_content_top
                }
            }
        };

        new_scroll_top = new_scroll_top
            .max(Lines::zero())
            .min(self.max_scroll_top_in_lines());

        ScrollPosition::FixedAtPosition {
            scroll_lines: self.scroll_lines_from_scroll_top(new_scroll_top),
        }
    }

    fn snackbar_header_height(&self) -> Lines {
        let row_height = self.size_info.cell_height_px();
        self.snackbar_header_state()
            .header_position
            .map_or(0., |header| header.rect.height() / row_height.as_f32())
            .into_lines()
    }

    fn scroll_to_blocklist_row_if_not_visible(&self, row: Lines) -> ScrollPosition {
        let row = self.block_list_y_to_scroll_y(row);

        let total_block_heights = self.block_list.block_heights().summary().height;
        let visible_rows = self.content_element_height_lines();

        let max_scroll_position = (total_block_heights - visible_rows).max(Lines::zero());
        let current_scroll_position = self.scroll_top_in_lines();

        let header_height = self.snackbar_header_height();

        // The visible screen in which there can be a text selection is bounded as such:
        //
        //  -----current_scroll_position + header_height -----------
        // |                                                       |
        // |                                                       |
        // |                                                       |
        // |                                                       |
        //  -------(current_scroll_position + visible_rows)--------
        //
        // If the point lies above the top boundary, we scroll upwards to reveal the line, making adjustments
        // based on the snackbar of the block where the new point lies. If the point lies below the boundary
        // boundary (or in a partial bottom row), we scroll such that the desired row is fully revealed as the
        // bottommost visible row. If the point lies in the boundaries, there's no scroll adjustment.
        let new_scroll_position = if row < current_scroll_position + header_height {
            // Scrolling up: place the row at the very top of the viewport
            row - header_height
        } else if row > current_scroll_position + (visible_rows - 1.0.into_lines()) {
            // Scrolling down: place the row as the last visible row in the viewport
            row - (visible_rows - 1.0.into_lines())
        } else {
            // If the row lies in the viewport, no adjustments are needed
            current_scroll_position
        };

        let new_top = new_scroll_position
            .max(Lines::zero())
            .min(max_scroll_position);
        ScrollPosition::FixedAtPosition {
            scroll_lines: self.scroll_lines_from_scroll_top(new_top),
        }
    }

    fn scroll_position_after_clear(&self) -> ScrollPosition {
        match self.input_mode {
            InputMode::PinnedToTop | InputMode::PinnedToBottom => {
                ScrollPosition::FollowsBottomOfMostRecentBlock
            }
            InputMode::Waterfall => ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock {
                scroll_top_in_lines: self.max_scroll_top_in_lines(),
            },
        }
    }

    // Returns whether the input is rendered exactly at the bottom of its pane.
    fn is_input_rendered_at_bottom_of_pane(&self, app: &AppContext) -> bool {
        match self.input_mode {
            InputMode::Waterfall => {
                let current_scroll_top_px = self.scroll_top_in_pixels();
                let total_block_height_px = self
                    .block_list
                    .block_heights()
                    .summary()
                    .height
                    .to_pixels(self.size_info.cell_height_px());
                let (gap_height_px, blocklist_inset_px) =
                    if let Some(gap) = self.block_list.active_gap() {
                        (
                            Some(gap.height().to_pixels(self.size_info.cell_height_px())),
                            // See doc comment on `blocklist_top_inset_when_in_waterfall_mode`
                            // for context.
                            self.inline_menu_positioner
                                .as_ref(app)
                                .blocklist_top_inset_when_in_waterfall_mode(app),
                        )
                    } else {
                        (None, None)
                    };
                let total_block_height_without_gap_px =
                    total_block_height_px - gap_height_px.unwrap_or_default();
                let visible_block_height_px = total_block_height_without_gap_px
                    - current_scroll_top_px
                    - blocklist_inset_px.unwrap_or_default();
                let input_height_px = Pixels::new(self.input_size.y());
                let max_blocklist_element_height =
                    self.size_info.pane_height_px() - input_height_px;
                visible_block_height_px >= max_blocklist_element_height
            }
            InputMode::PinnedToBottom => true,
            InputMode::PinnedToTop => false,
        }
    }

    // Returns the scroll position to set after a command has finished executing
    fn scroll_position_after_command_execution(&self, app: &AppContext) -> ScrollPosition {
        match (self.input_mode, self.block_list.active_gap()) {
            (InputMode::Waterfall, Some(gap)) => {
                // In gap waterfall mode, the logic is somewhat complex for how to adjust scroll position
                // after a command is executed.  The basic result we want is:
                //  1. If the gap is scrolled into view don't change the scroll position and just add the command
                //     above the input box while moving the input box down.  Because the gap is shrinking by the size
                //     of the new command you don't actually need to change the scroll position in this case.
                //  2. If the gap is scrolled below the input (so that the input appears sticky at the bottom) then
                //     we need to scroll so that the most recent command is just above the input and the input stays
                //     at the bottom, possibly leaving whatever is left of the gap below it.  This is what the math
                //     below is figuring out.
                let total_block_height_px = self
                    .block_list
                    .block_heights()
                    .summary()
                    .height
                    .to_pixels(self.size_info.cell_height_px());
                let gap_height_px = gap.height().to_pixels(self.size_info.cell_height_px());
                let block_heights_without_gap_px = total_block_height_px - gap_height_px;
                if self.is_input_rendered_at_bottom_of_pane(app) {
                    let new_position_in_lines = (block_heights_without_gap_px
                        - Pixels::new(self.blocklist_element_size.y()))
                    .to_lines(self.size_info.cell_height_px());
                    ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock {
                        scroll_top_in_lines: new_position_in_lines
                            .max(Lines::zero())
                            .min(self.max_scroll_top_in_lines()),
                    }
                } else {
                    ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock {
                        scroll_top_in_lines: self.scroll_top_in_lines(),
                    }
                }
            }
            (_, _) => ScrollPosition::FollowsBottomOfMostRecentBlock,
        }
    }

    /// Calculates the next scroll position for the given viewport state and scroll delta.
    fn scroll_position_for_delta(&self, delta: Lines) -> ScrollPosition {
        let max_scroll_top = self.max_scroll_top_in_lines();
        let current_top = self.scroll_top_in_lines();

        let new_top = (current_top - delta).max(Lines::zero()).min(max_scroll_top);
        let fix_to_bottom = new_top >= max_scroll_top
            && matches!(
                self.input_mode,
                InputMode::PinnedToBottom | InputMode::Waterfall
            );

        if fix_to_bottom {
            ScrollPosition::FollowsBottomOfMostRecentBlock
        } else if self.block_list.active_block().is_active_and_long_running()
            && self.does_block_exceed_viewport(self.block_list.active_block_index(), new_top)
        {
            // We only want to account for truncation if the active block is fully spanning
            // the viewport. If there are other block items in the viewport, we don't want
            // truncation to affect the scroll position because the user might want to have
            // their scroll position fixed between different blocks.
            ScrollPosition::FixedWithinLongRunningBlock {
                scroll_lines: self.scroll_lines_from_scroll_top(new_top),
                num_output_lines_truncated: self
                    .block_list
                    .active_block()
                    .output_grid()
                    .grid_handler()
                    .num_lines_truncated(),
            }
        } else {
            ScrollPosition::FixedAtPosition {
                scroll_lines: self.scroll_lines_from_scroll_top(new_top),
            }
        }
    }

    /// Returns whether applying the given scroll delta would move the block list.
    pub fn can_scroll_by_delta(&self, delta: Lines) -> bool {
        let current_top = self.scroll_top_in_lines();
        let new_top = (current_top - delta)
            .max(Lines::zero())
            .min(self.max_scroll_top_in_lines());

        new_top != current_top
    }

    /// Returns the block index at the given block list point, or None if
    /// there is no block at that point in the blocklist.
    pub fn block_index_from_point(&self, point: BlockListPoint) -> Option<BlockIndex> {
        let mut block_heights_cursor = self
            .block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        block_heights_cursor.seek(&BlockHeight::from(point.row), SeekBias::Right);

        block_heights_cursor.item().and_then(|item| match item {
            BlockHeightItem::Block(_) => Some(block_heights_cursor.start().block_count.into()),
            BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent { .. } => None,
        })
    }

    pub fn block_height_item_from_point(&self, point: BlockListPoint) -> Option<&BlockHeightItem> {
        let mut block_heights_cursor = self
            .block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        block_heights_cursor.seek(&BlockHeight::from(point.row), SeekBias::Right);

        block_heights_cursor.item()
    }

    /// Returns whether the last non-hidden block is visible according to the given block visibility mode
    /// Note that "in view" here means that the block is scrolled at least partially into view
    /// and has a non-zero height.
    pub fn is_most_recent_block_in_view(&self, block_visibility_mode: BlockVisibilityMode) -> bool {
        self.block_list
            .last_non_hidden_block_by_index()
            .is_some_and(|block_index| self.is_block_in_view(block_index, block_visibility_mode))
    }

    /// Returns whether a block is visible according to the given block visibility mode
    /// Note that "in view" here means that the block is scrolled at least partially into view
    /// and has a non-zero height.
    pub fn is_block_in_view(
        &self,
        block_index: BlockIndex,
        block_visibility: BlockVisibilityMode,
    ) -> bool {
        let top_of_viewport_in_lines = self.scroll_top_in_lines();
        let bottom_of_viewport_in_lines =
            top_of_viewport_in_lines + self.content_element_height_lines();
        match block_visibility {
            BlockVisibilityMode::TopOfBlockVisible => {
                let top_of_block_in_lines = self.top_of_block_in_lines(block_index);
                heights_approx_gte(top_of_block_in_lines, top_of_viewport_in_lines)
                    && heights_approx_lte(top_of_block_in_lines, bottom_of_viewport_in_lines)
            }
            BlockVisibilityMode::BottomOfBlockVisible => {
                let bottom_of_block_in_lines = self.bottom_of_block_in_lines(block_index);
                heights_approx_gte(bottom_of_block_in_lines, top_of_viewport_in_lines)
                    && heights_approx_lte(bottom_of_block_in_lines, bottom_of_viewport_in_lines)
            }
        }
    }

    /// Returns true iff the block at `block_index` starts before the viewport
    /// (starting at `new_scroll_top`) and ends after it.
    fn does_block_exceed_viewport(&self, block_index: BlockIndex, new_scroll_top: Lines) -> bool {
        let top_of_viewport_in_lines = new_scroll_top;
        let bottom_of_viewport_in_lines =
            top_of_viewport_in_lines + self.content_element_height_lines();
        let top_of_block_in_lines = self.top_of_block_in_lines(block_index);
        let bottom_of_block_in_lines = self.bottom_of_block_in_lines(block_index);

        heights_approx_lt(top_of_block_in_lines, top_of_viewport_in_lines)
            && heights_approx_gt(bottom_of_block_in_lines, bottom_of_viewport_in_lines)
    }

    pub fn max_scroll_top_px(&self) -> Pixels {
        self.max_scroll_top_in_lines()
            .to_pixels(self.size_info.cell_height_px)
    }

    /// Returns the max possible value in lines for scroll_top (how far from the top of the
    /// blocklist it's possible to scroll down)
    pub fn max_scroll_top_in_lines(&self) -> Lines {
        match (self.input_mode, self.block_list.active_gap()) {
            (InputMode::Waterfall, Some(gap)) => {
                // In waterfall mode with a gap, the max scroll top is always right
                // at the top of the gap.
                //
                // This diagram of the positions in the blocklist model (not view) shows why.
                //
                // | Block list up to gap  |
                // | -- top of viewport -- | <-- max_scroll_top
                // | ------- GAP --------- |
                // | commands after gap -- |
                // | ------ input -------- |
                // | bottom of viewport    |
                //
                // The invariant is that the GAP + the height of the blocks after the gap
                // always equals the height of the viewport.
                //
                // Note that in waterfall mode, the actual layout in the view looks like this:
                //
                // | Block list up to gap  |
                // | -- top of viewport -- | <-- max_scroll_top
                // | commands after gap -- |
                // | ------ input -------- |
                // | ------- GAP --------- |
                // | bottom of viewport    |
                //
                // but the re-ordering of the items in the view doesn't change max scroll top.
                let mut cursor = self
                    .block_list
                    .block_heights()
                    .cursor::<TotalIndex, BlockHeightSummary>();
                cursor.seek(&gap.index(), SeekBias::Right);
                cursor.start().height
            }
            (_, _) => {
                let total_block_height = self.block_list.block_heights().summary().height;
                (total_block_height - self.content_element_height_lines()).max(Lines::zero())
            }
        }
    }

    /// Returns the number of lines the top of this block is offset from zero
    /// scroll top.
    pub fn top_of_block_in_lines(&self, block_index: BlockIndex) -> Lines {
        let mut cursor = self
            .block_list
            .block_heights()
            .cursor::<BlockIndex, BlockHeightSummary>();
        cursor.seek(&block_index, SeekBias::Right);

        let block_list_height = self.block_list.block_heights().summary().height;
        let num_visible_lines = self.content_element_height_lines();
        match (self.input_mode, self.block_list.active_gap()) {
            (InputMode::PinnedToBottom, _) | (InputMode::Waterfall, None) => {
                let mut top = cursor.start().height;
                // Adjust the top for the case where the blocks don't fill the viewport
                if block_list_height < num_visible_lines {
                    top += num_visible_lines - block_list_height;
                }
                top.max(Lines::zero())
            }
            (InputMode::PinnedToTop, _) => {
                (block_list_height - cursor.end().height).max(Lines::zero())
            }
            (InputMode::Waterfall, Some(gap)) => {
                // If there is a gap in waterfall mode, we need to take into account whether
                // the block is above or below the gap.
                let top = if cursor.start().total_count < gap.index().0 {
                    cursor.start().height
                } else {
                    cursor.start().height - gap.height()
                };
                top.max(Lines::zero())
            }
        }
    }

    /// Returns the (top, bottom) scroll bounds in lines for the rich content at the given TotalIndex.
    fn rich_content_scroll_bounds(&self, index: TotalIndex) -> (Lines, Lines) {
        let mut cursor = self
            .block_list
            .block_heights()
            .cursor::<TotalIndex, BlockHeightSummary>();
        cursor.seek(&index, SeekBias::Right);

        if let InputMode::PinnedToTop = self.input_mode {
            // With `PinnedToTop`, we have to "invert" the scroll position calculation
            // since the block list is inverted.
            let block_list_height = self.block_list.block_heights().summary().height;
            (
                block_list_height - cursor.end().height,
                block_list_height - cursor.start().height,
            )
        } else {
            (cursor.start().height, cursor.end().height)
        }
    }

    /// Returns the scroll position for navigating the top of the block at the
    /// given index into view
    fn scroll_position_at_top_of_block(&self, block_index: BlockIndex) -> ScrollPosition {
        self.scroll_position_at_top_of_block_with_buffer(block_index, Lines::zero())
    }

    /// Returns the scroll position for navigating the top of the block at the
    /// given index into view, with a buffer of space above the top of the block.
    fn scroll_position_at_top_of_block_with_buffer(
        &self,
        block_index: BlockIndex,
        buffer_lines: Lines,
    ) -> ScrollPosition {
        let top_of_block_with_buffer =
            (self.top_of_block_in_lines(block_index) - buffer_lines).max(Lines::zero());
        match self.input_mode {
            InputMode::PinnedToBottom | InputMode::Waterfall => {
                if top_of_block_with_buffer >= self.max_scroll_top_in_lines() {
                    ScrollPosition::FollowsBottomOfMostRecentBlock
                } else {
                    ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(top_of_block_with_buffer),
                    }
                }
            }
            InputMode::PinnedToTop => ScrollPosition::FixedAtPosition {
                scroll_lines: self.scroll_lines_from_scroll_top(
                    top_of_block_with_buffer.min(self.max_scroll_top_in_lines()),
                ),
            },
        }
    }

    /// Returns the number of lines the bottom of this block is offset from zero
    /// scroll top.
    pub fn bottom_of_block_in_lines(&self, block_index: BlockIndex) -> Lines {
        let top_of_block = self.top_of_block_in_lines(block_index);
        let block_height = self
            .block_list
            .block_at(block_index)
            .map_or(Lines::zero(), |b| {
                b.height(self.block_list.agent_view_state())
            });
        top_of_block + block_height
    }

    /// Returns the scroll position for navigating the bottom of the block at the
    /// given index into view
    pub fn scroll_position_at_bottom_of_block(&self, block_index: BlockIndex) -> ScrollPosition {
        let bottom_of_block = self.bottom_of_block_in_lines(block_index);
        let scroll_top = (bottom_of_block - self.content_element_height_lines())
            .max(Lines::zero())
            .min(self.max_scroll_top_in_lines());

        // Check if this command block is the most recent block. This means there is no command or rich content block that is after
        // the block at given index.
        let is_most_recent_visible_block = self
            .block_list
            .last_non_hidden_block_by_index()
            .is_some_and(|index| {
                index == block_index
                    && self
                        .block_list
                        .last_non_hidden_rich_content_block_after_block(Some(index))
                        .is_none()
            });
        match self.input_mode {
            InputMode::PinnedToBottom | InputMode::PinnedToTop => {
                if is_most_recent_visible_block {
                    ScrollPosition::FollowsBottomOfMostRecentBlock
                } else {
                    ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(scroll_top),
                    }
                }
            }
            InputMode::Waterfall => {
                if is_most_recent_visible_block {
                    if self.block_list.active_gap().is_some() {
                        ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock {
                            scroll_top_in_lines: scroll_top,
                        }
                    } else {
                        ScrollPosition::FollowsBottomOfMostRecentBlock
                    }
                } else {
                    ScrollPosition::FixedAtPosition {
                        scroll_lines: self.scroll_lines_from_scroll_top(scroll_top),
                    }
                }
            }
        }
    }

    /// Returns whether we are in waterfall mode with an active gap.
    pub fn is_waterfall_gap_mode(&self) -> bool {
        matches!(self.input_mode, InputMode::Waterfall) && self.block_list.active_gap().is_some()
    }

    /// Returns the scroll position of any "overhanging" bottom block - this
    /// is a block whose bottom part is scrolled off the bottom of the screen
    pub fn overhanging_bottom_block(&self, app: &AppContext) -> Option<OverhangingBlock> {
        if matches!(self.input_mode, InputMode::Waterfall)
            && !self.is_input_rendered_at_bottom_of_pane(app)
        {
            // For waterfall mode the input needs to be at the bottom of the pane
            // for there to be an overhang.
            return None;
        }
        let bottom_of_view = self.scroll_top_in_lines() + self.content_element_height_lines();
        let last_block = self.iter_from(bottom_of_view).next()?;
        let block_index = last_block.block_index?;
        let top_of_block = self.top_of_block_in_lines(block_index);
        let bottom_of_block = self.bottom_of_block_in_lines(block_index);
        let is_most_recent_block =
            self.block_list.last_non_hidden_block_by_index() == Some(block_index);
        // A block is ovehanging when top_of_block < bottom_of_view < bottom_of_block
        (bottom_of_block - bottom_of_view > HEIGHT_FUDGE_FACTOR_LINES).then_some(OverhangingBlock {
            visible_block_height_px: (bottom_of_view - top_of_block)
                .max((0.).into_lines())
                .to_pixels(self.size_info.cell_height_px()),
            block_index,
            is_most_recent_block,
        })
    }

    /// Converts a pixel coordinate to a point in the `BlockList` coordinate space
    pub fn screen_coord_to_blocklist_point(
        &self,
        viewport_origin: Vector2F,
        snackbar_point: SnackbarPoint,
        clamping_mode: ClampingMode,
    ) -> Option<BlockListPoint> {
        let relative_coord = snackbar_point.coord - viewport_origin;
        let total_block_height = self.block_list.block_heights().summary().height;
        let size = self.size_info;
        let content_element_lines = self.content_element_height_lines();
        let mut coord_in_lines = relative_coord
            .y()
            .into_pixels()
            .to_lines(size.cell_height_px());

        let (is_coord_above_blocks, is_coord_below_blocks) = match self.input_mode {
            InputMode::PinnedToBottom => {
                let screen_taller_than_blocks = content_element_lines > total_block_height;
                let coord_above_bottom_blocks =
                    coord_in_lines < content_element_lines - total_block_height;
                (
                    screen_taller_than_blocks && coord_above_bottom_blocks,
                    relative_coord.y() > self.blocklist_element_size.y(),
                )
            }
            InputMode::Waterfall => (
                relative_coord.y() < 0.,
                relative_coord.y() > self.blocklist_element_size.y(),
            ),
            InputMode::PinnedToTop => {
                let screen_taller_than_blocks = content_element_lines > total_block_height;
                let coord_below_top_blocks = coord_in_lines > total_block_height;
                (
                    relative_coord.y() < 0.,
                    screen_taller_than_blocks && coord_below_top_blocks,
                )
            }
        };

        match clamping_mode {
            ClampingMode::ClampToGridIfWithinBlock => {
                if total_block_height == Lines::zero()
                    || relative_coord.x() < 0.
                    || relative_coord.x()
                        - self.horizontal_clipped_scroll_state.scroll_start().as_f32()
                        > self.size_info.pane_width_px().as_f32()
                    || is_coord_above_blocks
                    || is_coord_below_blocks
                {
                    return None;
                }
            }
            ClampingMode::ReturnNoneIfNotInGrid => {
                let min_x = size.padding_x_px;
                let max_x =
                    size.padding_x_px + ((size.columns as f32).into_pixels() * size.cell_width_px);

                if total_block_height == Lines::zero()
                    || is_coord_above_blocks
                    || is_coord_below_blocks
                    || relative_coord.x() < min_x.as_f32()
                    || relative_coord.x() > max_x.as_f32()
                {
                    return None;
                }
            }
            ClampingMode::ClampToGrid => {
                if total_block_height == Lines::zero() {
                    return None;
                }
            }
        }

        // Translate the coordinate position upwards if it's contained inside
        // the snackbar header.
        coord_in_lines -= self
            .snackbar_header_state()
            .header_translation_for_coord(size, snackbar_point);

        let scroll_top = self.scroll_top_in_lines();
        let offset = (content_element_lines - total_block_height).max(Lines::zero());
        let column = ((relative_coord.x() - size.padding_x_px.as_f32()).max(0.)
            / size.cell_width_px().as_f32()) as usize;
        let row = match self.input_mode {
            InputMode::PinnedToBottom => coord_in_lines + scroll_top - offset,
            InputMode::Waterfall => {
                match self.block_list.active_gap() {
                    Some(gap) => {
                        let mut row = coord_in_lines + scroll_top;
                        let mut cursor = self
                            .block_list
                            .block_heights()
                            .cursor::<BlockHeight, BlockHeightSummary>();
                        cursor.seek(&BlockHeight::from(row), SeekBias::Right);

                        // In waterfall gap mode if the row is after the gap, we need
                        // to adjust the row to account for the gap because we are moving from
                        // screen to model cooredinates here.
                        if cursor.start().total_count >= gap.index().0 {
                            row += gap.height();
                        }
                        row
                    }
                    None => {
                        // With no gap the logic is the same as pinned to bottom, except
                        // the offset doesn't apply because you can never have any space
                        // between the top of the blocks and the top of the screen
                        coord_in_lines + scroll_top
                    }
                }
            }
            InputMode::PinnedToTop => {
                // The logic here is somewhat tricky. In inverted mode
                // the blocklist has the most recent blocks at the top, so we
                // generally can translate a screen coord by subtracting its
                // current row from the total block height.  This will correctly
                // find the block at the given row, but will actually find the wrong line
                // within it because within a single block, the direction is always
                // top to bottom.
                //
                // To illustrate, assume you had a blocklist with 3 blocks,
                // rendered in terms of recency order, from 3, the most recent, to 1,
                // the least recent, with no scroll.  Assume the screen coord is
                // at line 3 in the second block.
                //
                // | Block 3 = 5 lines long|
                // | Block 2 = 10 lines long| <-- screen coord at line 3 of block 2
                // | Block 1 = 15 lines long|
                //
                // In this case the total_block_height = 30, and coord_in_lines = 8 (5 lines
                // from block 3 and 3 lines from block 2).
                // This implies that total_block_height - coord_in_lines = 22.
                //
                // If we treated 22 as the BlockListPoint, then we would actually be pointing
                // at row 7 in Block 2 (because 22 - 15 = 7).  That's because the blocklist
                // heights in the model are stored like this:
                //
                // | Block 1 = 15 lines long|
                // | Block 2 = 10 lines long|
                // | Block 3 = 5 lines long|
                //
                // So instead, we need to calculate how many lines into Block 2 the screen coord
                // is, and add that directly to the total height before Block 2 in the blocklist.
                // This yields 3 + 15 = 18, which is the correct BlockListPoint.

                // We use the inverted_row to identify the block the coord is in.
                let inverted_row =
                    (total_block_height - (coord_in_lines + scroll_top)).max(Lines::zero());
                let mut cursor = self
                    .block_list
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                cursor.seek(&BlockHeight::from(inverted_row), SeekBias::Right);

                // The "overhang" is how many rows past the start of the block the row is
                // when looking at the block list non-inverted.
                let overhang = inverted_row - cursor.start().height;
                cursor.item().map_or(inverted_row, |item| {
                    // We need to actually subtract the overhang from the block height
                    // to get how many rows past the start of the block we are at,
                    // and then add that to the height.
                    let rows_past_block_start = item.height().into_lines() - overhang;
                    cursor.start().height + rows_past_block_start
                })
            }
        };

        // Use saturating subtraction to compute the max valid row index, ensuring it
        // doesn't go negative when total_block_height is less than 1 line.
        let max_row = (total_block_height - 1.0.into_lines()).max(Lines::zero());
        Some(BlockListPoint::new(
            row.max(Lines::zero()).min(max_row),
            column.min(self.size_info.columns() - 1),
        ))
    }

    // Take a point in the block list space and returns the point within a grid in the blocklist, if
    // contained with a block.
    pub fn block_list_point_to_grid_point(
        &self,
        point: BlockListPoint,
    ) -> Option<WithinBlock<IndexPoint>> {
        let mut block_heights_cursor = self
            .block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        block_heights_cursor.seek(&BlockHeight::from(point.row), SeekBias::Right);

        let block_index = match block_heights_cursor.item() {
            Some(BlockHeightItem::Block(_)) => {
                BlockIndex::from(block_heights_cursor.start().block_count)
            }
            _ => return None,
        };

        let block = self.block_list.block_at(block_index)?;

        let row_within_grid = point.row - block_heights_cursor.start().height;

        // Determine whether it is in the command grid, prompt/command grid, output grid or none of them.
        let (point, grid_type) = match block.find(row_within_grid) {
            BlockSection::PromptAndCommandGrid(row) => (
                IndexPoint {
                    row: row.as_f64() as usize,
                    col: point.column,
                },
                GridType::PromptAndCommand,
            ),
            BlockSection::OutputGrid(row) => (
                IndexPoint {
                    row: row.as_f64() as usize,
                    col: point.column,
                },
                GridType::Output,
            ),
            _ => return None,
        };

        Some(WithinBlock::new(point, block_index, grid_type))
    }

    // Translates a row in the BlockListPoint space into a row in
    // the scroll space of the viewport (how many rows from the top-left
    // block are we), taking into account block direction.
    pub fn block_list_y_to_scroll_y(&self, block_list_y_lines: Lines) -> Lines {
        match self.input_mode {
            InputMode::PinnedToBottom => block_list_y_lines,
            InputMode::Waterfall => {
                let Some(gap) = self.block_list.active_gap() else {
                    return block_list_y_lines;
                };
                let mut cursor = self
                    .block_list
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                cursor.seek(&BlockHeight::from(block_list_y_lines), SeekBias::Right);
                // If there's a gap we need to potentially adjust the position based
                // on whether we are before or after it.
                if cursor.start().total_count < gap.index().0 {
                    block_list_y_lines
                } else {
                    block_list_y_lines - gap.height()
                }
            }
            InputMode::PinnedToTop => {
                let total_block_height = self.block_list.block_heights().summary().height;
                let mut cursor = self
                    .block_list
                    .block_heights()
                    .cursor::<BlockHeight, BlockHeightSummary>();
                cursor.seek(&BlockHeight::from(block_list_y_lines), SeekBias::Right);
                let overhang = block_list_y_lines - cursor.start().height;
                let block_start = total_block_height - cursor.end().height;
                block_start + overhang
            }
        }
    }

    /// Returns the index (in the original grid) of the output row at the top of
    /// the viewport in the given block index. Returns None if the row at the top
    /// of the viewport is not in the output grid of the given block.
    pub fn get_first_visible_output_row(&self, block_index: BlockIndex) -> Option<usize> {
        let header_height = self.snackbar_header_height();
        let top_of_viewport = self.scroll_top_in_lines();
        let top_of_block = self.top_of_block_in_lines(block_index);
        let block = self.block_list.block_at(block_index)?;
        let viewport_offset_into_block = (top_of_viewport + header_height) - top_of_block;
        let viewport_offset_into_output =
            viewport_offset_into_block - block.output_grid_offset().into_lines();

        if !heights_approx_gte(viewport_offset_into_output, 0.0_f32.into_lines()) {
            // The viewport is before the block output.
            return None;
        }
        let output_offset_row = viewport_offset_into_output.as_f64().round() as usize;
        if output_offset_row >= block.output_grid().len_displayed() {
            // The viewport is after the block output.
            return None;
        }

        let original_row = block
            .output_grid()
            .grid_handler()
            .maybe_translate_row_from_displayed_to_original(output_offset_row);
        Some(original_row)
    }

    /// Convert a selection range in block list coordinate space to points in viewport coordinate space,
    /// which accounts for input mode.
    pub fn selection_as_viewport_points(
        &self,
        range: &SelectionRange,
    ) -> (SelectionPoint, SelectionPoint) {
        let start = SelectionPoint {
            row: self.block_list_y_to_scroll_y(range.start.row),
            col: range.start.column,
        };
        let end = SelectionPoint {
            row: self.block_list_y_to_scroll_y(range.end.row),
            col: range.end.column,
        };
        (start, end)
    }

    /// Returns true if the start point <= end point after converting the selection from
    /// block list coordinate space to viewport coordinate space, which accounts for input mode.
    pub fn is_range_in_order_in_viewport(&self, range: &SelectionRange) -> bool {
        let (start, end) = self.selection_as_viewport_points(range);
        start <= end
    }
}

impl Iterator for ViewportIter<'_> {
    type Item = ViewportIterItem;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If the block is not in the viewport at all, stop rendering. The blocks are ordered,
            // so this means the rest of the blocks also aren't in the viewport.
            //
            // Add a 'fudge' factor to ensure that an item with zero height right at the bottom of
            // the viewport may still get relaid out, if this condition would otherwise fail due to
            // floating point imprecision.
            if self.top_of_current_block.as_f64() > self.bottom_offset.as_f64() + f64::EPSILON * 5.
            {
                return None;
            }

            let item = self.block_heights_iter.next()?;

            // If there is a gap in waterfall mode, it needs to be handled
            // outside of the blocklist viewport because the gap is rendered below the input
            // box, so skip it.
            if matches!(item, BlockHeightItem::Gap(_))
                && matches!(self.input_mode, InputMode::Waterfall)
            {
                continue;
            }

            let is_block = matches!(item, BlockHeightItem::Block(_));

            // Note that in inverted case we need to return the updated
            // index in the ViewportIterItem, whereas in the non-inverted
            // case, we update the index after returning the item.  This is just
            // an artifact of the way the indexing works.
            if self.input_mode.is_inverted_blocklist() {
                if self.curr_entry_index.0 > 0 {
                    self.curr_entry_index = (self.curr_entry_index.0 - 1).into();
                }
                if is_block && self.curr_block_index > 0.into() {
                    self.curr_block_index -= 1.into();
                }
            }

            let block_index = if is_block {
                Some(self.curr_block_index)
            } else {
                None
            };

            let next = Some(ViewportIterItem {
                block_index,
                entry_index: self.curr_entry_index,
                block_height_item: *item,
                top_of_current_block: self.top_of_current_block,
            });

            let block_height = item.height();
            self.top_of_current_block += block_height.into_lines();

            if !self.input_mode.is_inverted_blocklist() {
                self.curr_entry_index = (self.curr_entry_index.0 + 1).into();
                if is_block {
                    self.curr_block_index += 1.into();
                }
            }

            match item {
                BlockHeightItem::RichContent(RichContentItem {
                    agent_view_conversation_id: fullscreen_agent_view_conversation_id,
                    ..
                }) => match self.agent_view_state {
                    AgentViewState::Active {
                        conversation_id,
                        display_mode: AgentViewDisplayMode::FullScreen,
                        ..
                    } => {
                        // If currently in a fullscreen agent view, only return this item if its
                        // conversation id matches that of the active agent view.
                        if fullscreen_agent_view_conversation_id
                            .is_some_and(|id| id == *conversation_id)
                        {
                            return next;
                        }
                    }
                    AgentViewState::Active {
                        display_mode: AgentViewDisplayMode::Inline,
                        ..
                    }
                    | AgentViewState::Inactive => {
                        // If not in a fullscreen agent view, return the item only if it 'belongs'
                        // to the terminal mode (represented as no `ai_conversation_id`).
                        if fullscreen_agent_view_conversation_id.is_none() {
                            return next;
                        }
                    }
                },
                _ => {
                    if !FeatureFlag::AgentView.is_enabled() || block_height.as_f64() > 0. {
                        return next;
                    }
                }
            }
        }
    }
}
