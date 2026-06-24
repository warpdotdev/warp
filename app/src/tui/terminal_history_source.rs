//! Terminal history adapter for the generic TUI virtual list.

use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::EntityId;
use warpui_core::elements::tui::{TuiBuffer, TuiRect, TuiVirtualListSource};

use super::grid_render;
use crate::ai::blocklist::agent_view::AgentViewState;
use crate::terminal::color;
use crate::terminal::model::block::{Block, BlockId};
use crate::terminal::model::blocks::{BlockHeightItem, BlockList};
use crate::terminal::model::terminal_model::TerminalModel;

/// Stable identity for a terminal-history item in the TUI transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalHistoryItemId {
    Block(BlockId),
    RichContent(EntityId),
}

/// Virtual-list source over a `TerminalModel`'s command block history.
pub struct TerminalHistorySource {
    model: Arc<FairMutex<TerminalModel>>,
    colors: color::List,
}

impl TerminalHistorySource {
    /// Creates a source backed by `model`.
    pub fn new(model: Arc<FairMutex<TerminalModel>>, colors: color::List) -> Self {
        Self { model, colors }
    }

    fn block_rows(block: &Block, agent_view_state: &AgentViewState) -> Option<(usize, usize)> {
        if !block.is_visible(agent_view_state) || !(block.started() || block.finished()) {
            return None;
        }

        let prompt_and_command_rows = if block.should_hide_command_grid() {
            0
        } else {
            block.prompt_and_command_grid().len_displayed()
        };
        let output_rows = if block.should_hide_output_grid() {
            0
        } else {
            block.output_grid().len_displayed()
        };

        (prompt_and_command_rows + output_rows > 0)
            .then_some((prompt_and_command_rows, output_rows))
    }

    fn item_id_for_height_item(
        block_list: &BlockList,
        height_item: &BlockHeightItem,
        block_count: &mut usize,
    ) -> Option<TerminalHistoryItemId> {
        match height_item {
            BlockHeightItem::Block(_) => {
                let item = block_list
                    .blocks()
                    .get(*block_count)
                    .map(|block| TerminalHistoryItemId::Block(block.id().clone()));
                *block_count += 1;
                item
            }
            BlockHeightItem::RichContent(item) => {
                Some(TerminalHistoryItemId::RichContent(item.view_id))
            }
            BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. } => None,
        }
    }

    fn item_height_for_block_list(block_list: &BlockList, item: &TerminalHistoryItemId) -> usize {
        match item {
            TerminalHistoryItemId::Block(block_id) => block_list
                .block_with_id(block_id)
                .and_then(|block| Self::block_rows(block, block_list.agent_view_state()))
                .map(|(prompt_and_command_rows, output_rows)| prompt_and_command_rows + output_rows)
                .unwrap_or(0),
            // Rich content is represented in the item model now so AI/rich blocks
            // have a stable seam. Rendering support can fill this in without
            // changing the virtual-list contract.
            TerminalHistoryItemId::RichContent(_) => 0,
        }
    }

    fn item_position(block_list: &BlockList, target: &TerminalHistoryItemId) -> Option<usize> {
        let mut block_count = 0usize;
        for (position, height_item) in block_list.block_heights().cursor::<(), ()>().enumerate() {
            if Self::item_id_for_height_item(block_list, height_item, &mut block_count)
                .as_ref()
                .is_some_and(|item| item == target)
            {
                return Some(position);
            }
        }
        None
    }

    fn find_next_from_position(
        block_list: &BlockList,
        start: usize,
    ) -> Option<TerminalHistoryItemId> {
        let mut block_count = 0usize;
        for (position, height_item) in block_list.block_heights().cursor::<(), ()>().enumerate() {
            let item = Self::item_id_for_height_item(block_list, height_item, &mut block_count);
            if position >= start {
                if let Some(item) = item {
                    if Self::item_height_for_block_list(block_list, &item) > 0 {
                        return Some(item);
                    }
                }
            }
        }
        None
    }

    fn find_previous_before_or_at_position(
        block_list: &BlockList,
        start: usize,
    ) -> Option<TerminalHistoryItemId> {
        let mut block_count = 0usize;
        let mut previous = None;
        for (position, height_item) in block_list.block_heights().cursor::<(), ()>().enumerate() {
            if position > start {
                break;
            }
            let item = Self::item_id_for_height_item(block_list, height_item, &mut block_count);
            if let Some(item) = item {
                if Self::item_height_for_block_list(block_list, &item) > 0 {
                    previous = Some(item);
                }
            }
        }
        previous
    }
}

impl TuiVirtualListSource for TerminalHistorySource {
    type ItemId = TerminalHistoryItemId;

    fn first_item(&self) -> Option<Self::ItemId> {
        let model = self.model.lock();
        Self::find_next_from_position(model.block_list(), 0)
    }

    fn last_item(&self) -> Option<Self::ItemId> {
        let model = self.model.lock();
        Self::find_previous_before_or_at_position(model.block_list(), usize::MAX)
    }

    fn next_item(&self, item: Self::ItemId) -> Option<Self::ItemId> {
        let model = self.model.lock();
        let block_list = model.block_list();
        let position = Self::item_position(block_list, &item)?;
        Self::find_next_from_position(block_list, position + 1)
    }

    fn previous_item(&self, item: Self::ItemId) -> Option<Self::ItemId> {
        let model = self.model.lock();
        let block_list = model.block_list();
        let position = Self::item_position(block_list, &item)?;
        position
            .checked_sub(1)
            .and_then(|position| Self::find_previous_before_or_at_position(block_list, position))
    }

    fn item_height(&self, item: Self::ItemId, _width: u16) -> usize {
        let model = self.model.lock();
        Self::item_height_for_block_list(model.block_list(), &item)
    }

    fn render_item_slice(
        &self,
        item: Self::ItemId,
        row_offset: usize,
        rows: u16,
        area: TuiRect,
        buffer: &mut TuiBuffer,
    ) {
        let TerminalHistoryItemId::Block(block_id) = item else {
            return;
        };
        let model = self.model.lock();
        let Some(block) = model.block_list().block_with_id(&block_id) else {
            return;
        };
        let Some((prompt_and_command_rows, output_rows)) =
            Self::block_rows(block, model.block_list().agent_view_state())
        else {
            return;
        };

        let mut remaining_rows = rows;
        let mut dst_y = area.y;

        if row_offset < prompt_and_command_rows {
            let rows_to_render =
                (prompt_and_command_rows - row_offset).min(usize::from(remaining_rows)) as u16;
            let slice_area = TuiRect::new(area.x, dst_y, area.width, rows_to_render);
            grid_render::render_block_grid_slice(
                block.prompt_and_command_grid(),
                row_offset,
                slice_area,
                buffer,
                &self.colors,
            );
            remaining_rows -= rows_to_render;
            dst_y = dst_y.saturating_add(rows_to_render);
        }

        if remaining_rows > 0 {
            let output_offset = row_offset.saturating_sub(prompt_and_command_rows);
            if output_offset < output_rows {
                let rows_to_render =
                    (output_rows - output_offset).min(usize::from(remaining_rows)) as u16;
                let slice_area = TuiRect::new(area.x, dst_y, area.width, rows_to_render);
                grid_render::render_block_grid_slice(
                    block.output_grid(),
                    output_offset,
                    slice_area,
                    buffer,
                    &self.colors,
                );
            }
        }
    }
}
