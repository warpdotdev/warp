//! Canonical terminal-history ordering adapter for the generalized TUI viewport.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use sum_tree::{Cursor, SeekBias};
use warp::tui_export::{
    AIAgentExchangeId, AIConversationId, Block, BlockHeightItem, BlockHeightSummary, BlockId,
    BlockList, TerminalModel, TotalIndex,
};
use warpui::{EntityId, ViewHandle};
use warpui_core::elements::tui::{
    TuiViewportCursor, TuiViewportIndex, TuiViewportIndexItem, TuiViewportIndexPosition,
};

use super::agent_block::TuiAgentBlockView;

/// A registered TUI agent block and its canonical exchange identity.
#[derive(Clone)]
pub(super) struct AgentBlockRegistration {
    pub(super) view: ViewHandle<TuiAgentBlockView>,
    pub(super) conversation_id: AIConversationId,
    pub(super) exchange_id: AIAgentExchangeId,
}

pub(super) type AgentBlockRegistry = Rc<RefCell<HashMap<EntityId, AgentBlockRegistration>>>;

/// Stable identities used by the transcript viewport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TerminalHistoryItemId {
    TerminalBlock(BlockId),
    AgentBlock(EntityId),
}

/// Owned item descriptors passed to the transcript renderer.
pub(super) enum TerminalHistoryItem {
    TerminalBlock {
        block_id: BlockId,
    },
    AgentBlock {
        registration: AgentBlockRegistration,
    },
}

/// Adapts a terminal model's canonical block-list sum tree for TUI viewporting.
#[derive(Clone)]
pub(super) struct TerminalHistoryIndex {
    model: Arc<FairMutex<TerminalModel>>,
    agent_blocks: AgentBlockRegistry,
    dirty_agent_blocks: Rc<RefCell<HashSet<EntityId>>>,
}

impl TerminalHistoryIndex {
    /// Creates a terminal-history index over the canonical terminal model.
    pub(super) fn new(
        model: Arc<FairMutex<TerminalModel>>,
        agent_blocks: AgentBlockRegistry,
        dirty_agent_blocks: Rc<RefCell<HashSet<EntityId>>>,
    ) -> Self {
        Self {
            model,
            agent_blocks,
            dirty_agent_blocks,
        }
    }
}

struct TerminalHistoryCursor<'a> {
    block_list: &'a BlockList,
    agent_blocks: &'a HashMap<EntityId, AgentBlockRegistration>,
    dirty_agent_blocks: &'a HashSet<EntityId>,
    cursor: Cursor<'a, BlockHeightItem, TotalIndex, BlockHeightSummary>,
}

impl TerminalHistoryCursor<'_> {
    fn move_to_supported_next(&mut self) {
        while self.cursor.item().is_some() && !self.current_is_supported() {
            self.cursor.next();
        }
    }

    fn move_to_supported_prev(&mut self) {
        while self.cursor.item().is_some() && !self.current_is_supported() {
            self.cursor.prev();
        }
    }

    fn current_is_supported(&self) -> bool {
        match self.cursor.item() {
            Some(BlockHeightItem::Block(_)) => self
                .current_block()
                .is_some_and(|block| block_rows(block, self.block_list).is_some()),
            Some(BlockHeightItem::RichContent(item)) => {
                !item.should_hide && self.agent_blocks.contains_key(&item.view_id)
            }
            Some(
                BlockHeightItem::Gap(_)
                | BlockHeightItem::RestoredBlockSeparator { .. }
                | BlockHeightItem::InlineBanner { .. }
                | BlockHeightItem::SubshellSeparator { .. },
            )
            | None => false,
        }
    }

    fn current_block(&self) -> Option<&Block> {
        self.block_list
            .block_at(self.cursor.start().block_count.into())
    }
}

impl TuiViewportCursor for TerminalHistoryCursor<'_> {
    type ItemId = TerminalHistoryItemId;
    type Item = TerminalHistoryItem;

    fn item(&self) -> Option<TuiViewportIndexItem<Self::ItemId, Self::Item>> {
        match self.cursor.item()? {
            BlockHeightItem::Block(_) => {
                let block = self.current_block()?;
                let height = block_rows(block, self.block_list)?;
                let block_id = block.id().clone();
                Some(TuiViewportIndexItem {
                    id: TerminalHistoryItemId::TerminalBlock(block_id.clone()),
                    item: TerminalHistoryItem::TerminalBlock { block_id },
                    height,
                    needs_measurement: false,
                })
            }
            BlockHeightItem::RichContent(item) => {
                let registration = self.agent_blocks.get(&item.view_id)?.clone();
                Some(TuiViewportIndexItem {
                    id: TerminalHistoryItemId::AgentBlock(item.view_id),
                    item: TerminalHistoryItem::AgentBlock { registration },
                    height: item.last_laid_out_height.as_f64().ceil().max(1.0) as usize,
                    needs_measurement: self.dirty_agent_blocks.contains(&item.view_id),
                })
            }
            BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. } => None,
        }
    }

    fn next(&mut self) {
        self.cursor.next();
        self.move_to_supported_next();
    }

    fn prev(&mut self) {
        self.cursor.prev();
        self.move_to_supported_prev();
    }
}

impl TuiViewportIndex for TerminalHistoryIndex {
    type ItemId = TerminalHistoryItemId;
    type Item = TerminalHistoryItem;

    fn with_cursor<R>(
        &self,
        position: TuiViewportIndexPosition<'_, Self::ItemId>,
        f: impl FnOnce(&mut dyn TuiViewportCursor<ItemId = Self::ItemId, Item = Self::Item>) -> R,
    ) -> R {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let dirty_agent_blocks = self.dirty_agent_blocks.borrow();
        let mut cursor = block_list
            .block_heights()
            .cursor::<TotalIndex, BlockHeightSummary>();
        match position {
            TuiViewportIndexPosition::Start => {
                cursor.seek(&TotalIndex(0), SeekBias::Right);
            }
            TuiViewportIndexPosition::End => {
                cursor.seek(
                    &TotalIndex(block_list.block_heights().summary().total_count),
                    SeekBias::Right,
                );
                cursor.prev();
            }
            TuiViewportIndexPosition::Item(item_id) => {
                let position = match item_id {
                    TerminalHistoryItemId::TerminalBlock(block_id) => {
                        block_list.total_index_for_block_id(block_id)
                    }
                    TerminalHistoryItemId::AgentBlock(view_id) => {
                        block_list.total_index_for_rich_content(*view_id)
                    }
                };
                cursor.seek(&position.unwrap_or_default(), SeekBias::Right);
            }
        }
        let mut cursor = TerminalHistoryCursor {
            block_list,
            agent_blocks: &agent_blocks,
            dirty_agent_blocks: &dirty_agent_blocks,
            cursor,
        };
        match position {
            TuiViewportIndexPosition::End => cursor.move_to_supported_prev(),
            TuiViewportIndexPosition::Start | TuiViewportIndexPosition::Item(_) => {
                cursor.move_to_supported_next()
            }
        }
        f(&mut cursor)
    }

    fn update_heights(&self, updates: &[(Self::ItemId, usize)]) {
        let mut heights = HashMap::new();
        let mut dirty = self.dirty_agent_blocks.borrow_mut();
        for (item_id, height) in updates {
            if let TerminalHistoryItemId::AgentBlock(view_id) = item_id {
                heights.insert(*view_id, *height as f64);
                dirty.remove(view_id);
            }
        }
        if !heights.is_empty() {
            self.model
                .lock()
                .block_list_mut()
                .update_rich_content_heights(&heights);
        }
    }
}

fn block_rows(block: &Block, block_list: &BlockList) -> Option<usize> {
    if !block.is_visible(block_list.agent_view_state()) || !(block.started() || block.finished()) {
        return None;
    }
    let prompt_rows = if block.should_hide_command_grid() {
        0
    } else {
        block.prompt_and_command_grid().len_displayed()
    };
    let output_rows = if block.should_hide_output_grid() {
        0
    } else {
        block.output_grid().len_displayed()
    };
    (prompt_rows + output_rows > 0).then_some(prompt_rows + output_rows)
}

#[cfg(test)]
#[path = "terminal_history_index_tests.rs"]
mod tests;
