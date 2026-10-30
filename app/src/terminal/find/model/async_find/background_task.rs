//! Background task for async find operations.
//!
//! This module contains the logic that runs on a background thread to scan
//! terminal blocks for matches without blocking the main thread. The task
//! pulls work items from a shared [`FindWorkQueue`] and streams results
//! back via an `async_channel`.

use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

use futures_lite::future::yield_now;
use instant::Instant;
use parking_lot::FairMutex;
use warp_terminal::model::grid::Dimensions;
use warpui::{Entity, ModelContext};

use super::work_queue::{FindWorkItem, FindWorkQueue};
use super::{AbsoluteMatch, AsyncFindConfig, FindTaskMessage, searchable_grid_handler};
use crate::terminal::block_list_element::GridType;
use crate::terminal::model::TerminalModel;
use crate::terminal::model::find::{FindConfig, RegexDFAs};
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::BlockIndex;

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
            } => {
                scan_grid_chunked(
                    block_index,
                    grid_type,
                    *row_range.start(),
                    Some(*row_range.end() + 1),
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
            terminal_model,
            dfas,
            result_tx,
        )
        .await;
    }
}

/// The rows of one scan chunk, in both grid and absolute coordinates.
#[derive(Debug, PartialEq, Eq)]
struct ChunkRows {
    /// The grid rows to scan, which are empty if none of the chunk's rows are in the grid.
    grid_rows: Range<usize>,
    absolute_rows: RangeInclusive<u64>,
    /// Whether this chunk reaches the end of the requested range (or of the grid).
    is_last: bool,
}

/// Returns the next chunk of rows to scan, starting at absolute row `absolute_start` and ending
/// before absolute row `absolute_end` (or at the end of the grid if `None`), given the grid's
/// current truncation and size. Rows already truncated from scrollback are skipped.
///
/// If none of the remaining rows of a bounded range are in the grid (e.g. they were truncated from
/// scrollback, possibly with stored matches, before or during the scan), returns a last chunk
/// with no grid rows that covers all of them, so that matches stored in those rows are still
/// cleared. Returns `None` if an unbounded scan has no rows left.
fn next_chunk_rows(
    absolute_start: u64,
    absolute_end: Option<u64>,
    num_lines_truncated: u64,
    total_rows: usize,
) -> Option<ChunkRows> {
    let total_rows = total_rows as u64;
    let grid_start = absolute_start.saturating_sub(num_lines_truncated);
    let grid_end = absolute_end
        .map_or(total_rows, |end| end.saturating_sub(num_lines_truncated))
        .min(total_rows);
    if grid_start >= grid_end {
        let absolute_end = absolute_end.filter(|&end| absolute_start < end)?;
        return Some(ChunkRows {
            grid_rows: 0..0,
            absolute_rows: absolute_start..=absolute_end - 1,
            is_last: true,
        });
    }
    let chunk_end = (grid_start + ROWS_PER_CHUNK as u64).min(grid_end);
    Some(ChunkRows {
        grid_rows: grid_start as usize..chunk_end as usize,
        absolute_rows: grid_start + num_lines_truncated..=chunk_end - 1 + num_lines_truncated,
        is_last: chunk_end >= grid_end,
    })
}

/// Scans a range of rows within a single grid in chunks, releasing the
/// terminal model lock between chunks to avoid blocking the main thread.
///
/// Both full-block scanning and dirty-range scanning delegate to this function. A
/// [`FindTaskMessage::BlockGridMatches`] is sent per chunk, even for empty chunks, so that old
/// matches in the chunk's rows are cleared and matches truncated from scrollback are pruned.
///
/// Scan positions are tracked in absolute rows and converted to grid rows per chunk, under the same
/// lock as the scan, because scrollback can be truncated while the lock is released.
///
/// # Arguments
/// * `start_row` - First absolute row to scan (inclusive).
/// * `end_row` - Upper bound on absolute rows to scan (exclusive). `None` scans to
///   the end of the grid.
async fn scan_grid_chunked(
    block_index: BlockIndex,
    grid_type: GridType,
    start_row: u64,
    end_row: Option<u64>,
    terminal_model: &Arc<FairMutex<TerminalModel>>,
    dfas: &RegexDFAs,
    result_tx: &async_channel::Sender<FindTaskMessage>,
) {
    let mut current_row = start_row;

    loop {
        let (mut matches, chunk, elapsed) = {
            let lock_start = Instant::now();
            let model = terminal_model.lock();

            let Some(block) = model.block_list().block_at(block_index) else {
                // Block no longer exists.
                return;
            };
            let Some(grid_handler) = searchable_grid_handler(block, grid_type) else {
                return;
            };
            let Some(chunk) = next_chunk_rows(
                current_row,
                end_row,
                grid_handler.num_lines_truncated(),
                grid_handler.total_rows(),
            ) else {
                return;
            };

            let point_matches = scan_grid_range(
                grid_handler,
                dfas,
                chunk.grid_rows.start,
                chunk.grid_rows.end,
            );
            let matches: Vec<AbsoluteMatch> = point_matches
                .iter()
                .map(|range| AbsoluteMatch::from_range(range, grid_handler))
                .collect();

            (matches, chunk, lock_start.elapsed())
        };
        let next_row = *chunk.absolute_rows.end() + 1;

        // find_in_range returns matches in descending order; reverse to ascending.
        matches.reverse();

        let _ = result_tx
            .send(FindTaskMessage::BlockGridMatches {
                block_index,
                grid_type,
                row_range: chunk.absolute_rows,
                matches,
            })
            .await;

        if chunk.is_last {
            break;
        }

        current_row = next_row;

        // Yield to let other tasks run if we held the lock for a while.
        if elapsed.as_millis() > MAX_LOCK_DURATION_MS as u128 / 2 {
            yield_now().await;
        }
    }
}

/// Scans the grid rows from `start_row` (inclusive) to `end_row` (exclusive) for matches.
fn scan_grid_range(
    grid: &GridHandler,
    dfas: &RegexDFAs,
    start_row: usize,
    end_row: usize,
) -> Vec<RangeInclusive<Point>> {
    // The inclusive end point below would otherwise make an empty range scan row `end_row - 1`.
    if start_row >= end_row {
        return Vec::new();
    }
    let columns = Dimensions::columns(grid);
    let start_point = Point::new(start_row, 0);
    let end_point = Point::new(end_row.saturating_sub(1), columns.saturating_sub(1));

    grid.find_in_range(dfas, start_point, end_point).collect()
}

/// Runs [`scan_grid_chunked`] to completion and returns the messages it sent.
#[cfg(test)]
pub(super) fn scan_grid_messages(
    block_index: BlockIndex,
    grid_type: GridType,
    start_row: u64,
    end_row: Option<u64>,
    terminal_model: &Arc<FairMutex<TerminalModel>>,
    dfas: &RegexDFAs,
) -> Vec<FindTaskMessage> {
    let (result_tx, result_rx) = async_channel::unbounded();
    futures_lite::future::block_on(scan_grid_chunked(
        block_index,
        grid_type,
        start_row,
        end_row,
        terminal_model,
        dfas,
        &result_tx,
    ));
    drop(result_tx);
    std::iter::from_fn(|| result_rx.try_recv().ok()).collect()
}

#[cfg(test)]
#[path = "background_task_tests.rs"]
mod tests;
