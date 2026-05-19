//! Async find implementation for terminal content.
//!
//! This module provides asynchronous find functionality that runs on a background thread,
//! streaming results back to the main thread to avoid blocking the UI.

mod background_task;
mod work_queue;

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::FairMutex;
use sum_tree::SeekBias;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{EntityId, ModelContext};

use crate::terminal::block_list_element::GridType;
use crate::terminal::find::model::TerminalFindModel;
use crate::terminal::model::blocks::{
    BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, TotalIndex,
};
use crate::terminal::model::grid::grid_handler::{AbsolutePoint, GridHandler};
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::{BlockIndex, BlockSortDirection};
use crate::terminal::model::TerminalModel;
use crate::throttle::throttle;
use crate::view_components::find::{FindDirection, FindEvent};

use super::rich_content::{FindableRichContentHandle, RichContentMatchId};
use super::FindOptions;

use background_task::spawn_find_task;
use work_queue::FindWorkQueue;

/// Status of an async find operation.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum AsyncFindStatus {
    /// No find operation in progress.
    #[default]
    Idle,
    /// Find operation is running.
    Scanning,
    /// Find operation completed.
    Complete,
}

impl std::fmt::Display for AsyncFindStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Scanning => write!(f, "Scanning"),
            Self::Complete => write!(f, "Complete"),
        }
    }
}

/// Message streamed from background find task to main thread.
#[derive(Debug)]
pub enum FindTaskMessage {
    /// Matches found in a terminal block's grid.
    BlockGridMatches {
        block_index: BlockIndex,
        grid_type: GridType,
        matches: Vec<AbsoluteMatch>,
    },
    /// Matches found in a dirty range within a terminal block's grid.
    ///
    /// Unlike `BlockGridMatches` (which extends results), these matches are
    /// merged into existing results using `update_dirty_matches`.
    DirtyRangeMatches {
        block_index: BlockIndex,
        grid_type: GridType,
        /// The dirty range in absolute row coordinates, used for merging.
        dirty_range: RangeInclusive<u64>,
        matches: Vec<AbsoluteMatch>,
    },
    /// Request to scan an AI block on the main thread.
    ScanAIBlock {
        view_id: EntityId,
        total_index: TotalIndex,
    },
    /// The work queue has drained (current batch of work is complete).
    Done,
}

/// Configuration for an async find run.
#[derive(Debug, Clone)]
pub struct AsyncFindConfig {
    /// The search query.
    pub query: Arc<String>,
    /// Whether the search is case-sensitive.
    pub is_case_sensitive: bool,
    /// Whether the query should be interpreted as a regex.
    pub is_regex_enabled: bool,
    /// If `Some`, only search within these block indices.
    pub blocks_to_include: Option<Vec<BlockIndex>>,
    /// The sort direction of blocks in the view.
    pub block_sort_direction: BlockSortDirection,
}

impl AsyncFindConfig {
    /// Creates a new config from FindOptions and a block sort direction.
    ///
    /// Returns `None` if there is no query or if the query is empty/whitespace.
    pub fn from_options(
        options: &FindOptions,
        block_sort_direction: BlockSortDirection,
    ) -> Option<Self> {
        let query = options.query.clone()?;
        if query.trim().is_empty() {
            return None;
        }
        Some(Self {
            query,
            is_case_sensitive: options.is_case_sensitive,
            is_regex_enabled: options.is_regex_enabled,
            blocks_to_include: options.blocks_to_include_in_results.clone(),
            block_sort_direction,
        })
    }
}

/// A match stored with absolute row indices to handle scrollback truncation.
///
/// Using absolute indices (offset from original row 0) allows us to:
/// 1. Avoid updating all match indices when rows are truncated
/// 2. Efficiently filter out truncated matches at query time
/// 3. Support incremental dirty-range scanning without full rescans
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsoluteMatch {
    /// Start point with absolute row index.
    pub start: AbsolutePoint,
    /// End point with absolute row index.
    pub end: AbsolutePoint,
}

impl AbsoluteMatch {
    /// Creates an AbsoluteMatch from a relative Point range.
    pub fn from_range(range: &RangeInclusive<Point>, grid: &GridHandler) -> Self {
        Self {
            start: AbsolutePoint::from_point(*range.start(), grid),
            end: AbsolutePoint::from_point(*range.end(), grid),
        }
    }

    /// Converts back to a relative Point range.
    ///
    /// Returns `None` if either point has been truncated from scrollback.
    pub fn to_range(&self, grid: &GridHandler) -> Option<RangeInclusive<Point>> {
        let start = self.start.to_point(grid)?;
        let end = self.end.to_point(grid)?;
        Some(start..=end)
    }

    /// Returns true if this match has been truncated from scrollback.
    pub fn is_truncated(&self, num_lines_truncated: u64) -> bool {
        // A match is truncated if its start point is truncated.
        self.start.is_truncated(num_lines_truncated)
    }

    /// Returns the absolute start row.
    pub fn start_row(&self) -> u64 {
        self.start.row
    }

    /// Returns the absolute end row.
    pub fn end_row(&self) -> u64 {
        self.end.row
    }
}

impl PartialOrd for AbsoluteMatch {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AbsoluteMatch {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Sort by end point (for ascending order iteration during rendering),
        // then by start point for consistency with derived PartialEq.
        //
        // This ordering assumes matches within a single grid do not overlap,
        // which is guaranteed by `find_in_range` producing non-overlapping
        // results. `update_dirty_matches` relies on this property to use
        // positional range-based splicing for merging.
        self.end.cmp(&other.end).then(self.start.cmp(&other.start))
    }
}

/// A focused match from the async find controller.
///
/// Similar to `BlockGridMatch` but uses `AbsoluteMatch` for the range, allowing
/// the caller to convert to relative coordinates when needed.
#[derive(Debug, Clone)]
pub struct AsyncBlockGridMatch {
    /// The type of grid in which the match was found.
    pub grid_type: GridType,
    /// The match range in absolute coordinates.
    pub range: AbsoluteMatch,
    /// The index of the containing block.
    pub block_index: BlockIndex,
}

/// Per-block find results, keyed by block index.
#[derive(Debug, Default)]
pub(crate) struct BlockFindResults {
    /// Matches for terminal blocks, keyed by (block_index, grid_type).
    /// Matches are stored in ascending order by end point.
    pub(crate) terminal_matches: HashMap<(BlockIndex, GridType), Vec<AbsoluteMatch>>,
    /// Matches for AI blocks, keyed by view_id.
    pub(crate) ai_matches: HashMap<EntityId, Vec<RichContentMatchId>>,
    /// TotalIndex for each terminal block that has been scanned.
    terminal_total_indices: HashMap<BlockIndex, TotalIndex>,
    /// TotalIndex for each AI block that has been scanned.
    ai_total_indices: HashMap<EntityId, TotalIndex>,
}

impl BlockFindResults {
    /// Returns the total number of matches across all blocks.
    fn total_match_count(&self) -> usize {
        let terminal_count: usize = self.terminal_matches.values().map(|v| v.len()).sum();
        let ai_count: usize = self.ai_matches.values().map(|v| v.len()).sum();
        terminal_count + ai_count
    }

    /// Clears all results.
    fn clear(&mut self) {
        self.terminal_matches.clear();
        self.ai_matches.clear();
        self.terminal_total_indices.clear();
        self.ai_total_indices.clear();
    }

    /// Removes all results for a specific block index.
    fn remove_block(&mut self, block_index: BlockIndex) {
        self.terminal_matches
            .retain(|(idx, _), _| *idx != block_index);
        self.terminal_total_indices.remove(&block_index);
    }

    /// Updates matches for a dirty range within a specific block grid.
    ///
    /// This removes all existing matches that overlap with the dirty range and
    /// inserts the new matches in their place, maintaining ascending order.
    ///
    /// Adapted from `FilterState::update_dirty_matches` in filtering.rs.
    fn update_dirty_matches(
        &mut self,
        block_index: BlockIndex,
        grid_type: GridType,
        dirty_range: RangeInclusive<u64>,
        new_matches: Vec<AbsoluteMatch>,
    ) {
        let matches = self
            .terminal_matches
            .entry((block_index, grid_type))
            .or_default();

        // If there are no current matches, just insert the new ones.
        if matches.is_empty() {
            *matches = new_matches;
            return;
        }

        // If the dirty range is before all existing matches, insert at the start.
        if matches
            .first()
            .is_some_and(|first_match| *dirty_range.end() < first_match.start_row())
        {
            matches.splice(0..0, new_matches);
            return;
        }

        // If the dirty range is after all existing matches, append at the end.
        if matches
            .last()
            .is_some_and(|last_match| last_match.end_row() < *dirty_range.start())
        {
            matches.extend(new_matches);
            return;
        }

        // Find the range of matches that overlap with the dirty range.
        // A match overlaps if: match.start <= dirty.end AND match.end >= dirty.start
        let replace_start = matches
            .iter()
            .position(|m| m.end_row() >= *dirty_range.start())
            .unwrap_or(matches.len());

        let replace_end = matches
            .iter()
            .rposition(|m| m.start_row() <= *dirty_range.end())
            .map(|pos| pos + 1)
            .unwrap_or(replace_start);

        let replace_range = if replace_start <= replace_end {
            replace_start..replace_end
        } else {
            // Dirty range lies between two adjacent matches; insert without replacing.
            replace_start..replace_start
        };

        matches.splice(replace_range, new_matches);

        // Assert that matches are still in ascending order.
        debug_assert!(
            matches.windows(2).all(|w| w[0] <= w[1]),
            "Matches should be in ascending order after update_dirty_matches"
        );
    }
}

/// Information about a block to be searched.
#[derive(Debug, Clone)]
pub enum BlockInfo {
    /// A terminal command block.
    Terminal {
        block_index: BlockIndex,
        total_index: TotalIndex,
    },
    /// A rich content block (e.g., AI block).
    RichContent {
        view_id: EntityId,
        total_index: TotalIndex,
    },
}

/// Controller for async find operations.
///
/// This model manages background find tasks, processes results, and maintains
/// find state for the terminal view.
pub struct AsyncFindController {
    /// Reference to the terminal model.
    terminal_model: Arc<FairMutex<TerminalModel>>,

    /// Current find configuration, if any.
    current_config: Option<AsyncFindConfig>,

    /// Per-block results for the current find run.
    block_results: BlockFindResults,

    /// Current status of the find operation.
    status: AsyncFindStatus,

    /// Sender for the result channel. The receiver is consumed by
    /// `spawn_stream_local` in `start_find`, so only the sender is stored
    /// here (to be cloned into each background task via `spawn_find_task`).
    result_tx: Option<async_channel::Sender<FindTaskMessage>>,

    /// Sender for the throttled UI-update channel. `process_message` sends
    /// a `()` signal here; a throttled stream on the other end emits
    /// `FindEvent::RanFind` at most every 50 ms.
    throttle_tx: Option<async_channel::Sender<()>>,

    /// Handle to abort the background task's future.
    task_handle: Option<SpawnedFutureHandle>,

    /// Shared work queue for the background task.
    work_queue: Option<FindWorkQueue>,

    /// Rich content views for AI block searching.
    rich_content_views: HashMap<EntityId, Box<dyn FindableRichContentHandle>>,

    /// The block sort direction for the current/last find run.
    block_sort_direction: BlockSortDirection,

    /// The currently focused match index (0-based), if any.
    focused_match_index: Option<usize>,

    /// The FindOptions for the current find run, stored for `active_find_options()` access.
    current_find_options: Option<FindOptions>,

    /// Cached result of `focused_terminal_match()`, updated when focus or matches change.
    /// Avoids re-sorting HashMap keys and iterating on every call.
    cached_focused_match: Option<AsyncBlockGridMatch>,

    /// Monotonically increasing generation counter, bumped each time new streams
    /// are spawned. The result stream callback captures the generation at spawn
    /// time and skips messages that arrive after a newer generation has started,
    /// preventing stale `Done` messages from prematurely ending a new scan.
    generation: u64,
}

impl AsyncFindController {
    /// Creates a new AsyncFindController.
    pub fn new(terminal_model: Arc<FairMutex<TerminalModel>>) -> Self {
        Self {
            terminal_model,
            current_config: None,
            block_results: BlockFindResults::default(),
            status: AsyncFindStatus::Idle,
            result_tx: None,
            throttle_tx: None,
            task_handle: None,
            work_queue: None,
            rich_content_views: HashMap::new(),
            block_sort_direction: BlockSortDirection::MostRecentLast,
            focused_match_index: None,
            current_find_options: None,
            cached_focused_match: None,
            generation: 0,
        }
    }

    /// Returns the current find status.
    pub fn status(&self) -> &AsyncFindStatus {
        &self.status
    }

    /// Returns the current config, if any.
    pub fn current_config(&self) -> Option<&AsyncFindConfig> {
        self.current_config.as_ref()
    }

    /// Returns the FindOptions for the current find run, if any.
    pub fn find_options(&self) -> Option<&FindOptions> {
        self.current_find_options.as_ref()
    }

    /// Returns the total number of matches found so far.
    pub fn match_count(&self) -> usize {
        self.block_results.total_match_count()
    }

    /// Returns true if a find operation is currently in progress.
    pub fn is_scanning(&self) -> bool {
        matches!(self.status, AsyncFindStatus::Scanning)
    }

    /// Returns true if there is an active find configuration.
    ///
    /// This indicates that find is active and new blocks should be scanned.
    pub fn has_active_find(&self) -> bool {
        self.current_config.is_some()
    }

    /// Returns the currently focused match index (0-based), if any.
    pub fn focused_match_index(&self) -> Option<usize> {
        self.focused_match_index
    }

    /// Focuses the next or previous match based on the given direction.
    ///
    /// Since matches are stored newest-first (index 0 = most recent block),
    /// the mapping between UI direction and index direction depends on the
    /// block sort direction:
    /// - MostRecentLast: Down decrements (toward newest), Up increments (toward oldest).
    /// - MostRecentFirst: Down increments (toward oldest), Up decrements (toward newest).
    pub fn focus_next_match(&mut self, direction: FindDirection) {
        let total = self.match_count();
        if total == 0 {
            self.focused_match_index = None;
            return;
        }

        let new_index = match (self.focused_match_index, direction) {
            (None, _) => 0,
            (Some(current), direction) => {
                // Determine whether this direction means "toward lower indices"
                // (newest/bottom for MostRecentLast, newest/top for MostRecentFirst).
                let decrement = matches!(
                    (direction, self.block_sort_direction),
                    (FindDirection::Down, BlockSortDirection::MostRecentLast)
                        | (FindDirection::Up, BlockSortDirection::MostRecentFirst)
                );
                if decrement {
                    if current == 0 {
                        total - 1 // Wrap around.
                    } else {
                        current - 1
                    }
                } else if current + 1 >= total {
                    0 // Wrap around.
                } else {
                    current + 1
                }
            }
        };

        self.focused_match_index = Some(new_index);
        self.update_cached_focused_match();
    }

    /// Returns the focused match as an AsyncBlockGridMatch if it's a terminal match.
    ///
    /// Returns a cached value that is updated when focus or matches change,
    /// avoiding the cost of sorting and iterating on every call.
    pub fn focused_terminal_match(&self) -> Option<AsyncBlockGridMatch> {
        self.cached_focused_match.clone()
    }

    /// Recomputes the cached focused match from the current matches and focus index.
    fn update_cached_focused_match(&mut self) {
        self.cached_focused_match = self.compute_focused_terminal_match();
    }

    /// Computes the focused terminal match by iterating through all matches
    /// (terminal and AI) in visual display order, derived from the TotalIndex
    /// maps stored in the block results.
    ///
    /// Returns `Some` if the focused index lands on a terminal match, `None`
    /// if it lands on an AI match or is out of range.
    fn compute_focused_terminal_match(&self) -> Option<AsyncBlockGridMatch> {
        let focused_idx = self.focused_match_index?;
        let mut current_idx = 0;

        // Determine grid iteration order within each terminal block.
        let grid_types: [GridType; 2] = match self.block_sort_direction {
            BlockSortDirection::MostRecentFirst => [GridType::PromptAndCommand, GridType::Output],
            BlockSortDirection::MostRecentLast => [GridType::Output, GridType::PromptAndCommand],
        };

        // Build a unified list of all blocks with results, sorted by TotalIndex.
        let mut ordered_blocks: Vec<(TotalIndex, BlockInfo)> = Vec::new();
        for (&block_index, &total_index) in &self.block_results.terminal_total_indices {
            ordered_blocks.push((
                total_index,
                BlockInfo::Terminal {
                    block_index,
                    total_index,
                },
            ));
        }
        for (&view_id, &total_index) in &self.block_results.ai_total_indices {
            ordered_blocks.push((
                total_index,
                BlockInfo::RichContent {
                    view_id,
                    total_index,
                },
            ));
        }

        // Sort by TotalIndex descending so that matches closest to the end
        // of the blocklist (newest blocks, near the prompt) come first.
        ordered_blocks.sort_by(|a, b| b.0.cmp(&a.0));

        for (_, block_info) in &ordered_blocks {
            match block_info {
                BlockInfo::Terminal { block_index, .. } => {
                    let reverse_within_grid = matches!(
                        self.block_sort_direction,
                        BlockSortDirection::MostRecentLast
                    );
                    for &grid_type in &grid_types {
                        if let Some(matches) = self
                            .block_results
                            .terminal_matches
                            .get(&(*block_index, grid_type))
                        {
                            // For MostRecentLast, sync focus traversal
                            // iterates from the bottom of each grid first.
                            let iter: Box<dyn Iterator<Item = &AbsoluteMatch>> =
                                if reverse_within_grid {
                                    Box::new(matches.iter().rev())
                                } else {
                                    Box::new(matches.iter())
                                };
                            for match_range in iter {
                                if current_idx == focused_idx {
                                    return Some(AsyncBlockGridMatch {
                                        block_index: *block_index,
                                        grid_type,
                                        range: match_range.clone(),
                                    });
                                }
                                current_idx += 1;
                            }
                        }
                    }
                }
                BlockInfo::RichContent { view_id, .. } => {
                    // Count AI matches so the index arithmetic stays correct,
                    // but don't return them as terminal matches.
                    if let Some(ai_matches) = self.block_results.ai_matches.get(view_id) {
                        current_idx += ai_matches.len();
                    }
                }
            }
        }

        None
    }

    /// Registers a rich content view for AI block searching.
    pub(crate) fn register_rich_content_view(
        &mut self,
        view_id: EntityId,
        handle: Box<dyn FindableRichContentHandle>,
    ) {
        self.rich_content_views.insert(view_id, handle);
    }

    /// Unregisters a rich content view.
    pub(crate) fn unregister_rich_content_view(&mut self, view_id: EntityId) {
        self.rich_content_views.remove(&view_id);
    }

    /// Starts a new find operation with the given options.
    ///
    /// If a find operation is already in progress, it will be cancelled first.
    /// This spawns a background task, a result stream, and a throttled UI-update
    /// stream on the provided context.
    pub fn start_find(
        &mut self,
        options: &FindOptions,
        block_sort_direction: BlockSortDirection,
        ctx: &mut ModelContext<TerminalFindModel>,
    ) {
        // Cancel any existing find operation.
        self.cancel_current_find();

        // Check for query refinement optimization.
        if let (Some(current_config), Some(new_query)) =
            (&self.current_config, options.query.as_ref())
        {
            if !options.is_regex_enabled
                && !current_config.is_regex_enabled
                && options.is_case_sensitive == current_config.is_case_sensitive
                && is_query_refinement(&current_config.query, new_query)
            {
                // New query is a refinement of the old query — filter existing results.
                self.filter_results_for_refinement(options, block_sort_direction, ctx);
                return;
            }
        }

        // Create new config.
        let Some(config) = AsyncFindConfig::from_options(options, block_sort_direction) else {
            // No query — clear results and return.
            self.clear_results(ctx);
            return;
        };

        self.current_config = Some(config.clone());
        self.block_sort_direction = block_sort_direction;
        self.block_results.clear();
        self.focused_match_index = None;
        self.cached_focused_match = None;
        self.current_find_options = Some(options.clone());
        self.status = AsyncFindStatus::Scanning;

        // Build the work queue from the current block list.
        let queue = FindWorkQueue::new();
        let block_info = {
            let mut model = self.terminal_model.lock();
            let info = collect_block_info(model.block_list(), &config);
            // Clear stale dirty ranges on the active block, since the full scan
            // covers all of its current content. Without this, the first
            // incremental update could redundantly re-scan already-covered rows.
            if let Some(output_grid) = model
                .block_list_mut()
                .active_block_mut()
                .grid_of_type_mut(GridType::Output)
            {
                output_grid.grid_handler_mut().take_find_dirty_rows_range();
            }
            info
        };
        queue.enqueue_full_scan(&block_info);
        self.work_queue = Some(queue.clone());

        // Populate TotalIndex maps from the block info so that
        // compute_focused_terminal_match can order results correctly.
        for info in &block_info {
            match info {
                BlockInfo::Terminal {
                    block_index,
                    total_index,
                } => {
                    self.block_results
                        .terminal_total_indices
                        .insert(*block_index, *total_index);
                }
                BlockInfo::RichContent {
                    view_id,
                    total_index,
                } => {
                    self.block_results
                        .ai_total_indices
                        .insert(*view_id, *total_index);
                }
            }
        }

        // Create result channel and spawn streams.
        let (result_tx, result_rx) = async_channel::unbounded();
        self.result_tx = Some(result_tx.clone());
        self.spawn_result_and_throttle_streams(result_rx, ctx);

        // Spawn background task.
        self.task_handle = Some(spawn_find_task(
            config,
            self.terminal_model.clone(),
            queue,
            result_tx,
            ctx,
        ));
    }

    /// Processes a single message from the background find task.
    ///
    /// Called by the result stream's `on_item` callback for each message
    /// delivered from the background task.
    pub fn process_message(
        &mut self,
        msg: FindTaskMessage,
        ctx: &mut ModelContext<TerminalFindModel>,
    ) {
        match msg {
            FindTaskMessage::BlockGridMatches {
                block_index,
                grid_type,
                matches,
            } => {
                if !matches.is_empty() {
                    // Store TotalIndex for this block if not already known
                    // (e.g. the block was added after the initial scan).
                    if !self
                        .block_results
                        .terminal_total_indices
                        .contains_key(&block_index)
                    {
                        let total_index = {
                            let model = self.terminal_model.lock();
                            total_index_for_block(block_index, model.block_list())
                        };
                        self.block_results
                            .terminal_total_indices
                            .insert(block_index, total_index);
                    }

                    self.block_results
                        .terminal_matches
                        .entry((block_index, grid_type))
                        .or_default()
                        .extend(matches);

                    // Auto-select the first match when results first arrive.
                    if self.focused_match_index.is_none() {
                        self.focused_match_index = Some(0);
                    }

                    self.clamp_focused_match_index();
                }
            }
            FindTaskMessage::DirtyRangeMatches {
                block_index,
                grid_type,
                dirty_range,
                matches,
            } => {
                self.block_results.update_dirty_matches(
                    block_index,
                    grid_type,
                    dirty_range,
                    matches,
                );
                // Prune matches that have been truncated from scrollback.
                // Dirty range messages arrive when the active block receives
                // new output, which is exactly when truncation can occur.
                self.prune_truncated_matches(block_index, grid_type);
                self.clamp_focused_match_index();
            }
            FindTaskMessage::ScanAIBlock {
                view_id,
                total_index,
            } => {
                // Scan AI block on main thread.
                if let Some(view) = self.rich_content_views.get(&view_id) {
                    if let Some(config) = &self.current_config {
                        let options = FindOptions {
                            query: Some(config.query.clone()),
                            is_case_sensitive: config.is_case_sensitive,
                            is_regex_enabled: config.is_regex_enabled,
                            blocks_to_include_in_results: None,
                        };
                        let start = instant::Instant::now();
                        let match_ids = view.run_find(&options, ctx);
                        let elapsed = start.elapsed();
                        log::trace!(
                            "[async_find] AI block scan took {}ms for view_id={:?}",
                            elapsed.as_millis(),
                            view_id
                        );
                        if !match_ids.is_empty() {
                            self.block_results.ai_matches.insert(view_id, match_ids);
                            self.block_results
                                .ai_total_indices
                                .insert(view_id, total_index);
                            self.clamp_focused_match_index();
                        }
                    }
                }
            }
            FindTaskMessage::Done => {
                self.status = AsyncFindStatus::Complete;
            }
        }

        // Signal the throttled UI-update stream.
        if let Some(tx) = &self.throttle_tx {
            let _ = tx.try_send(());
        }
    }

    /// Cancels the current find operation, if any.
    ///
    /// Dropping the senders closes the result and throttle streams naturally.
    /// The find configuration is preserved so `has_active_find()` remains true.
    pub fn cancel_current_find(&mut self) {
        // Close the work queue, which causes the background task's pop() to
        // return Err(QueueClosed) and exit.
        if let Some(queue) = self.work_queue.take() {
            queue.close();
        }

        // Abort the background future so it stops being polled.
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }

        // Drop senders to close the result and throttle streams.
        self.result_tx = None;
        self.throttle_tx = None;
        self.status = AsyncFindStatus::Idle;
    }

    /// Clears all find results and resets state.
    pub fn clear_results(&mut self, ctx: &mut ModelContext<TerminalFindModel>) {
        self.cancel_current_find();
        self.current_config = None;
        self.block_results.clear();
        self.focused_match_index = None;
        self.cached_focused_match = None;
        self.status = AsyncFindStatus::Idle;
        self.current_find_options = None;

        // Clear matches in AI blocks.
        for view in self.rich_content_views.values() {
            view.clear_matches(ctx);
        }

        // Note: The owning entity is responsible for emitting events to notify listeners.
    }

    /// Invalidates results for a specific block and rescans it.
    ///
    /// This enqueues work into the shared queue so the background task handles
    /// it asynchronously. No scanning happens on the main thread.
    ///
    /// # Arguments
    /// * `block_index` - The index of the block that changed.
    /// * `dirty_info` - If provided, a `(row_range, grid_type, num_lines_truncated)`
    ///   tuple describing the dirty region. If `None`, a full block rescan is enqueued.
    pub fn invalidate_block(
        &mut self,
        block_index: BlockIndex,
        dirty_info: Option<(RangeInclusive<usize>, GridType, u64)>,
    ) {
        if self.current_config.is_none() {
            return;
        }

        let Some(queue) = self.work_queue.clone() else {
            return;
        };

        // For a full block rescan (no dirty range), clear existing results now
        // so stale matches are not shown while the rescan is pending.
        if dirty_info.is_none() {
            self.block_results.remove_block(block_index);
            self.clamp_focused_match_index();
        }

        log::trace!(
            "[async_find] invalidate_block: enqueuing work for block {:?}, dirty={:?}",
            block_index,
            dirty_info.as_ref().map(|(r, _, _)| r),
        );

        queue.invalidate_block(block_index, dirty_info);

        // Mark status as scanning so the polling loop stays alive while the
        // background task processes the new work.
        self.status = AsyncFindStatus::Scanning;
    }

    /// Returns matches for a specific terminal block grid as AbsoluteMatch references.
    ///
    /// Callers should convert to relative `Point` ranges using `AbsoluteMatch::to_range()`.
    pub fn matches_for_block_grid(
        &self,
        block_index: BlockIndex,
        grid_type: GridType,
    ) -> Option<&Vec<AbsoluteMatch>> {
        self.block_results
            .terminal_matches
            .get(&(block_index, grid_type))
    }

    /// Returns matches for a specific AI block.
    pub fn matches_for_ai_block(&self, view_id: EntityId) -> Option<&Vec<RichContentMatchId>> {
        self.block_results.ai_matches.get(&view_id)
    }

    /// Prunes truncated matches for a specific block and grid type.
    ///
    /// This removes matches whose start row has been truncated from scrollback,
    /// keeping the match count and focused index accurate.
    fn prune_truncated_matches(&mut self, block_index: BlockIndex, grid_type: GridType) {
        let num_lines_truncated = {
            let model = self.terminal_model.lock();
            let Some(block) = model.block_list().block_at(block_index) else {
                return;
            };
            match grid_type {
                GridType::Output => block.output_grid().grid_handler().num_lines_truncated(),
                GridType::PromptAndCommand => block
                    .prompt_and_command_grid()
                    .grid_handler()
                    .num_lines_truncated(),
                _ => return,
            }
        };

        if num_lines_truncated == 0 {
            return;
        }

        if let Some(matches) = self
            .block_results
            .terminal_matches
            .get_mut(&(block_index, grid_type))
        {
            matches.retain(|m| !m.is_truncated(num_lines_truncated));
        }
    }

    /// Clamps the focused match index to the current match count.
    ///
    /// If the count is 0, sets the index to `None`. This should be called after
    /// any operation that can reduce the total match count (e.g. dirty range
    /// updates, block removal).
    fn clamp_focused_match_index(&mut self) {
        let total = self.match_count();
        if total == 0 {
            self.focused_match_index = None;
        } else {
            self.focused_match_index = self.focused_match_index.map(|i| i.min(total - 1));
        }
        self.update_cached_focused_match();
    }

    /// Filters existing results for a query refinement.
    fn filter_results_for_refinement(
        &mut self,
        options: &FindOptions,
        block_sort_direction: BlockSortDirection,
        ctx: &mut ModelContext<TerminalFindModel>,
    ) {
        // For now, we do a full rescan when the query is refined.
        // A future optimization could filter existing matches without rescanning,
        // and update the query for pending queue items.

        // Create new config.
        let Some(config) = AsyncFindConfig::from_options(options, block_sort_direction) else {
            self.clear_results(ctx);
            return;
        };

        // Cancel existing operation and clear results, but keep the task alive
        // by re-using start_find which handles everything.
        self.cancel_current_find();

        self.current_config = Some(config.clone());
        self.block_sort_direction = block_sort_direction;
        self.block_results.clear();
        self.focused_match_index = None;
        self.cached_focused_match = None;
        self.current_find_options = Some(options.clone());
        self.status = AsyncFindStatus::Scanning;

        // Build the work queue from the current block list.
        let queue = FindWorkQueue::new();
        let block_info = {
            let mut model = self.terminal_model.lock();
            let info = collect_block_info(model.block_list(), &config);
            // Clear stale dirty ranges (same rationale as in start_find).
            if let Some(output_grid) = model
                .block_list_mut()
                .active_block_mut()
                .grid_of_type_mut(GridType::Output)
            {
                output_grid.grid_handler_mut().take_find_dirty_rows_range();
            }
            info
        };
        queue.enqueue_full_scan(&block_info);
        self.work_queue = Some(queue.clone());

        // Populate TotalIndex maps from the block info.
        for info in &block_info {
            match info {
                BlockInfo::Terminal {
                    block_index,
                    total_index,
                } => {
                    self.block_results
                        .terminal_total_indices
                        .insert(*block_index, *total_index);
                }
                BlockInfo::RichContent {
                    view_id,
                    total_index,
                } => {
                    self.block_results
                        .ai_total_indices
                        .insert(*view_id, *total_index);
                }
            }
        }

        // Create result channel and spawn streams.
        let (result_tx, result_rx) = async_channel::unbounded();
        self.result_tx = Some(result_tx.clone());
        self.spawn_result_and_throttle_streams(result_rx, ctx);

        // Spawn background task.
        self.task_handle = Some(spawn_find_task(
            config,
            self.terminal_model.clone(),
            queue,
            result_tx,
            ctx,
        ));
    }

    /// Spawns the result delivery stream and the throttled UI-update stream.
    ///
    /// The result stream invokes `process_message` for every `FindTaskMessage`
    /// received from the background task. The throttle stream coalesces rapid
    /// signals and emits `FindEvent::RanFind` at most every 50 ms (with the
    /// first signal passing through immediately).
    fn spawn_result_and_throttle_streams(
        &mut self,
        result_rx: async_channel::Receiver<FindTaskMessage>,
        ctx: &mut ModelContext<TerminalFindModel>,
    ) {
        const THROTTLE_INTERVAL: Duration = Duration::from_millis(50);

        // Bump generation so that stale messages from an old find's stream
        // are discarded by the callback check below.
        self.generation += 1;
        let generation = self.generation;

        // Result stream: delivers every message to process_message.
        ctx.spawn_stream_local(
            result_rx,
            move |me, msg, ctx| {
                if let Some(controller) = &mut me.async_find_controller {
                    if controller.generation == generation {
                        controller.process_message(msg, ctx);
                    }
                }
            },
            |_me, _ctx| {},
        );

        // Throttle stream: coalesces rapid signals into periodic UI updates.
        let (throttle_tx, throttle_rx) = async_channel::unbounded();
        self.throttle_tx = Some(throttle_tx);

        ctx.spawn_stream_local(
            throttle(THROTTLE_INTERVAL, throttle_rx),
            |_me, (), ctx| {
                ctx.emit(FindEvent::RanFind);
            },
            |_me, _ctx| {},
        );
    }
}

#[cfg(test)]
impl AsyncFindController {
    /// Sets up internal state for testing by setting the status directly.
    pub(crate) fn set_test_status(&mut self, status: AsyncFindStatus) {
        self.status = status;
    }

    /// Returns a mutable reference to block results for testing.
    pub(crate) fn block_results_mut(&mut self) -> &mut BlockFindResults {
        &mut self.block_results
    }
}

/// Returns true if `new_query` is a refinement of `old_query`.
///
/// A query is considered a refinement if it starts with the old query,
/// meaning any match of the new query must also be a match of the old query.
/// An empty old_query is not considered a valid refinement base.
fn is_query_refinement(old_query: &str, new_query: &str) -> bool {
    !old_query.is_empty() && new_query.starts_with(old_query) && new_query.len() > old_query.len()
}

/// Collects information about blocks to search.
///
/// The returned list is always in newest-first order, regardless of
/// `config.block_sort_direction`. This gives the background task a
/// consistent iteration order; the main thread re-sorts results by
/// `block_sort_direction` when computing the focused match.
pub fn collect_block_info(block_list: &BlockList, config: &AsyncFindConfig) -> Vec<BlockInfo> {
    let mut block_info = Vec::new();

    // If specific blocks are requested, only collect those.
    if let Some(blocks_to_include) = &config.blocks_to_include {
        for &block_index in blocks_to_include {
            if block_list.block_at(block_index).is_some() {
                let total_index = total_index_for_block(block_index, block_list);
                block_info.push(BlockInfo::Terminal {
                    block_index,
                    total_index,
                });
            }
        }
        // Sort by recency (newest first).
        block_info.sort_by(|a, b| {
            let idx_a = match a {
                BlockInfo::Terminal { block_index, .. } => block_index.0,
                BlockInfo::RichContent { .. } => 0,
            };
            let idx_b = match b {
                BlockInfo::Terminal { block_index, .. } => block_index.0,
                BlockInfo::RichContent { .. } => 0,
            };
            idx_b.cmp(&idx_a)
        });
        return block_info;
    }

    // Otherwise, iterate through all blocks via the height cursor.
    let mut cursor = block_list
        .block_heights()
        .cursor::<BlockHeight, BlockHeightSummary>();
    cursor.descend_to_last_item(block_list.block_heights());

    while let Some(item) = cursor.item() {
        match item {
            BlockHeightItem::Block(height)
                if height.into_lines() > warpui::units::Lines::zero() =>
            {
                let summary = cursor.start();
                block_info.push(BlockInfo::Terminal {
                    block_index: summary.block_count.into(),
                    total_index: summary.total_count.into(),
                });
            }
            BlockHeightItem::RichContent(rich_content_item)
                if rich_content_item.last_laid_out_height.into_lines()
                    > warpui::units::Lines::zero() =>
            {
                block_info.push(BlockInfo::RichContent {
                    view_id: rich_content_item.view_id,
                    total_index: cursor.start().total_count.into(),
                });
            }
            _ => {}
        }
        cursor.prev();
    }

    block_info
}

/// Computes the TotalIndex for a terminal block at the given BlockIndex.
///
/// Uses the block list's height tree to find the block's position among
/// all items (blocks, gaps, rich content, etc.).
fn total_index_for_block(block_index: BlockIndex, block_list: &BlockList) -> TotalIndex {
    let mut cursor = block_list.block_heights().cursor::<BlockIndex, ()>();
    TotalIndex(
        cursor
            .slice(&block_index, SeekBias::Right)
            .summary()
            .total_count,
    )
}

#[cfg(test)]
#[path = "async_find_tests.rs"]
mod tests;
