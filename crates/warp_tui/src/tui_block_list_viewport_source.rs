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
    TuiChildView, TuiElement, TuiLayoutContext, TuiRowResize, TuiSelectionSpan, TuiViewportContent,
    TuiViewportWindow, TuiViewportedElement, TuiVisibleViewportItem,
};
use warpui_core::AppContext;

use super::agent_block::TuiAIBlock;
use super::terminal_block::{should_render_terminal_block, TerminalBlockElement};
use super::tui_cli_subagent_view::TuiCLISubagentView;

pub(super) type AgentBlockRegistry = Rc<RefCell<HashMap<EntityId, ViewHandle<TuiAIBlock>>>>;
pub(super) type CLISubagentBlockRegistry =
    Rc<RefCell<HashMap<EntityId, ViewHandle<TuiCLISubagentView>>>>;

/// Extra rows above and below the viewport whose non-dirty agent blocks are
/// re-measured each frame, so near-off-screen reflow (e.g. a width change) is
/// reflected before windowing. Mirrors the GUI blocklist's overhang pass.
const OVERHANG_ROWS: usize = 20;

/// Stable identities used by TUI block-list viewport tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TuiBlockListViewportItemId {
    Terminal(BlockId),
    Agent(EntityId),
    CLISubagent(EntityId),
}

struct TuiBlockListVisibleItem {
    origin_y: usize,
    /// Full cached height from the canonical `BlockList`.
    height: usize,
    kind: TuiBlockListVisibleItemKind,
}

enum TuiBlockListVisibleItemKind {
    Terminal(BlockId),
    Agent(ViewHandle<TuiAIBlock>),
    CLISubagent(ViewHandle<TuiCLISubagentView>),
}

/// Adapts a terminal model's canonical block-list order for TUI viewporting.
pub(super) struct TuiBlockListViewportSource {
    model: Arc<FairMutex<TerminalModel>>,
    agent_blocks: AgentBlockRegistry,
    cli_subagent_blocks: CLISubagentBlockRegistry,
    height_changes: RefCell<Vec<TuiRowResize>>,
}

impl TuiBlockListViewportSource {
    /// Creates a TUI viewport source over the canonical terminal model.
    #[cfg(test)]
    pub(super) fn new(
        model: Arc<FairMutex<TerminalModel>>,
        agent_blocks: AgentBlockRegistry,
    ) -> Self {
        Self {
            model,
            agent_blocks,
            cli_subagent_blocks: Rc::new(RefCell::new(HashMap::new())),
            height_changes: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn new_with_cli_subagents(
        model: Arc<FairMutex<TerminalModel>>,
        agent_blocks: AgentBlockRegistry,
        cli_subagent_blocks: CLISubagentBlockRegistry,
    ) -> Self {
        Self {
            model,
            agent_blocks,
            cli_subagent_blocks,
            height_changes: RefCell::new(Vec::new()),
        }
    }

    /// Collects the agent-block view ids to measure this frame: the drained
    /// dirty set (measured wherever they sit) plus, from the viewport window
    /// padded by [`OVERHANG_ROWS`], the non-dirty agent blocks whose cached
    /// height could be stale.
    ///
    /// A non-dirty band block is re-measured only when its cached height cannot
    /// be trusted: its last measurement was at a different width (reflow), it
    /// has never been measured (no recorded width), or it is still streaming
    /// (its height can grow without a per-update invalidation — e.g. an
    /// expanded, still-running shell command). At a stable width with no
    /// dynamic height, nothing extra is measured and the cached
    /// `last_laid_out_height` is reused. Off-band blocks keep their cached
    /// height until they scroll into the band.
    fn agent_heights_to_measure(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        app: &AppContext,
    ) -> HashSet<EntityId> {
        let mut model = self.model.lock();
        let mut view_ids = model.block_list_mut().take_dirty_rich_content_items();

        let agent_blocks = self.agent_blocks.borrow();
        let cli_subagent_blocks = self.cli_subagent_blocks.borrow();
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
                    if !rich_content.should_hide {
                        if let Some(view) = agent_blocks.get(&rich_content.view_id) {
                            if view
                                .as_ref(app)
                                .needs_height_measurement(available_width, app)
                            {
                                view_ids.insert(rich_content.view_id);
                            }
                        } else if let Some(view) = cli_subagent_blocks.get(&rich_content.view_id) {
                            if view
                                .as_ref(app)
                                .needs_height_measurement(available_width, app)
                            {
                                view_ids.insert(rich_content.view_id);
                            }
                        }
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
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> HashMap<EntityId, BlockHeight> {
        let agent_blocks = self.agent_blocks.borrow();
        let cli_subagent_blocks = self.cli_subagent_blocks.borrow();
        view_ids
            .into_iter()
            .filter_map(|view_id| {
                let height = if let Some(view) = agent_blocks.get(&view_id) {
                    let view = view.as_ref(app);
                    let height = view.desired_height(width, ctx, app).max(1);
                    view.record_height_measurement(width);
                    height
                } else {
                    let view = cli_subagent_blocks.get(&view_id)?.as_ref(app);
                    let height = view.desired_height(width, ctx, app).max(1);
                    view.record_height_measurement(width);
                    height
                };
                Some((view_id, BlockHeight::from(height as f64)))
            })
            .collect()
    }

    /// Writes measured rich-content heights back to the canonical block list.
    fn write_line_heights(&self, line_heights: &HashMap<EntityId, BlockHeight>) {
        if line_heights.is_empty() {
            return;
        }
        self.height_changes
            .borrow_mut()
            .extend(self.rich_content_row_resizes(line_heights));
        self.model
            .lock()
            .block_list_mut()
            .update_rich_content_heights_in_lines(line_heights);
    }

    /// Collects one [`TuiRowResize`] per changed rich-content item, in canonical
    /// block-list order, computed against cached heights before the new ones
    /// are written back. The viewport reports them to the selectable wrapper,
    /// which rebases selected rows around content growth or shrinkage.
    fn rich_content_row_resizes(
        &self,
        line_heights: &HashMap<EntityId, BlockHeight>,
    ) -> Vec<TuiRowResize> {
        let model = self.model.lock();
        let block_list = model.block_list();
        let mut changes = line_heights
            .iter()
            .filter_map(|(view_id, height)| {
                let old_rows = block_list.rich_content_row_range(*view_id)?;
                let new_height = height.as_f64().ceil().max(0.0) as usize;
                (old_rows.len() != new_height).then_some(TuiRowResize {
                    old_rows,
                    new_height,
                })
            })
            .collect::<Vec<_>>();
        changes.sort_by_key(|resize| resize.old_rows.start);
        changes
    }

    fn visible_items_in_window(
        &self,
        window: TuiViewportWindow,
    ) -> (usize, Vec<TuiBlockListVisibleItem>) {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let cli_subagent_blocks = self.cli_subagent_blocks.borrow();
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
                            kind: TuiBlockListVisibleItemKind::Terminal(block.id().clone()),
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
                            kind: TuiBlockListVisibleItemKind::Agent(view.clone()),
                        })
                    } else if let Some(view) = cli_subagent_blocks.get(&item.view_id) {
                        let height = item.last_laid_out_height.as_f64().ceil().max(1.0) as usize;
                        Some(TuiBlockListVisibleItem {
                            origin_y: item_top,
                            height,
                            kind: TuiBlockListVisibleItemKind::CLISubagent(view.clone()),
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

    /// Returns viewport items without measuring or mutating cached heights.
    ///
    /// Two callers need this read-only path: `visible_items` calls it after
    /// it has already measured and written fresh heights for this frame, and
    /// `selection_content` calls it directly because selection scraping reads
    /// arbitrary row windows (often outside the rendered viewport) and must
    /// not dirty heights or emit resize events mid-gesture. Any path that
    /// needs up-to-date heights must measure first via `visible_items`.
    fn read_only_content(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
    ) -> TuiViewportContent {
        let (content_height, visible_items) = self.visible_items_in_window(window);
        let items = visible_items
            .into_iter()
            .map(|item| item.render(&self.model, window, available_width))
            .collect();
        TuiViewportContent {
            content_height,
            items,
        }
    }

    #[cfg(test)]
    pub(super) fn item_ids_for_test(&self) -> Vec<TuiBlockListViewportItemId> {
        let model = self.model.lock();
        let block_list = model.block_list();
        let agent_blocks = self.agent_blocks.borrow();
        let cli_subagent_blocks = self.cli_subagent_blocks.borrow();
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
                        item_ids.push(TuiBlockListViewportItemId::Terminal(block.id().clone()));
                    }
                }
                BlockHeightItem::RichContent(item)
                    if !item.should_hide && agent_blocks.contains_key(&item.view_id) =>
                {
                    item_ids.push(TuiBlockListViewportItemId::Agent(item.view_id));
                }
                BlockHeightItem::RichContent(item)
                    if !item.should_hide && cli_subagent_blocks.contains_key(&item.view_id) =>
                {
                    item_ids.push(TuiBlockListViewportItemId::CLISubagent(item.view_id));
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
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiViewportContent {
        // Refresh cached heights before windowing: the dirty set plus any band
        // agent blocks whose cached height is stale (see
        // `agent_heights_to_measure`).
        let view_ids_to_measure = self.agent_heights_to_measure(window, available_width, app);
        let heights = self.measured_agent_heights(view_ids_to_measure, available_width, ctx, app);
        self.write_line_heights(&heights);

        self.read_only_content(window, available_width)
    }

    fn selection_content(
        &self,
        window: TuiViewportWindow,
        available_width: u16,
        _app: &AppContext,
    ) -> Option<TuiViewportContent> {
        Some(self.read_only_content(window, available_width))
    }

    fn selection_logical_text(
        &self,
        selection: TuiSelectionSpan,
        available_width: u16,
        app: &AppContext,
    ) -> Option<String> {
        let end_row_exclusive = if selection.end.col == 0 {
            selection.end.row
        } else {
            selection.end.row.saturating_add(1)
        };
        // Source logical text only when the whole selection lands inside a
        // single agent block. Overlap with a second item, a terminal block, or
        // any other non-agent content returns `None`, keeping those selections
        // on the per-row grid path.
        let (block_top, view) = {
            let model = self.model.lock();
            let block_list = model.block_list();
            let agent_blocks = self.agent_blocks.borrow();
            let mut found: Option<(usize, ViewHandle<TuiAIBlock>)> = None;
            let mut cursor = block_list
                .block_heights()
                .cursor::<BlockHeight, BlockHeightSummary>();
            cursor.seek_clamped(
                &BlockHeight::from(selection.start.row as f64),
                SeekBias::Left,
            );
            while let Some(item) = cursor.item() {
                let item_top = cursor.start().height.as_f64().floor().max(0.0) as usize;
                if item_top >= end_row_exclusive {
                    break;
                }
                let item_bottom = item_top.saturating_add(item.height().as_f64().ceil() as usize);
                let overlaps = item_bottom > selection.start.row && item_top < end_row_exclusive;
                if overlaps {
                    match item {
                        BlockHeightItem::RichContent(rich) if !rich.should_hide => {
                            if found.is_some() {
                                return None;
                            }
                            let view = agent_blocks.get(&rich.view_id)?;
                            found = Some((item_top, view.clone()));
                        }
                        _ => return None,
                    }
                }
                cursor.next();
            }
            found?
        };
        view.as_ref(app)
            .selection_logical_text(selection, block_top, available_width, app)
    }

    fn take_selection_row_resizes(&self) -> Vec<TuiRowResize> {
        self.height_changes.borrow_mut().drain(..).collect()
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
    ) -> TuiVisibleViewportItem {
        let visible_rows = self.visible_rows(window);
        // Terminal blocks get pre-sliced below; rich content stays whole and lets `TuiClipped`
        // handle any partial visibility.
        let origin_y = match &self.kind {
            TuiBlockListVisibleItemKind::Terminal(_) => {
                self.origin_y.saturating_add(visible_rows.start)
            }
            TuiBlockListVisibleItemKind::Agent(_) | TuiBlockListVisibleItemKind::CLISubagent(_) => {
                self.origin_y
            }
        };
        TuiVisibleViewportItem {
            origin_y,
            element: self.render_element(model, visible_rows, available_width),
        }
    }

    fn render_element(
        self,
        model: &Arc<FairMutex<TerminalModel>>,
        visible_rows: Range<usize>,
        width: u16,
    ) -> Box<dyn TuiElement> {
        match self.kind {
            TuiBlockListVisibleItemKind::Terminal(block_id) => {
                debug_assert!(visible_rows.end <= self.height);
                TerminalBlockElement::visible_rows(model.clone(), block_id, visible_rows, width)
                    .finish()
            }
            TuiBlockListVisibleItemKind::Agent(view) => TuiChildView::new(&view).finish(),
            TuiBlockListVisibleItemKind::CLISubagent(view) => TuiChildView::new(&view).finish(),
        }
    }
}

#[cfg(test)]
#[path = "tui_block_list_viewport_source_tests.rs"]
mod tests;
