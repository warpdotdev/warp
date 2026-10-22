use std::sync::Arc;

use parking_lot::FairMutex;
use warp_terminal::model::grid::Dimensions as _;

use super::{ChunkRows, ROWS_PER_CHUNK, next_chunk_rows, scan_grid_messages, scan_grid_range};
use crate::terminal::block_list_element::GridType;
use crate::terminal::find::model::async_find::{AbsoluteMatch, FindTaskMessage};
use crate::terminal::model::TerminalModel;
use crate::terminal::model::find::{FindConfig, RegexDFAs};
use crate::terminal::model::terminal_model::BlockIndex;

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

/// Scans the output grid for "foo" from `start_row` up to `end_row` (or the end of the grid) and
/// returns the messages sent.
fn scan_output_messages(
    terminal_model: &Arc<FairMutex<TerminalModel>>,
    block_index: BlockIndex,
    start_row: u64,
    end_row: Option<u64>,
) -> Vec<FindTaskMessage> {
    scan_grid_messages(
        block_index,
        GridType::Output,
        start_row,
        end_row,
        terminal_model,
        &foo_dfas(),
    )
}

/// Returns a running block whose output scrollback has been truncated, with its block index and
/// the output grid's number of truncated lines.
fn model_with_truncated_output() -> (Arc<FairMutex<TerminalModel>>, BlockIndex, u64) {
    let mut model = TerminalModel::mock_with_max_block_scroll_limit(MAX_BLOCK_SCROLL_LIMIT);
    model.simulate_long_running_block("cmd", &filler_lines(3 * MAX_BLOCK_SCROLL_LIMIT));
    let num_lines_truncated = output_num_lines_truncated(&model);
    let block_index = model.block_list().active_block_index();
    (
        Arc::new(FairMutex::new(model)),
        block_index,
        num_lines_truncated,
    )
}

/// A bounded range that is entirely gone from the grid when it is scanned is still reported, as
/// one empty chunk covering the whole range, so the handler clears and prunes its rows.
#[test]
fn test_bounded_scan_of_truncated_range_sends_one_empty_chunk_for_range() {
    let (terminal_model, block_index, num_lines_truncated) = model_with_truncated_output();
    assert!(num_lines_truncated > 5);

    let messages = scan_output_messages(&terminal_model, block_index, 2, Some(5));

    let [
        FindTaskMessage::BlockGridMatches {
            block_index: message_block_index,
            grid_type,
            row_range,
            matches,
        },
    ] = messages.as_slice()
    else {
        panic!("Expected exactly one BlockGridMatches, got {messages:?}.");
    };
    assert_eq!(*message_block_index, block_index);
    assert_eq!(*grid_type, GridType::Output);
    assert_eq!(*row_range, 2..=4);
    assert!(matches.is_empty());
}

#[test]
fn test_unbounded_scan_with_no_rows_left_sends_nothing() {
    let (terminal_model, block_index, num_lines_truncated) = model_with_truncated_output();
    let total_rows = terminal_model
        .lock()
        .block_list()
        .active_block()
        .output_grid()
        .grid_handler()
        .total_rows() as u64;

    let messages = scan_output_messages(
        &terminal_model,
        block_index,
        num_lines_truncated + total_rows,
        None,
    );

    assert!(
        messages.is_empty(),
        "Expected no messages, got {messages:?}."
    );
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
    let messages = scan_output_messages(
        &terminal_model,
        block_index,
        *dirty_rows.start(),
        Some(*dirty_rows.end() + 1),
    );

    let mut match_rows = Vec::new();
    for message in messages {
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
}

#[test]
fn test_next_chunk_rows_covers_bounded_range_gone_from_grid_without_grid_rows() {
    let gone_chunk = |absolute_rows| {
        Some(ChunkRows {
            grid_rows: 0..0,
            absolute_rows,
            is_last: true,
        })
    };
    assert_eq!(
        next_chunk_rows(105, Some(110), 110, 50),
        gone_chunk(105..=109)
    );
    assert_eq!(next_chunk_rows(60, Some(70), 0, 50), gone_chunk(60..=69));
    assert_eq!(next_chunk_rows(60, None, 0, 50), None);
}

#[test]
fn test_scan_grid_range_of_empty_range_finds_nothing() {
    let mut model = TerminalModel::mock_with_max_block_scroll_limit(MAX_BLOCK_SCROLL_LIMIT);
    model.simulate_long_running_block("cmd", "foo\r\n");
    let grid_handler = model
        .block_list()
        .active_block()
        .output_grid()
        .grid_handler();
    let dfas = foo_dfas();
    assert_eq!(scan_grid_range(grid_handler, &dfas, 0, 1).len(), 1);

    assert_eq!(scan_grid_range(grid_handler, &dfas, 0, 0), vec![]);
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
