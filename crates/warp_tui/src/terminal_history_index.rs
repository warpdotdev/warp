//! Canonical terminal-history ordering adapter for the generalized TUI viewport.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use sum_tree::SeekBias;
use warp::tui_export::{
    AIAgentExchangeId, AIConversationId, Block, BlockHeightItem, BlockHeightSummary, BlockId,
    BlockList, TerminalModel, TotalIndex,
};
use warpui::{EntityId, ViewHandle};
use warpui_core::elements::tui::{
    TuiElement, TuiViewportContent, TuiViewportWindow, TuiViewportedElement, TuiVisibleViewportItem,
};
use warpui_core::AppContext;

use super::agent_block::TuiAgentBlockView;
use super::terminal_block::render_terminal_block_rows;

/// A registered TUI agent block and its canonical exchange identity.
#[derive(Clone)]
pub(super) struct AgentBlockRegistration {
    pub(super) view: ViewHandle<TuiAgentBlockView>,
    pub(super) conversation_id: AIConversationId,
    pub(super) exchange_id: AIAgentExchangeId,
}

pub(super) type AgentBlockRegistry = Rc<RefCell<HashMap<EntityId, AgentBlockRegistration>>>;

/// Stable identities used by terminal-history tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TerminalHistoryItemId {
    TerminalBlock(BlockId),
    AgentBlock(EntityId),
}

enum TerminalHistoryVisibleItem {
    TerminalBlock {
        block_id: BlockId,
    },
    AgentBlock {
        registration: AgentBlockRegistration,
    },
}

struct TerminalHistoryVisibleItemDescriptor {
    origin_y: usize,
    height: usize,
    item: TerminalHistoryVisibleItem,
}

/// Adapts a terminal model's canonical block-list order for TUI viewporting.
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

    fn measured_dirty_agent_heights(
        &self,
        width: u16,
        app: &AppContext,
    ) -> HashMap<EntityId, usize> {
        let agent_blocks = self.agent_blocks.borrow();
        let dirty_agent_blocks = self.dirty_agent_blocks.borrow();
        agent_blocks
            .iter()
            .filter_map(|(view_id, registration)| {
                dirty_agent_blocks.contains(view_id).then(|| {
                    (
                        *view_id,
                        registration.view.as_ref(app).desired_height(width, app),
                    )
                })
            })
            .collect()
    }

    fn visible_item_descriptors(
        &self,
        window: TuiViewportWindow,
        measured_agent_heights: &HashMap<EntityId, usize>,
    ) -> (
        usize,
        Vec<TerminalHistoryVisibleItemDescriptor>,
        HashMap<EntityId, f64>,
    ) {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let dirty_agent_blocks = self.dirty_agent_blocks.borrow();
        let viewport_bottom = window.scroll_top.saturating_add(window.viewport_height);
        let mut descriptors = Vec::new();
        let mut height_updates = HashMap::new();
        let mut content_height = 0usize;
        let mut cursor = block_list
            .block_heights()
            .cursor::<TotalIndex, BlockHeightSummary>();
        cursor.seek(&TotalIndex(0), SeekBias::Right);

        while let Some(item) = cursor.item() {
            let descriptor = match item {
                BlockHeightItem::Block(_) => {
                    let block = block_list.block_at(cursor.start().block_count.into());
                    block.and_then(|block| {
                        let height = block_rows(block, block_list)?;
                        Some((
                            height,
                            TerminalHistoryVisibleItem::TerminalBlock {
                                block_id: block.id().clone(),
                            },
                        ))
                    })
                }
                BlockHeightItem::RichContent(item) => {
                    if item.should_hide {
                        None
                    } else if let Some(registration) = agent_blocks.get(&item.view_id) {
                        let height = if dirty_agent_blocks.contains(&item.view_id) {
                            let height = measured_agent_heights
                                .get(&item.view_id)
                                .copied()
                                .unwrap_or(1)
                                .max(1);
                            height_updates.insert(item.view_id, height as f64);
                            height
                        } else {
                            item.last_laid_out_height.as_f64().ceil().max(1.0) as usize
                        };
                        Some((
                            height,
                            TerminalHistoryVisibleItem::AgentBlock {
                                registration: registration.clone(),
                            },
                        ))
                    } else {
                        None
                    }
                }
                BlockHeightItem::Gap(_)
                | BlockHeightItem::RestoredBlockSeparator { .. }
                | BlockHeightItem::InlineBanner { .. }
                | BlockHeightItem::SubshellSeparator { .. } => None,
            };

            if let Some((height, item)) = descriptor {
                let item_top = content_height;
                let item_bottom = item_top.saturating_add(height);
                if item_bottom > window.scroll_top && item_top < viewport_bottom {
                    descriptors.push(TerminalHistoryVisibleItemDescriptor {
                        origin_y: item_top,
                        height,
                        item,
                    });
                }
                content_height = item_bottom;
            }
            cursor.next();
        }

        (content_height, descriptors, height_updates)
    }

    fn apply_height_updates(&self, height_updates: HashMap<EntityId, f64>) {
        if height_updates.is_empty() {
            return;
        }
        self.model
            .lock()
            .block_list_mut()
            .update_rich_content_heights(&height_updates);
        let mut dirty_agent_blocks = self.dirty_agent_blocks.borrow_mut();
        for view_id in height_updates.keys() {
            dirty_agent_blocks.remove(view_id);
        }
    }

    #[cfg(test)]
    pub(super) fn item_ids_for_test(&self) -> Vec<TerminalHistoryItemId> {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let mut item_ids = Vec::new();
        let mut cursor = block_list
            .block_heights()
            .cursor::<TotalIndex, BlockHeightSummary>();
        cursor.seek(&TotalIndex(0), SeekBias::Right);

        while let Some(item) = cursor.item() {
            match item {
                BlockHeightItem::Block(_) => {
                    let block = block_list.block_at(cursor.start().block_count.into());
                    if let Some(block) =
                        block.filter(|block| block_rows(block, block_list).is_some())
                    {
                        item_ids.push(TerminalHistoryItemId::TerminalBlock(block.id().clone()));
                    }
                }
                BlockHeightItem::RichContent(item)
                    if !item.should_hide && agent_blocks.contains_key(&item.view_id) =>
                {
                    item_ids.push(TerminalHistoryItemId::AgentBlock(item.view_id));
                }
                BlockHeightItem::RichContent(_)
                | BlockHeightItem::Gap(_)
                | BlockHeightItem::RestoredBlockSeparator { .. }
                | BlockHeightItem::InlineBanner { .. }
                | BlockHeightItem::SubshellSeparator { .. } => {}
            }
            cursor.next();
        }
        item_ids
    }
}

impl TuiViewportedElement for TerminalHistoryIndex {
    fn visible_items(&self, window: TuiViewportWindow, app: &AppContext) -> TuiViewportContent {
        let measured_agent_heights = self.measured_dirty_agent_heights(window.viewport_width, app);
        let (content_height, descriptors, height_updates) =
            self.visible_item_descriptors(window, &measured_agent_heights);
        self.apply_height_updates(height_updates);

        let items = descriptors
            .into_iter()
            .map(|descriptor| descriptor.render(&self.model, window, app))
            .collect();

        TuiViewportContent {
            content_height,
            items,
        }
    }
}

impl TerminalHistoryVisibleItemDescriptor {
    fn render(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        window: TuiViewportWindow,
        app: &AppContext,
    ) -> TuiVisibleViewportItem {
        let visible_rows = self.visible_rows(window);
        let origin_y = if matches!(&self.item, TerminalHistoryVisibleItem::TerminalBlock { .. }) {
            self.origin_y.saturating_add(visible_rows.start)
        } else {
            self.origin_y
        };
        TuiVisibleViewportItem {
            origin_y,
            element: self
                .item
                .render(model, visible_rows, window.viewport_width, app),
        }
    }

    fn visible_rows(&self, window: TuiViewportWindow) -> Range<usize> {
        let item_top = self.origin_y;
        let item_bottom = item_top.saturating_add(self.height);
        let visible_top = item_top.max(window.scroll_top);
        let visible_bottom =
            item_bottom.min(window.scroll_top.saturating_add(window.viewport_height));
        visible_top.saturating_sub(item_top)..visible_bottom.saturating_sub(item_top)
    }
}

impl TerminalHistoryVisibleItem {
    fn render(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        visible_rows: Range<usize>,
        width: u16,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        match self {
            Self::TerminalBlock { block_id } => {
                render_terminal_block_rows(model, &block_id, visible_rows, width)
            }
            Self::AgentBlock { registration } => registration.view.as_ref(app).render_full(app),
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
