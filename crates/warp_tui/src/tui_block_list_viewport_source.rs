//! TUI viewport source backed by the canonical terminal block list.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use sum_tree::SeekBias;
#[cfg(test)]
use warp::tui_export::TotalIndex;
use warp::tui_export::{
    Block, BlockHeight, BlockHeightItem, BlockHeightSummary, BlockId, BlockList, TerminalModel,
};
use warpui::{EntityId, ViewHandle};
use warpui_core::elements::tui::{
    TuiElement, TuiMeasuredViewportItemHeight, TuiViewportContent, TuiViewportWindow,
    TuiViewportedElement, TuiVisibleViewportItem,
};
use warpui_core::{AppContext, TuiView};

use super::agent_block::TuiAgentBlockView;
use super::terminal_block::TerminalBlockRowsElement;

pub(super) type AgentBlockRegistry = Rc<RefCell<HashMap<EntityId, ViewHandle<TuiAgentBlockView>>>>;

/// Stable identities used by TUI block-list viewport tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiBlockListViewportItemId {
    TerminalBlock(BlockId),
    AgentBlock(EntityId),
}

enum TuiBlockListVisibleItem {
    TerminalBlock {
        origin_y: usize,
        height: usize,
        block_id: BlockId,
    },
    AgentBlock {
        origin_y: usize,
        height: usize,
        view: ViewHandle<TuiAgentBlockView>,
    },
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
                let view = agent_blocks.get(&view_id)?;
                Some((
                    view_id,
                    view.as_ref(app).desired_height(width, app).max(1) as f64,
                ))
            })
            .collect()
    }

    fn visible_items_in_window(
        &self,
        window: TuiViewportWindow,
    ) -> (usize, Vec<TuiBlockListVisibleItem>) {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let viewport_bottom = window
            .scroll_top
            .saturating_add(usize::from(window.viewport_height));
        let mut visible_items = Vec::new();
        let content_height = block_list
            .block_heights()
            .summary()
            .height
            .as_f64()
            .ceil()
            .max(0.0) as usize;
        let mut cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        cursor.seek_clamped(&BlockHeight::from(window.scroll_top as f64), SeekBias::Left);

        while let Some(item) = cursor.item() {
            let item_top = cursor.start().height.as_f64().floor().max(0.0) as usize;
            let item_bottom = item_top.saturating_add(item.height().as_f64().ceil() as usize);
            if item_bottom <= window.scroll_top {
                cursor.next();
                continue;
            }
            if item_top >= viewport_bottom {
                break;
            }
            let visible_item = match item {
                BlockHeightItem::Block(_) => {
                    let block = block_list.block_at(cursor.start().block_count.into());
                    block.and_then(|block| {
                        let height = block_rows(block, block_list)?;
                        Some(TuiBlockListVisibleItem::TerminalBlock {
                            origin_y: item_top,
                            height,
                            block_id: block.id().clone(),
                        })
                    })
                }
                BlockHeightItem::RichContent(item) => {
                    if item.should_hide {
                        None
                    } else if let Some(view) = agent_blocks.get(&item.view_id) {
                        let height = item.last_laid_out_height.as_f64().ceil().max(1.0) as usize;
                        Some(TuiBlockListVisibleItem::AgentBlock {
                            origin_y: item_top,
                            height,
                            view: view.clone(),
                        })
                    } else {
                        None
                    }
                }
                BlockHeightItem::Gap(_)
                | BlockHeightItem::RestoredBlockSeparator { .. }
                | BlockHeightItem::InlineBanner { .. }
                | BlockHeightItem::SubshellSeparator { .. } => None,
            };
            if let Some(item) = visible_item {
                let height = item.height();
                let rendered_item_bottom = item_top.saturating_add(height);
                if rendered_item_bottom > window.scroll_top && item_top < viewport_bottom {
                    visible_items.push(item);
                }
            }
            cursor.next();
        }

        (content_height, visible_items)
    }

    fn apply_height_updates(&self, height_updates: &HashMap<EntityId, f64>) {
        if height_updates.is_empty() {
            return;
        }
        let mut model = self.model.lock();
        let cell_height_px = f64::from(model.block_list().size().cell_height_px().as_f32());
        let height_updates = height_updates
            .iter()
            .map(|(view_id, height)| (*view_id, height * cell_height_px))
            .collect::<HashMap<_, _>>();
        model
            .block_list_mut()
            .update_rich_content_heights(&height_updates);
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
        let dirty_rich_content_items = self
            .model
            .lock()
            .block_list_mut()
            .take_dirty_rich_content_items();
        let height_updates =
            self.measured_dirty_agent_heights(dirty_rich_content_items, available_width, app);
        self.apply_height_updates(&height_updates);
        let (content_height, visible_items) = self.visible_items_in_window(window);
        let items = visible_items
            .into_iter()
            .map(|item| item.render(&self.model, window, available_width, app))
            .collect();

        TuiViewportContent {
            content_height,
            items,
        }
    }

    fn update_visible_item_heights(
        &self,
        measured_heights: &[TuiMeasuredViewportItemHeight],
        _app: &AppContext,
    ) -> bool {
        if measured_heights.is_empty() {
            return false;
        }

        let mut model = self.model.lock();
        let cell_height_px = f64::from(model.block_list().size().cell_height_px().as_f32());
        let height_updates = measured_heights
            .iter()
            .filter_map(|measured_height| {
                let measured_height_rows = f64::from(measured_height.height.max(1));
                let current_height_rows =
                    rich_content_height_rows(model.block_list(), measured_height.item_id)?;
                (f64::abs(measured_height_rows - current_height_rows) > 0.01).then_some((
                    measured_height.item_id,
                    measured_height_rows * cell_height_px,
                ))
            })
            .collect::<HashMap<_, _>>();
        if height_updates.is_empty() {
            return false;
        }

        model
            .block_list_mut()
            .update_rich_content_heights(&height_updates);
        true
    }
}

impl TuiBlockListVisibleItem {
    fn height(&self) -> usize {
        match self {
            Self::TerminalBlock { height, .. } | Self::AgentBlock { height, .. } => *height,
        }
    }

    fn origin_y(&self) -> usize {
        match self {
            Self::TerminalBlock { origin_y, .. } | Self::AgentBlock { origin_y, .. } => *origin_y,
        }
    }
    fn render(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        window: TuiViewportWindow,
        available_width: u16,
        app: &AppContext,
    ) -> TuiVisibleViewportItem {
        let visible_rows = self.visible_rows(window);
        let measured_height_id = match &self {
            Self::AgentBlock { view, .. } => Some(view.id()),
            Self::TerminalBlock { .. } => None,
        };
        let origin_y = self.origin_y();
        let render_rows = if matches!(&self, TuiBlockListVisibleItem::TerminalBlock { .. }) {
            0..self.height()
        } else {
            visible_rows
        };
        TuiVisibleViewportItem {
            origin_y,
            measured_height_id,
            element: self.render_element(model, render_rows, available_width, app),
        }
    }

    fn visible_rows(&self, window: TuiViewportWindow) -> Range<usize> {
        let item_top = self.origin_y();
        let item_bottom = item_top.saturating_add(self.height());
        let visible_top = item_top.max(window.scroll_top);
        let visible_bottom = item_bottom.min(
            window
                .scroll_top
                .saturating_add(usize::from(window.viewport_height)),
        );
        visible_top.saturating_sub(item_top)..visible_bottom.saturating_sub(item_top)
    }

    fn render_element(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        visible_rows: Range<usize>,
        width: u16,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        match self {
            Self::TerminalBlock { block_id, .. } => Box::new(TerminalBlockRowsElement::new(
                model.clone(),
                block_id,
                visible_rows,
                width,
            )),
            Self::AgentBlock { view, .. } => view.as_ref(app).render(app),
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

fn rich_content_height_rows(block_list: &BlockList, view_id: EntityId) -> Option<f64> {
    block_list
        .block_heights()
        .cursor::<(), ()>()
        .find_map(|item| match item {
            BlockHeightItem::RichContent(item) if item.view_id == view_id => {
                Some(item.last_laid_out_height.as_f64())
            }
            BlockHeightItem::Block(_)
            | BlockHeightItem::RichContent(_)
            | BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. } => None,
        })
}

#[cfg(test)]
#[path = "tui_block_list_viewport_source_tests.rs"]
mod tests;
