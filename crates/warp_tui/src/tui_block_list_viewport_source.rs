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
use warp::tui_export::{BlockHeight, BlockHeightItem, BlockHeightSummary, BlockId, TerminalModel};
use warpui::{EntityId, ViewHandle};
use warpui_core::elements::tui::{
    TuiElement, TuiViewportContent, TuiViewportWindow, TuiViewportedElement, TuiVisibleViewportItem,
};
use warpui_core::{AppContext, TuiView};

use super::agent_block::TuiAIBlock;
use super::terminal_block::{should_render_terminal_block, TerminalBlockVisibleRowsElement};

pub(super) type AgentBlockRegistry = Rc<RefCell<HashMap<EntityId, ViewHandle<TuiAIBlock>>>>;

/// Extra rows above and below the viewport whose non-dirty agent blocks are
/// re-measured each frame, so near-off-screen reflow (e.g. a width change) is
/// reflected before windowing. Mirrors the GUI blocklist's overhang pass.
const OVERHANG_ROWS: usize = 20;

/// Stable identities used by TUI block-list viewport tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiBlockListViewportItemId {
    TerminalBlock(BlockId),
    AgentBlock(EntityId),
}

struct TuiBlockListVisibleItem {
    origin_y: usize,
    /// Full cached height from the canonical `BlockList`.
    height: usize,
    kind: TuiBlockListVisibleItemKind,
}

enum TuiBlockListVisibleItemKind {
    TerminalBlock(BlockId),
    AgentBlock(ViewHandle<TuiAIBlock>),
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

    /// Collects the agent-block view ids to measure this frame: the drained
    /// dirty set (measured wherever they sit) plus every non-dirty agent block
    /// whose row range intersects the viewport window padded by [`OVERHANG_ROWS`].
    /// The overhang band catches reflow of near-off-screen blocks that were
    /// never dirtied, so their heights are fresh before the window is computed.
    fn agent_heights_to_measure(&self, window: TuiViewportWindow) -> HashSet<EntityId> {
        let mut model = self.model.lock();
        let mut view_ids = model.block_list_mut().take_dirty_rich_content_items();

        let agent_blocks = self.agent_blocks.borrow();
        let block_list = model.block_list();
        let band_top = window.scroll_top.saturating_sub(OVERHANG_ROWS);
        let band_bottom = window
            .scroll_top
            .saturating_add(usize::from(window.viewport_height))
            .saturating_add(OVERHANG_ROWS);
        let mut cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        cursor.seek_clamped(&BlockHeight::from(band_top as f64), SeekBias::Left);
        while let Some(item) = cursor.item() {
            let item_top = cursor.start().height.as_f64().floor().max(0.0) as usize;
            if item_top >= band_bottom {
                break;
            }
            let item_bottom = item_top.saturating_add(item.height().as_f64().ceil() as usize);
            if item_bottom > band_top {
                if let BlockHeightItem::RichContent(rich_content) = item {
                    if !rich_content.should_hide && agent_blocks.contains_key(&rich_content.view_id)
                    {
                        view_ids.insert(rich_content.view_id);
                    }
                }
            }
            cursor.next();
        }
        view_ids
    }

    /// Measures each agent block's wrapped height at `width`, returning heights
    /// in the block list's native line unit.
    fn measured_agent_heights(
        &self,
        view_ids: HashSet<EntityId>,
        width: u16,
        app: &AppContext,
    ) -> HashMap<EntityId, BlockHeight> {
        let agent_blocks = self.agent_blocks.borrow();
        view_ids
            .into_iter()
            .filter_map(|view_id| {
                let view = agent_blocks.get(&view_id)?;
                Some((
                    view_id,
                    BlockHeight::from(view.as_ref(app).desired_height(width, app).max(1) as f64),
                ))
            })
            .collect()
    }

    /// Writes measured rich-content heights back to the canonical block list.
    /// Heights are already in the block list's native line unit (one line per
    /// terminal row), so no pixel round-trip is needed.
    fn write_line_heights(&self, line_heights: &HashMap<EntityId, BlockHeight>) {
        if line_heights.is_empty() {
            return;
        }
        self.model
            .lock()
            .block_list_mut()
            .update_rich_content_heights_in_lines(line_heights);
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
                    let height = item.height().as_f64().ceil().max(0.0) as usize;
                    let block = block_list.block_at(cursor.start().block_count.into());
                    block.and_then(|block| {
                        if height == 0 || !should_render_terminal_block(block, block_list) {
                            return None;
                        }

                        Some(TuiBlockListVisibleItem {
                            origin_y: item_top,
                            height,
                            kind: TuiBlockListVisibleItemKind::TerminalBlock(block.id().clone()),
                        })
                    })
                }
                BlockHeightItem::RichContent(item) => {
                    if item.should_hide {
                        None
                    } else if let Some(view) = agent_blocks.get(&item.view_id) {
                        let height = item.last_laid_out_height.as_f64().ceil().max(1.0) as usize;
                        Some(TuiBlockListVisibleItem {
                            origin_y: item_top,
                            height,
                            kind: TuiBlockListVisibleItemKind::AgentBlock(view.clone()),
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
                let rendered_item_bottom = item_top.saturating_add(item.height);
                if rendered_item_bottom > window.scroll_top && item_top < viewport_bottom {
                    visible_items.push(item);
                }
            }
            cursor.next();
        }

        (content_height, visible_items)
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
                        block.filter(|block| should_render_terminal_block(block, block_list))
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
        // Refresh cached heights before windowing: the dirty set plus a band of
        // near-off-screen agent blocks (see `agent_heights_to_measure`).
        let view_ids_to_measure = self.agent_heights_to_measure(window);
        let heights = self.measured_agent_heights(view_ids_to_measure, available_width, app);
        self.write_line_heights(&heights);

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
}

impl TuiBlockListVisibleItem {
    fn visible_rows(&self, window: TuiViewportWindow) -> Range<usize> {
        let item_top = self.origin_y;
        let item_bottom = item_top.saturating_add(self.height);
        let visible_top = item_top.max(window.scroll_top);
        let visible_bottom = item_bottom.min(
            window
                .scroll_top
                .saturating_add(usize::from(window.viewport_height)),
        );
        visible_top.saturating_sub(item_top)..visible_bottom.saturating_sub(item_top)
    }

    fn render(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        window: TuiViewportWindow,
        available_width: u16,
        app: &AppContext,
    ) -> TuiVisibleViewportItem {
        let visible_rows = self.visible_rows(window);
        // Terminal blocks get pre-sliced below; rich content stays whole and lets `TuiClipped`
        // handle any partial visibility.
        let origin_y = match &self.kind {
            TuiBlockListVisibleItemKind::TerminalBlock(_) => {
                self.origin_y.saturating_add(visible_rows.start)
            }
            TuiBlockListVisibleItemKind::AgentBlock(_) => self.origin_y,
        };
        TuiVisibleViewportItem {
            origin_y,
            element: self.render_element(model, visible_rows, available_width, app),
        }
    }

    fn render_element(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        visible_rows: Range<usize>,
        width: u16,
        app: &AppContext,
    ) -> Box<dyn TuiElement> {
        match self.kind {
            TuiBlockListVisibleItemKind::TerminalBlock(block_id) => {
                debug_assert!(visible_rows.end <= self.height);
                TerminalBlockVisibleRowsElement::new(model.clone(), block_id, visible_rows, width)
                    .finish()
            }
            TuiBlockListVisibleItemKind::AgentBlock(view) => view.as_ref(app).render(app),
        }
    }
}

#[cfg(test)]
#[path = "tui_block_list_viewport_source_tests.rs"]
mod tests;
