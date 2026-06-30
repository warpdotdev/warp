//! TUI viewport source backed by the canonical terminal block list.

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

/// Stable identities used by TUI block-list viewport tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiBlockListViewportItemId {
    TerminalBlock(BlockId),
    AgentBlock(EntityId),
}

enum TuiBlockListVisibleItem {
    TerminalBlock {
        block_id: BlockId,
    },
    AgentBlock {
        registration: AgentBlockRegistration,
    },
}

struct TuiBlockListVisibleItemDescriptor {
    origin_y: usize,
    height: usize,
    item: TuiBlockListVisibleItem,
}

/// Adapts a terminal model's canonical block-list order for TUI viewporting.
#[derive(Clone)]
pub(super) struct TuiBlockListViewportSource {
    model: Arc<FairMutex<TerminalModel>>,
    agent_blocks: AgentBlockRegistry,
}

impl TuiBlockListViewportSource {
    /// Creates a TUI viewport source over the canonical terminal model.
    pub(super) fn new(
        model: Arc<FairMutex<TerminalModel>>,
        agent_blocks: AgentBlockRegistry,
    ) -> Self {
        Self {
            model,
            agent_blocks,
        }
    }

    fn take_dirty_rich_content_items(&self) -> HashSet<EntityId> {
        self.model
            .lock()
            .block_list_mut()
            .take_dirty_rich_content_items()
    }

    fn measured_dirty_agent_heights(
        &self,
        dirty_rich_content_items: HashSet<EntityId>,
        width: u16,
        app: &AppContext,
    ) -> HashMap<EntityId, f64> {
        let agent_blocks = self.agent_blocks.borrow();
        dirty_rich_content_items
            .into_iter()
            .filter_map(|view_id| {
                let registration = agent_blocks.get(&view_id)?;
                Some((
                    view_id,
                    registration
                        .view
                        .as_ref(app)
                        .desired_height(width, app)
                        .max(1) as f64,
                ))
            })
            .collect()
    }

    fn visible_item_descriptors(
        &self,
        window: TuiViewportWindow,
    ) -> (usize, Vec<TuiBlockListVisibleItemDescriptor>) {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let viewport_bottom = window
            .scroll_top
            .saturating_add(usize::from(window.viewport_height));
        let mut descriptors = Vec::new();
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
                            TuiBlockListVisibleItem::TerminalBlock {
                                block_id: block.id().clone(),
                            },
                        ))
                    })
                }
                BlockHeightItem::RichContent(item) => {
                    if item.should_hide {
                        None
                    } else if let Some(registration) = agent_blocks.get(&item.view_id) {
                        let height = item.last_laid_out_height.as_f64().ceil().max(1.0) as usize;
                        Some((
                            height,
                            TuiBlockListVisibleItem::AgentBlock {
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
                    descriptors.push(TuiBlockListVisibleItemDescriptor {
                        origin_y: item_top,
                        height,
                        item,
                    });
                }
                content_height = item_bottom;
            }
            cursor.next();
        }

        (content_height, descriptors)
    }

    fn apply_height_updates(&self, height_updates: &HashMap<EntityId, f64>) {
        if height_updates.is_empty() {
            return;
        }
        self.model
            .lock()
            .block_list_mut()
            .update_rich_content_heights(height_updates);
    }

    #[cfg(test)]
    pub(super) fn item_ids_for_test(&self) -> Vec<TuiBlockListViewportItemId> {
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
                        item_ids.push(TuiBlockListViewportItemId::TerminalBlock(
                            block.id().clone(),
                        ));
                    }
                }
                BlockHeightItem::RichContent(item)
                    if !item.should_hide && agent_blocks.contains_key(&item.view_id) =>
                {
                    item_ids.push(TuiBlockListViewportItemId::AgentBlock(item.view_id));
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

impl TuiViewportedElement for TuiBlockListViewportSource {
    fn visible_items(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        app: &AppContext,
    ) -> TuiViewportContent {
        let dirty_rich_content_items = self.take_dirty_rich_content_items();
        let height_updates =
            self.measured_dirty_agent_heights(dirty_rich_content_items, available_width, app);
        self.apply_height_updates(&height_updates);

        let (content_height, descriptors) = self.visible_item_descriptors(window);
        let items = descriptors
            .into_iter()
            .map(|descriptor| descriptor.render(&self.model, window, available_width, app))
            .collect();

        TuiViewportContent {
            content_height,
            items,
        }
    }
}

impl TuiBlockListVisibleItemDescriptor {
    fn render(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        window: TuiViewportWindow,
        available_width: u16,
        app: &AppContext,
    ) -> TuiVisibleViewportItem {
        let visible_rows = self.visible_rows(window);
        let origin_y = if matches!(&self.item, TuiBlockListVisibleItem::TerminalBlock { .. }) {
            self.origin_y.saturating_add(visible_rows.start)
        } else {
            self.origin_y
        };
        TuiVisibleViewportItem {
            origin_y,
            element: self
                .item
                .render(model, visible_rows, available_width, app),
        }
    }

    fn visible_rows(&self, window: TuiViewportWindow) -> Range<usize> {
        let item_top = self.origin_y;
        let item_bottom = item_top.saturating_add(self.height);
        let visible_top = item_top.max(window.scroll_top);
        let visible_bottom =
            item_bottom.min(window.scroll_top.saturating_add(usize::from(window.viewport_height)));
        visible_top.saturating_sub(item_top)..visible_bottom.saturating_sub(item_top)
    }
}

impl TuiBlockListVisibleItem {
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
#[path = "tui_block_list_viewport_source_tests.rs"]
mod tests;
