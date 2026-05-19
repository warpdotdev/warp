//! Background task for async find operations.
//!
//! This module contains the logic that runs on a background thread to scan
//! terminal blocks for matches without blocking the main thread. The task
//! pulls work items from a shared [`FindWorkQueue`] and streams results
//! back via an `async_channel`.

use std::ops::RangeInclusive;
use std::sync::Arc;

use futures_lite::future::yield_now;
use instant::Instant;
use parking_lot::FairMutex;
use warp_terminal::model::grid::Dimensions;
use warpui::ModelContext;

use warpui::Entity;

use crate::terminal::block_list_element::GridType;
use crate::terminal::model::find::{FindConfig, RegexDFAs};
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model::TerminalModel;

use super::work_queue::{FindWorkItem, FindWorkQueue};
use super::{AbsoluteMatch, AsyncFindConfig, FindTaskMessage};

/// Maximum time (in milliseconds) to hold the terminal model lock during a find chunk.
const MAX_LOCK_DURATION_MS: u64 = 5;

/// Number of rows to scan per chunk within a terminal block.
const ROWS_PER_CHUNK: usize = 1000;

/// Spawns a background find task that pulls work from the given queue.
///
/// Returns a handle that can be used to abort the spawned future.
pub fn spawn_find_task<E: Entity>(
    config: AsyncFindConfig,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    queue: FindWorkQueue,
    result_tx: async_channel::Sender<FindTaskMessage>,
    ctx: &mut ModelContext<E>,
) -> warpui::r#async::SpawnedFutureHandle {
    ctx.spawn(
        async move {
            run_find_task_loop(config, terminal_model, queue, result_tx).await;
        },
        |_me, (), _ctx| {
            // Task completed — nothing to do here as results are sent via channel.
        },
    )
}

/// Runs the main find task loop, pulling work items from the queue.
async fn run_find_task_loop(
    config: AsyncFindConfig,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    queue: FindWorkQueue,
    result_tx: async_channel::Sender<FindTaskMessage>,
) {
    // Build RegexDFAs from config.
    let Ok(dfas) = RegexDFAs::new_with_config(
        config.query.as_str(),
        FindConfig {
            is_regex_enabled: config.is_regex_enabled,
            is_case_sensitive: config.is_case_sensitive,
        },
    ) else {
        // Invalid regex — signal completion with no matches.
        let _ = result_tx.send(FindTaskMessage::Done).await;
        return;
    };

    while let Ok((item, queue_drained)) = queue.pop().await {
        match item {
            FindWorkItem::FullBlock { block_index } => {
                scan_terminal_block_chunked(
                    block_index,
                    &terminal_model,
                    &dfas,
                    &result_tx,
                    config.block_sort_direction,
                )
                .await;
            }
            FindWorkItem::DirtyRange {
                block_index,
                grid_type,
                row_range,
                num_lines_truncated,
            } => {
                scan_grid_chunked(
                    block_index,
                    grid_type,
                    *row_range.start(),
                    Some(*row_range.end() + 1),
                    ScanResultMode::DirtyRange {
                        num_lines_truncated,
                    },
                    &terminal_model,
                    &dfas,
                    &result_tx,
                )
                .await;
            }
            FindWorkItem::AIBlock {
                view_id,
                total_index,
            } => {
                // Forward to main thread for execution.
                let _ = result_tx
                    .send(FindTaskMessage::ScanAIBlock {
                        view_id,
                        total_index,
                    })
                    .await;
            }
        }

        // The emptiness flag is checked atomically with the pop inside
        // the queue lock, avoiding the TOCTOU race of a separate
        // `is_empty()` call.
        if queue_drained {
            let _ = result_tx.send(FindTaskMessage::Done).await;
        }
    }
}

/// Scans a terminal block in chunks, streaming results back to the main thread.
async fn scan_terminal_block_chunked(
    block_index: BlockIndex,
    terminal_model: &Arc<FairMutex<TerminalModel>>,
    dfas: &RegexDFAs,
    result_tx: &async_channel::Sender<FindTaskMessage>,
    block_sort_direction: crate::terminal::model::terminal_model::BlockSortDirection,
) {
    // Determine grid order based on sort direction.
    let grid_order = match block_sort_direction {
        crate::terminal::model::terminal_model::BlockSortDirection::MostRecentFirst => {
            &[GridType::PromptAndCommand, GridType::Output]
        }
        crate::terminal::model::terminal_model::BlockSortDirection::MostRecentLast => {
            &[GridType::Output, GridType::PromptAndCommand]
        }
    };

    for &grid_type in grid_order {
        scan_grid_chunked(
            block_index,
            grid_type,
            0,
            None,
            ScanResultMode::FullBlock,
            terminal_model,
            dfas,
            result_tx,
        )
        .await;
    }
}

/// Controls how each chunk's matches are sent to the main thread.
enum ScanResultMode {
    /// Send [`FindTaskMessage::BlockGridMatches`] per chunk. Empty chunks are
    /// skipped (no message sent).
    FullBlock,
    /// Send [`FindTaskMessage::DirtyRangeMatches`] per chunk, converting the
    /// scanned row range to absolute coordinates using the provided truncation
    /// offset. Messages are always sent, even for empty chunks, so that old
    /// matches in the sub-range are cleared.
    DirtyRange { num_lines_truncated: u64 },
}

/// Scans a range of rows within a single grid in chunks, releasing the
/// terminal model lock between chunks to avoid blocking the main thread.
///
/// Both full-block scanning and dirty-range scanning delegate to this
/// function; the [`ScanResultMode`] determines the message type sent per
/// chunk.
///
/// # Arguments
/// * `start_row` — First row to scan (inclusive).
/// * `end_row` — Upper bound on rows to scan (exclusive). `None` scans to
///   the end of the grid.
/// * `mode` — Determines the message type sent per chunk.
#[allow(clippy::too_many_arguments)]
async fn scan_grid_chunked(
    block_index: BlockIndex,
    grid_type: GridType,
    start_row: usize,
    end_row: Option<usize>,
    mode: ScanResultMode,
    terminal_model: &Arc<FairMutex<TerminalModel>>,
    dfas: &RegexDFAs,
    result_tx: &async_channel::Sender<FindTaskMessage>,
) {
    let mut current_row = start_row;

    loop {
        let chunk_result = {
            let lock_start = Instant::now();
            let model = terminal_model.lock();

            let Some(block) = model.block_list().block_at(block_index) else {
                // Block no longer exists.
                return;
            };

            let grid = match grid_type {
                GridType::Output => block.output_grid(),
                GridType::PromptAndCommand => block.prompt_and_command_grid(),
                _ => return,
            };

            let grid_handler = grid.grid_handler();
            let total_rows = grid_handler.total_rows();
            let effective_end = end_row.unwrap_or(total_rows).min(total_rows);

            if current_row >= effective_end {
                return;
            }

            let chunk_end = (current_row + ROWS_PER_CHUNK).min(effective_end);

            let point_matches = scan_grid_range(grid_handler, dfas, current_row, chunk_end);
            let matches: Vec<AbsoluteMatch> = point_matches
                .iter()
                .map(|range| AbsoluteMatch::from_range(range, grid_handler))
                .collect();

            let elapsed = lock_start.elapsed();
            (matches, chunk_end, effective_end, elapsed)
        };

        let (mut matches, chunk_end, effective_end, elapsed) = chunk_result;

        // find_in_range returns matches in descending order; reverse to ascending.
        matches.reverse();

        // Send chunk results based on mode.
        match &mode {
            ScanResultMode::FullBlock => {
                if !matches.is_empty() {
                    let _ = result_tx
                        .send(FindTaskMessage::BlockGridMatches {
                            block_index,
                            grid_type,
                            matches,
                        })
                        .await;
                }
            }
            ScanResultMode::DirtyRange {
                num_lines_truncated,
            } => {
                let absolute_start = current_row as u64 + num_lines_truncated;
                let absolute_end = (chunk_end - 1) as u64 + num_lines_truncated;
                let _ = result_tx
                    .send(FindTaskMessage::DirtyRangeMatches {
                        block_index,
                        grid_type,
                        dirty_range: absolute_start..=absolute_end,
                        matches,
                    })
                    .await;
            }
        }

        if chunk_end >= effective_end {
            break;
        }

        current_row = chunk_end;

        // Yield to let other tasks run if we held the lock for a while.
        if elapsed.as_millis() > MAX_LOCK_DURATION_MS as u128 / 2 {
            yield_now().await;
        }
    }
}

/// Scans a range of rows in a grid for matches.
fn scan_grid_range(
    grid: &GridHandler,
    dfas: &RegexDFAs,
    start_row: usize,
    end_row: usize,
) -> Vec<RangeInclusive<Point>> {
    let columns = Dimensions::columns(grid);
    let start_point = Point::new(start_row, 0);
    let end_point = Point::new(end_row.saturating_sub(1), columns.saturating_sub(1));

    grid.find_in_range(dfas, start_point, end_point).collect()
}
