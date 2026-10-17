use std::sync::Arc;

use futures_lite::future::block_on;
use parking_lot::FairMutex;
use warp_terminal::model::grid::Dimensions as _;

use super::{ChunkRows, ROWS_PER_CHUNK, next_chunk_rows, scan_grid_chunked, scan_grid_range};
use crate::terminal::block_list_element::GridType;
use crate::terminal::find::model::async_find::{AbsoluteMatch, FindTaskMessage};
use crate::terminal::model::TerminalModel;
use crate::terminal::model::find::{FindConfig, RegexDFAs};

const MAX_BLOCK_SCROLL_LIMIT: usize = 20;

fn output_num_lines_truncated(model: &TerminalModel) -> u64 {
    model
        .block_list()
        .active_block()
        .output_grid()
        .grid_handler()
        .num_lines_truncated()
}

fn filler_lines(count: usize) -> String {
    "x\r\n".repeat(count)
}

fn foo_dfas() -> RegexDFAs {
    RegexDFAs::new_with_config(
        "foo",
        FindConfig {
            is_regex_enabled: false,
            is_case_sensitive: false,
        },
    )
    .unwrap()
}

/// Regression test: scrollback truncation while dirty rows accumulate, and between enqueueing a
/// dirty range and scanning it, must neither shift which content is scanned nor mislabel the rows
/// the results cover.
#[test]
fn test_dirty_range_scan_after_scrollback_truncation_scans_dirty_content() {
    let mut model = TerminalModel::mock_with_max_block_scroll_limit(MAX_BLOCK_SCROLL_LIMIT);
    model.simulate_long_running_block("cmd", &filler_lines(2 * MAX_BLOCK_SCROLL_LIMIT));
    let output_grid = model
        .block_list_mut()
        .active_block_mut()
        .grid_of_type_mut(GridType::Output)
        .unwrap();
    output_grid.grid_handler_mut().take_find_dirty_rows_range();

    model.process_bytes("foo\r\nfoo\r\nfoo\r\n");
    let enqueued_num_lines_truncated = output_num_lines_truncated(&model);
    let dirty_rows = model
        .block_list_mut()
        .active_block_mut()
        .grid_of_type_mut(GridType::Output)
        .unwrap()
        .grid_handler_mut()
        .take_find_dirty_rows_range()
        .unwrap();

    model.process_bytes(filler_lines(5).as_str());
    assert!(output_num_lines_truncated(&model) > enqueued_num_lines_truncated);

    let dfas = foo_dfas();
    let expected_match_rows = {
        let grid_handler = model
            .block_list()
            .active_block()
            .output_grid()
            .grid_handler();
        let mut rows = scan_grid_range(grid_handler, &dfas, 0, grid_handler.total_rows())
            .iter()
            .map(|range| AbsoluteMatch::from_range(range, grid_handler).start_row())
            .collect::<Vec<_>>();
        rows.sort();
        rows
    };
    assert_eq!(expected_match_rows.len(), 3);

    let block_index = model.block_list().active_block_index();
    let terminal_model = Arc::new(FairMutex::new(model));
    let (result_tx, result_rx) = async_channel::unbounded();
    block_on(scan_grid_chunked(
        block_index,
        GridType::Output,
        *dirty_rows.start(),
        Some(*dirty_rows.end() + 1),
        &terminal_model,
        &dfas,
        &result_tx,
    ));
    drop(result_tx);

    let mut match_rows = Vec::new();
    while let Ok(message) = result_rx.try_recv() {
        let FindTaskMessage::BlockGridMatches {
            row_range: dirty_range,
            matches,
            ..
        } = message
        else {
            panic!("Grid scans should only send BlockGridMatches.");
        };
        for absolute_match in &matches {
            assert!(
                dirty_range.contains(&absolute_match.start_row())
                    && dirty_range.contains(&absolute_match.end_row()),
                "Match {absolute_match:?} lies outside its batch's range {dirty_range:?}."
            );
        }
        match_rows.extend(matches.iter().map(AbsoluteMatch::start_row));
    }

    assert_eq!(match_rows, expected_match_rows);
}

#[test]
fn test_next_chunk_rows_splits_range_into_chunks() {
    let rows_per_chunk = ROWS_PER_CHUNK as u64;
    assert_eq!(
        next_chunk_rows(0, None, 0, ROWS_PER_CHUNK + 5),
        Some(ChunkRows {
            grid_rows: 0..ROWS_PER_CHUNK,
            absolute_rows: 0..=rows_per_chunk - 1,
            is_last: false,
        })
    );
    assert_eq!(
        next_chunk_rows(rows_per_chunk, None, 0, ROWS_PER_CHUNK + 5),
        Some(ChunkRows {
            grid_rows: ROWS_PER_CHUNK..ROWS_PER_CHUNK + 5,
            absolute_rows: rows_per_chunk..=rows_per_chunk + 4,
            is_last: true,
        })
    );
    assert_eq!(
        next_chunk_rows(rows_per_chunk + 5, None, 0, ROWS_PER_CHUNK + 5),
        None
    );
}

#[test]
fn test_next_chunk_rows_maps_absolute_rows_through_current_truncation() {
    assert_eq!(
        next_chunk_rows(105, Some(110), 100, 50),
        Some(ChunkRows {
            grid_rows: 5..10,
            absolute_rows: 105..=109,
            is_last: true,
        })
    );
}

#[test]
fn test_next_chunk_rows_skips_rows_truncated_since_the_range_was_recorded() {
    assert_eq!(
        next_chunk_rows(105, Some(110), 107, 50),
        Some(ChunkRows {
            grid_rows: 0..3,
            absolute_rows: 107..=109,
            is_last: true,
        })
    );
    assert_eq!(next_chunk_rows(105, Some(110), 110, 50), None);
}

#[test]
fn test_next_chunk_rows_clamps_to_grid_end() {
    assert_eq!(
        next_chunk_rows(0, Some(100), 0, 20),
        Some(ChunkRows {
            grid_rows: 0..20,
            absolute_rows: 0..=19,
            is_last: true,
        })
    );
}
