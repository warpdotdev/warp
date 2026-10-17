//! Tests for async find functionality.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::{App, EntityId, ModelHandle};

use super::work_queue::FindWorkQueue;
use super::{
    AbsoluteMatch, AsyncFindConfig, AsyncFindController, AsyncFindStatus, BlockFindResults,
    FindTaskMessage, is_query_refinement,
};
use crate::terminal::block_list_element::GridType;
use crate::terminal::find::model::block_list::run_find_on_block_list;
use crate::terminal::find::model::{FindOptions, TerminalFindModel};
use crate::terminal::find::{BlockListMatch, RichContentMatchId};
use crate::terminal::model::TerminalModel;
use crate::terminal::model::blocks::TotalIndex;
use crate::terminal::model::grid::grid_handler::AbsolutePoint;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::{BlockIndex, BlockSortDirection};
use crate::test_util::assert_eventually;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::view_components::find::FindDirection;

/// Helper to create an AbsoluteMatch at a given row with default column span.
fn make_match(row: u64) -> AbsoluteMatch {
    AbsoluteMatch {
        start: AbsolutePoint { row, col: 0 },
        end: AbsolutePoint { row, col: 5 },
        is_filtered: false,
    }
}

/// Helper to create an AbsoluteMatch at a given row and column range.
fn make_match_at(row: u64, start_col: usize, end_col: usize) -> AbsoluteMatch {
    AbsoluteMatch {
        start: AbsolutePoint {
            row,
            col: start_col,
        },
        end: AbsolutePoint { row, col: end_col },
        is_filtered: false,
    }
}

#[test]
fn test_async_find_produces_same_results_as_sync_find() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("foobar", "foo\r\nbar\r\n");
        mock_terminal_model.simulate_block("barbaz", "bar baz\r\n");

        let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));

        // Run sync find for comparison.
        let sync_run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                terminal_model.lock().block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // Run async find using TerminalFindModel.
        let test_model = app.add_model(|ctx| {
            let mut model = TerminalFindModel::new(terminal_model.clone(), ctx);
            if model.async_find_controller.is_none() {
                model.async_find_controller =
                    Some(AsyncFindController::new(terminal_model.clone()));
            }
            model
        });

        test_model.update(&mut app, |model, ctx| {
            model.async_find_controller.as_mut().unwrap().start_find(
                &FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                BlockSortDirection::MostRecentLast,
                ctx,
            );
        });

        // Wait for async find to complete. The stream-based delivery processes
        // results automatically; we just need to yield to the executor.
        assert_eventually!(
            200 => test_model.update(&mut app, |model, _ctx| {
                model
                    .async_find_controller
                    .as_ref()
                    .map(|c| matches!(c.status(), AsyncFindStatus::Complete))
                    .unwrap_or(false)
            }),
            "Async find should complete"
        );

        let (status, async_count) = test_model.update(&mut app, |model, _ctx| {
            let c = model.async_find_controller.as_ref().unwrap();
            (c.status().clone(), c.match_count())
        });

        assert_eq!(
            status,
            AsyncFindStatus::Complete,
            "Async find should complete"
        );

        // Compare match counts.
        let sync_count = sync_run.matches().count();
        assert_eq!(
            async_count, sync_count,
            "Async find should produce same number of matches as sync find"
        );

        // Verify the matches are in the expected blocks and grids.
        let model = terminal_model.lock();
        for sync_match in sync_run.matches() {
            if let BlockListMatch::CommandBlock(grid_match) = sync_match {
                let async_matches = test_model.update(&mut app, |m, _ctx| {
                    m.async_find_controller
                        .as_ref()
                        .unwrap()
                        .matches_for_block_grid(grid_match.block_index, grid_match.grid_type)
                        .cloned()
                });
                assert!(
                    async_matches.is_some(),
                    "Async find should have matches for block {:?} grid {:?}",
                    grid_match.block_index,
                    grid_match.grid_type
                );

                // Convert async match to relative range and compare.
                let block = model.block_list().block_at(grid_match.block_index).unwrap();
                let grid = match grid_match.grid_type {
                    GridType::Output => block.output_grid().grid_handler(),
                    GridType::PromptAndCommand => block.prompt_and_command_grid().grid_handler(),
                    _ => continue,
                };

                let async_ranges: Vec<_> = async_matches
                    .unwrap()
                    .iter()
                    .filter_map(|m| m.to_range(grid))
                    .collect();

                assert!(
                    async_ranges.contains(&grid_match.range),
                    "Async find should contain match {:?} in block {:?} grid {:?}",
                    grid_match.range,
                    grid_match.block_index,
                    grid_match.grid_type
                );
            }
        }
    });
}

#[test]
fn test_async_find_cancellation() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let mut mock_terminal_model = TerminalModel::mock(None, None);
        // Create some blocks with content.
        mock_terminal_model.simulate_block("cmd1", "line1\r\nline2\r\n");
        mock_terminal_model.simulate_block("cmd2", "line3\r\nline4\r\n");

        let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
        let test_model = app.add_model(|ctx| {
            let mut model = TerminalFindModel::new(terminal_model.clone(), ctx);
            if model.async_find_controller.is_none() {
                model.async_find_controller =
                    Some(AsyncFindController::new(terminal_model.clone()));
            }
            model
        });

        // Start a find operation.
        test_model.update(&mut app, |model, ctx| {
            model.async_find_controller.as_mut().unwrap().start_find(
                &FindOptions {
                    query: Some("line".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                BlockSortDirection::MostRecentLast,
                ctx,
            );
        });

        // Verify we're scanning.
        let is_scanning = test_model.update(&mut app, |model, _ctx| {
            model.async_find_controller.as_ref().unwrap().is_scanning()
        });
        assert!(is_scanning, "Should be scanning after starting find");

        // Cancel the find.
        test_model.update(&mut app, |model, _ctx| {
            model
                .async_find_controller
                .as_mut()
                .unwrap()
                .cancel_current_find();
        });

        // Verify cancellation state.
        let (is_scanning, has_active) = test_model.update(&mut app, |model, _ctx| {
            let c = model.async_find_controller.as_ref().unwrap();
            (c.is_scanning(), c.has_active_find())
        });
        assert!(!is_scanning, "Should not be scanning after cancellation");
        assert!(has_active, "Config should still be set after cancellation");

        // Clear results should reset everything.
        test_model.update(&mut app, |model, ctx| {
            model
                .async_find_controller
                .as_mut()
                .unwrap()
                .clear_results(ctx);
        });

        let (has_active, status) = test_model.update(&mut app, |model, _ctx| {
            let c = model.async_find_controller.as_ref().unwrap();
            (c.has_active_find(), c.status().clone())
        });
        assert!(
            !has_active,
            "Should not have active find after clear_results"
        );
        assert_eq!(
            status,
            AsyncFindStatus::Idle,
            "Status should be Idle after clear_results"
        );
    });
}

#[test]
fn test_message_processing_updates_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let mock_terminal_model = TerminalModel::mock(None, None);
        let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));

        let test_model = app.add_model(|ctx| {
            let mut model = TerminalFindModel::new(terminal_model.clone(), ctx);
            let mut controller = AsyncFindController::new(terminal_model);
            // Manually set up state as if a find is in progress.
            controller.set_test_status(AsyncFindStatus::Scanning);
            model.async_find_controller = Some(controller);
            model
        });

        // Process a BlockGridMatches message directly.
        test_model.update(&mut app, |model, ctx| {
            model
                .async_find_controller
                .as_mut()
                .unwrap()
                .process_message(
                    FindTaskMessage::BlockGridMatches {
                        block_index: BlockIndex(1),
                        grid_type: GridType::Output,
                        row_range: 0..=1,
                        matches: vec![make_match_at(0, 0, 2), make_match_at(1, 0, 2)],
                    },
                    ctx,
                );
        });

        // Verify state updates.
        let (match_count, status, focused_idx) = test_model.update(&mut app, |model, _ctx| {
            let c = model.async_find_controller.as_ref().unwrap();
            (c.match_count(), c.status().clone(), c.focused_match_index())
        });

        assert_eq!(match_count, 2, "Should have 2 matches");
        assert_eq!(
            status,
            AsyncFindStatus::Scanning,
            "Status should still be scanning until Done is received"
        );
        assert_eq!(focused_idx, Some(0), "Should auto-focus first match");

        // Process a Done message.
        test_model.update(&mut app, |model, ctx| {
            model
                .async_find_controller
                .as_mut()
                .unwrap()
                .process_message(FindTaskMessage::Done, ctx);
        });

        let status = test_model.update(&mut app, |model, _ctx| {
            model
                .async_find_controller
                .as_ref()
                .unwrap()
                .status()
                .clone()
        });

        assert_eq!(
            status,
            AsyncFindStatus::Complete,
            "Status should be Complete after Done message"
        );
    });
}

#[test]
fn test_block_invalidation_with_dirty_range() {
    // Test that dirty range invalidation merges correctly with existing matches.
    let mut results = BlockFindResults::default();
    let block_index = BlockIndex(0);
    let grid_type = GridType::Output;

    // Seed with matches at absolute rows 5, 15, 25.
    results.terminal_matches.insert(
        (block_index, grid_type),
        vec![
            make_match_at(5, 0, 2),
            make_match_at(15, 0, 2),
            make_match_at(25, 0, 2),
        ],
    );

    // Dirty range 10..=20 overlaps with match at row 15.
    // New matches found in dirty range: rows 12 and 18.
    let new_matches = vec![make_match_at(12, 0, 2), make_match_at(18, 0, 2)];
    results.update_dirty_matches(block_index, grid_type, 10..=20, new_matches);

    let stored = results
        .terminal_matches
        .get(&(block_index, grid_type))
        .unwrap();

    // Should have: 5, 12, 18, 25 (match at 15 was replaced).
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[0].start_row(), 5);
    assert_eq!(stored[1].start_row(), 12);
    assert_eq!(stored[2].start_row(), 18);
    assert_eq!(stored[3].start_row(), 25);
}

#[test]
fn test_focus_next_match_wraps_around() {
    let mock_terminal_model = TerminalModel::mock(None, None);
    let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
    let mut controller = AsyncFindController::new(terminal_model);

    // Manually add some matches.
    controller.block_results_mut().terminal_matches.insert(
        (BlockIndex(0), GridType::Output),
        vec![
            make_match_at(0, 0, 2),
            make_match_at(1, 0, 2),
            make_match_at(2, 0, 2),
        ],
    );

    assert_eq!(controller.match_count(), 3);

    // Default block_sort_direction is MostRecentLast, so:
    //   Down = decrement (toward newest/index 0)
    //   Up   = increment (toward oldest/higher indices)

    // Focus first match from None.
    controller.focus_next_match(FindDirection::Down);
    assert_eq!(controller.focused_match_index(), Some(0));

    // Down decrements: 0 wraps to last index.
    controller.focus_next_match(FindDirection::Down);
    assert_eq!(controller.focused_match_index(), Some(2));

    // Down decrements: 2 → 1.
    controller.focus_next_match(FindDirection::Down);
    assert_eq!(controller.focused_match_index(), Some(1));

    // Down decrements: 1 → 0.
    controller.focus_next_match(FindDirection::Down);
    assert_eq!(controller.focused_match_index(), Some(0));

    // Up increments: 0 → 1.
    controller.focus_next_match(FindDirection::Up);
    assert_eq!(controller.focused_match_index(), Some(1));

    // Up increments: 1 → 2.
    controller.focus_next_match(FindDirection::Up);
    assert_eq!(controller.focused_match_index(), Some(2));

    // Up wraps: 2 → 0.
    controller.focus_next_match(FindDirection::Up);
    assert_eq!(controller.focused_match_index(), Some(0));
}

#[test]
fn test_is_query_refinement() {
    assert!(is_query_refinement("hel", "hello"));
    assert!(is_query_refinement("foo", "foobar"));
    assert!(!is_query_refinement("hello", "hel"));
    assert!(!is_query_refinement("hello", "hello"));
    assert!(!is_query_refinement("bar", "foo"));
    assert!(!is_query_refinement("", "hello"));
}

#[test]
fn test_async_find_config_from_options() {
    // Empty query should return None.
    let options = FindOptions::default();
    assert!(AsyncFindConfig::from_options(&options, BlockSortDirection::MostRecentLast).is_none());

    // Query with only whitespace should return None.
    let options = FindOptions {
        query: Some(Arc::new("   ".to_string())),
        ..Default::default()
    };
    assert!(AsyncFindConfig::from_options(&options, BlockSortDirection::MostRecentLast).is_none());

    // Valid query should return Some config.
    let options = FindOptions {
        query: Some(Arc::new("hello".to_string())),
        is_case_sensitive: true,
        is_regex_enabled: false,
        blocks_to_include_in_results: Some(vec![BlockIndex(0), BlockIndex(1)]),
    };
    let config = AsyncFindConfig::from_options(&options, BlockSortDirection::MostRecentFirst);
    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config.query.as_str(), "hello");
    assert!(config.is_case_sensitive);
    assert!(!config.is_regex_enabled);
    assert_eq!(
        config.blocks_to_include,
        Some(vec![BlockIndex(0), BlockIndex(1)])
    );
}

#[test]
fn test_block_find_results_total_count() {
    let mut results = BlockFindResults::default();
    assert_eq!(results.total_match_count(), 0);

    // Add some terminal matches.
    results
        .terminal_matches
        .entry((BlockIndex(0), GridType::Output))
        .or_default()
        .push(make_match(0));
    assert_eq!(results.total_match_count(), 1);

    // Add more terminal matches.
    results
        .terminal_matches
        .entry((BlockIndex(0), GridType::Output))
        .or_default()
        .push(make_match(1));
    results
        .terminal_matches
        .entry((BlockIndex(1), GridType::PromptAndCommand))
        .or_default()
        .push(make_match(0));
    assert_eq!(results.total_match_count(), 3);
}

#[test]
fn test_block_find_results_remove_block() {
    let mut results = BlockFindResults::default();

    // Add matches for block 0 and block 1.
    results
        .terminal_matches
        .entry((BlockIndex(0), GridType::Output))
        .or_default()
        .push(make_match(0));
    results
        .terminal_matches
        .entry((BlockIndex(0), GridType::PromptAndCommand))
        .or_default()
        .push(make_match(0));
    results
        .terminal_matches
        .entry((BlockIndex(1), GridType::Output))
        .or_default()
        .push(make_match(0));
    assert_eq!(results.total_match_count(), 3);

    // Remove block 0.
    results.remove_block(BlockIndex(0));
    assert_eq!(results.total_match_count(), 1);

    // Block 1 should still have its matches.
    assert!(
        results
            .terminal_matches
            .contains_key(&(BlockIndex(1), GridType::Output))
    );
}

#[test]
fn test_async_find_status_display() {
    assert_eq!(format!("{}", AsyncFindStatus::Idle), "Idle");
    assert_eq!(format!("{}", AsyncFindStatus::Complete), "Complete");
    assert_eq!(format!("{}", AsyncFindStatus::Scanning), "Scanning");
}

#[test]
fn test_absolute_match_is_truncated() {
    let match_at_row_5 = make_match(5);
    // Not truncated when num_lines_truncated <= start row.
    assert!(!match_at_row_5.is_truncated(0));
    assert!(!match_at_row_5.is_truncated(5));
    // Truncated when num_lines_truncated > start row.
    assert!(match_at_row_5.is_truncated(6));
    assert!(match_at_row_5.is_truncated(100));
}

#[test]
fn test_update_dirty_matches_empty_existing() {
    let mut results = BlockFindResults::default();
    let block_index = BlockIndex(0);
    let grid_type = GridType::Output;

    // Update with new matches when there are no existing matches.
    let new_matches = vec![make_match(5), make_match(10), make_match(15)];
    results.update_dirty_matches(block_index, grid_type, 5..=15, new_matches.clone());

    let stored = results
        .terminal_matches
        .get(&(block_index, grid_type))
        .unwrap();
    assert_eq!(stored.len(), 3);
    assert_eq!(stored[0].start_row(), 5);
    assert_eq!(stored[1].start_row(), 10);
    assert_eq!(stored[2].start_row(), 15);
}

#[test]
fn test_update_dirty_matches_prepend() {
    let mut results = BlockFindResults::default();
    let block_index = BlockIndex(0);
    let grid_type = GridType::Output;

    // Seed with matches at rows 20, 30.
    results.terminal_matches.insert(
        (block_index, grid_type),
        vec![make_match(20), make_match(30)],
    );

    // Update with dirty range before all existing matches.
    let new_matches = vec![make_match(5), make_match(10)];
    results.update_dirty_matches(block_index, grid_type, 5..=10, new_matches);

    let stored = results
        .terminal_matches
        .get(&(block_index, grid_type))
        .unwrap();
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[0].start_row(), 5);
    assert_eq!(stored[1].start_row(), 10);
    assert_eq!(stored[2].start_row(), 20);
    assert_eq!(stored[3].start_row(), 30);
}

#[test]
fn test_update_dirty_matches_append() {
    let mut results = BlockFindResults::default();
    let block_index = BlockIndex(0);
    let grid_type = GridType::Output;

    // Seed with matches at rows 5, 10.
    results.terminal_matches.insert(
        (block_index, grid_type),
        vec![make_match(5), make_match(10)],
    );

    // Update with dirty range after all existing matches.
    let new_matches = vec![make_match(20), make_match(30)];
    results.update_dirty_matches(block_index, grid_type, 20..=30, new_matches);

    let stored = results
        .terminal_matches
        .get(&(block_index, grid_type))
        .unwrap();
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[0].start_row(), 5);
    assert_eq!(stored[1].start_row(), 10);
    assert_eq!(stored[2].start_row(), 20);
    assert_eq!(stored[3].start_row(), 30);
}

#[test]
fn test_update_dirty_matches_replace_middle() {
    let mut results = BlockFindResults::default();
    let block_index = BlockIndex(0);
    let grid_type = GridType::Output;

    // Seed with matches at rows 5, 15, 25.
    results.terminal_matches.insert(
        (block_index, grid_type),
        vec![make_match(5), make_match(15), make_match(25)],
    );

    // Update dirty range 10..=20, which overlaps with the match at row 15.
    // Replace it with matches at rows 12 and 18.
    let new_matches = vec![make_match(12), make_match(18)];
    results.update_dirty_matches(block_index, grid_type, 10..=20, new_matches);

    let stored = results
        .terminal_matches
        .get(&(block_index, grid_type))
        .unwrap();
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[0].start_row(), 5);
    assert_eq!(stored[1].start_row(), 12);
    assert_eq!(stored[2].start_row(), 18);
    assert_eq!(stored[3].start_row(), 25);
}

#[test]
fn test_update_dirty_matches_clear_range() {
    let mut results = BlockFindResults::default();
    let block_index = BlockIndex(0);
    let grid_type = GridType::Output;

    // Seed with matches at rows 5, 15, 25.
    results.terminal_matches.insert(
        (block_index, grid_type),
        vec![make_match(5), make_match(15), make_match(25)],
    );

    // Update dirty range 10..=20 with no new matches (clears the match at row 15).
    results.update_dirty_matches(block_index, grid_type, 10..=20, vec![]);

    let stored = results
        .terminal_matches
        .get(&(block_index, grid_type))
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].start_row(), 5);
    assert_eq!(stored[1].start_row(), 25);
}

fn assert_async_focused_order_matches_sync(block_sort_direction: BlockSortDirection) {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block(
            "ordtok command old ordtok",
            "ordtok old output one\r\nold output ordtok two\r\n",
        );
        mock_terminal_model.simulate_block(
            "ordtok command new ordtok",
            "ordtok new output one\r\nnew output ordtok two\r\n",
        );

        let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
        let find_options = FindOptions {
            query: Some("ordtok".to_owned().into()),
            is_regex_enabled: false,
            is_case_sensitive: false,
            ..Default::default()
        };

        let sync_order = app.update(|ctx| {
            run_find_on_block_list(
                find_options.clone(),
                terminal_model.lock().block_list(),
                &HashMap::new(),
                block_sort_direction,
                ctx,
            )
            .matches()
            .filter_map(|m| match m {
                BlockListMatch::CommandBlock(grid_match) => Some((
                    grid_match.block_index,
                    grid_match.grid_type,
                    *grid_match.range.start(),
                    *grid_match.range.end(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
        });

        let test_model = app.add_model(|ctx| {
            let mut model = TerminalFindModel::new(terminal_model.clone(), ctx);
            if model.async_find_controller.is_none() {
                model.async_find_controller =
                    Some(AsyncFindController::new(terminal_model.clone()));
            }
            model
        });

        test_model.update(&mut app, |model, ctx| {
            model
                .async_find_controller
                .as_mut()
                .expect("Async find controller should exist in test.")
                .start_find(&find_options, block_sort_direction, ctx);
        });

        assert_eventually!(
            200 => test_model.update(&mut app, |model, _ctx| {
                model
                    .async_find_controller
                    .as_ref()
                    .map(|c| matches!(c.status(), AsyncFindStatus::Complete))
                    .unwrap_or(false)
            }),
            "Async find should complete."
        );

        let (status, async_match_count) = test_model.update(&mut app, |model, _ctx| {
            let controller = model
                .async_find_controller
                .as_ref()
                .expect("Async find controller should exist in test.");
            (controller.status().clone(), controller.match_count())
        });
        assert_eq!(
            status,
            AsyncFindStatus::Complete,
            "Async find should complete.",
        );
        assert_eq!(
            async_match_count,
            sync_order.len(),
            "Async and sync paths should find the same number of terminal matches.",
        );

        let async_order_absolute = test_model.update(&mut app, |model, _ctx| {
            let controller = model
                .async_find_controller
                .as_mut()
                .expect("Async find controller should exist in test.");
            let mut ordered = Vec::new();
            for index in 0..controller.match_count() {
                controller.focused_match_index = Some(index);
                controller.update_cached_focused_match();
                let focused = controller
                    .focused_terminal_match()
                    .expect("Every focused index should resolve to a terminal match in this test.");
                ordered.push((focused.block_index, focused.grid_type, focused.range));
            }
            ordered
        });

        let async_order = {
            let model = terminal_model.lock();
            async_order_absolute
                .into_iter()
                .map(|(block_index, grid_type, absolute_match)| {
                    let block = model
                        .block_list()
                        .block_at(block_index)
                        .expect("Block should exist for focused async match.");
                    let grid = match grid_type {
                        GridType::Output => block.output_grid().grid_handler(),
                        GridType::PromptAndCommand => {
                            block.prompt_and_command_grid().grid_handler()
                        }
                        _ => panic!("Unexpected grid type in async focused match."),
                    };
                    let range = absolute_match
                        .to_range(grid)
                        .expect("Async focused match should map to a non-truncated range.");
                    (block_index, grid_type, *range.start(), *range.end())
                })
                .collect::<Vec<(BlockIndex, GridType, Point, Point)>>()
        };

        assert_eq!(
            async_order, sync_order,
            "Async focused ordering should match sync ordering for {:?}.",
            block_sort_direction
        );
    });
}

#[test]
fn test_async_focused_order_matches_sync_most_recent_last() {
    assert_async_focused_order_matches_sync(BlockSortDirection::MostRecentLast);
}

#[test]
fn test_async_focused_order_matches_sync_most_recent_first() {
    assert_async_focused_order_matches_sync(BlockSortDirection::MostRecentFirst);
}

#[test]
fn test_focused_ai_match_resolves_only_ai_block() {
    let mock_terminal_model = TerminalModel::mock(None, None);
    let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
    let mut controller = AsyncFindController::new(terminal_model);

    // Seed a single AI block with two matches. Default block sort direction is
    // MostRecentLast, which reverses per-AI-block traversal at iteration time.
    let view_id = EntityId::from_usize(42);
    let ai_match_a = RichContentMatchId::default();
    let ai_match_b = RichContentMatchId::default();
    {
        let results = controller.block_results_mut();
        results
            .ai_matches
            .insert(view_id, vec![ai_match_a, ai_match_b]);
        results.ai_total_indices.insert(view_id, TotalIndex(7));
    }

    assert_eq!(controller.match_count(), 2);
    assert!(
        controller.focused_terminal_match().is_none(),
        "There are no terminal matches; focused_terminal_match should be None."
    );

    // MostRecentLast reverses per-AI-block iteration, so index 0 resolves to
    // the last stored match (ai_match_b) and index 1 to the first.
    controller.focused_match_index = Some(0);
    controller.update_cached_focused_match();
    let focused = controller
        .focused_ai_match()
        .expect("AI match should be focused at index 0.");
    assert_eq!(focused.view_id, view_id);
    assert_eq!(focused.match_id, ai_match_b);
    assert_eq!(focused.total_index, TotalIndex(7));
    assert!(
        controller.focused_terminal_match().is_none(),
        "Terminal cache must be cleared when focus lands on an AI match."
    );

    controller.focused_match_index = Some(1);
    controller.update_cached_focused_match();
    let focused = controller
        .focused_ai_match()
        .expect("AI match should be focused at index 1.");
    assert_eq!(focused.match_id, ai_match_a);
}

#[test]
fn test_focused_ai_match_most_recent_first_preserves_storage_order() {
    let mock_terminal_model = TerminalModel::mock(None, None);
    let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
    let mut controller = AsyncFindController::new(terminal_model);

    // Override the default MostRecentLast so we exercise the un-reversed
    // per-AI-block iteration path.
    controller.block_sort_direction = BlockSortDirection::MostRecentFirst;

    let view_id = EntityId::from_usize(99);
    let ai_match_a = RichContentMatchId::default();
    let ai_match_b = RichContentMatchId::default();
    {
        let results = controller.block_results_mut();
        results
            .ai_matches
            .insert(view_id, vec![ai_match_a, ai_match_b]);
        results.ai_total_indices.insert(view_id, TotalIndex(3));
    }

    // MostRecentFirst iterates storage order: index 0 -> first, index 1 -> last.
    controller.focused_match_index = Some(0);
    controller.update_cached_focused_match();
    let focused = controller
        .focused_ai_match()
        .expect("AI match should be focused at index 0.");
    assert_eq!(focused.match_id, ai_match_a);

    controller.focused_match_index = Some(1);
    controller.update_cached_focused_match();
    let focused = controller
        .focused_ai_match()
        .expect("AI match should be focused at index 1.");
    assert_eq!(focused.match_id, ai_match_b);
}

#[test]
fn test_focused_match_index_walks_across_terminal_and_ai_blocks() {
    let mock_terminal_model = TerminalModel::mock(None, None);
    let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
    let mut controller = AsyncFindController::new(terminal_model);

    // Two blocks at different TotalIndex positions:
    //  - AI block (TotalIndex 5, newer) with one match.
    //  - Terminal block at BlockIndex(0) (TotalIndex 1, older) with one
    //    Output match. The AI block is sorted first because its TotalIndex
    //    is higher.
    let ai_view_id = EntityId::from_usize(11);
    let ai_match = RichContentMatchId::default();
    let terminal_match = make_match(0);
    {
        let results = controller.block_results_mut();
        results.ai_matches.insert(ai_view_id, vec![ai_match]);
        results.ai_total_indices.insert(ai_view_id, TotalIndex(5));
        results
            .terminal_matches
            .insert((BlockIndex(0), GridType::Output), vec![terminal_match]);
        results
            .terminal_total_indices
            .insert(BlockIndex(0), TotalIndex(1));
    }

    assert_eq!(controller.match_count(), 2);

    // Index 0 -> AI match (newest block, AI block in this fixture).
    controller.focused_match_index = Some(0);
    controller.update_cached_focused_match();
    let focused_ai = controller
        .focused_ai_match()
        .expect("Index 0 should resolve to AI match.");
    assert_eq!(focused_ai.view_id, ai_view_id);
    assert_eq!(focused_ai.match_id, ai_match);
    assert!(
        controller.focused_terminal_match().is_none(),
        "Terminal cache must be empty when focus is on AI block."
    );

    // Index 1 -> terminal match (older block).
    controller.focused_match_index = Some(1);
    controller.update_cached_focused_match();
    assert!(
        controller.focused_ai_match().is_none(),
        "AI cache must be empty when focus is on terminal block."
    );
    let focused_terminal = controller
        .focused_terminal_match()
        .expect("Index 1 should resolve to terminal match.");
    assert_eq!(focused_terminal.block_index, BlockIndex(0));
    assert_eq!(focused_terminal.grid_type, GridType::Output);
}

/// Creates a find model whose controller has an active find and work queue, but no background
/// task, so tests can deliver `FindTaskMessage`s in an exact order.
fn add_find_model_with_active_find(app: &mut App) -> ModelHandle<TerminalFindModel> {
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    add_find_model_with_active_find_on(app, terminal_model)
}

fn add_find_model_with_active_find_on(
    app: &mut App,
    terminal_model: Arc<FairMutex<TerminalModel>>,
) -> ModelHandle<TerminalFindModel> {
    app.add_model(|ctx| {
        let mut model = TerminalFindModel::new(terminal_model.clone(), ctx);
        let mut controller = AsyncFindController::new(terminal_model);
        controller.current_config = AsyncFindConfig::from_options(
            &FindOptions {
                query: Some("foo".to_owned().into()),
                ..Default::default()
            },
            BlockSortDirection::MostRecentLast,
        );
        controller.work_queue = Some(FindWorkQueue::new());
        controller.set_test_status(AsyncFindStatus::Scanning);
        model.async_find_controller = Some(controller);
        model
    })
}

fn process_messages(
    app: &mut App,
    find_model: &ModelHandle<TerminalFindModel>,
    messages: Vec<FindTaskMessage>,
) {
    find_model.update(app, |model, ctx| {
        let controller = model.async_find_controller.as_mut().unwrap();
        for message in messages {
            controller.process_message(message, ctx);
        }
    });
}

fn grid_matches(rows: RangeInclusive<u64>, match_rows: &[u64]) -> FindTaskMessage {
    FindTaskMessage::BlockGridMatches {
        block_index: BlockIndex(0),
        grid_type: GridType::Output,
        row_range: rows,
        matches: match_rows.iter().copied().map(make_match).collect(),
    }
}

fn stored_match_rows(app: &mut App, find_model: &ModelHandle<TerminalFindModel>) -> Vec<u64> {
    find_model.update(app, |model, _ctx| {
        model
            .async_find_controller
            .as_ref()
            .unwrap()
            .matches_for_block_grid(BlockIndex(0), GridType::Output)
            .map(|matches| matches.iter().map(AbsoluteMatch::start_row).collect())
            .unwrap_or_default()
    })
}

/// Regression test for warpdotdev/warp#14231: dirty-range results that were already in flight when
/// a full rescan was requested land after the block's results are cleared, followed by the full
/// rescan's chunks starting at row 0.
#[test]
fn test_full_rescan_racing_in_flight_dirty_results_keeps_matches_sorted_and_unique() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let find_model = add_find_model_with_active_find(&mut app);

        process_messages(
            &mut app,
            &find_model,
            vec![
                grid_matches(0..=49, &[0, 10, 20, 30, 40]),
                grid_matches(50..=99, &[50, 60, 70, 80, 90]),
            ],
        );

        find_model.update(&mut app, |model, _ctx| {
            model
                .async_find_controller
                .as_mut()
                .unwrap()
                .invalidate_block(BlockIndex(0), None);
        });
        assert!(stored_match_rows(&mut app, &find_model).is_empty());

        process_messages(
            &mut app,
            &find_model,
            vec![
                // Dirty-range results that were in flight before the invalidation.
                grid_matches(70..=84, &[70, 80]),
                grid_matches(85..=99, &[90]),
                // The full rescan's chunks.
                grid_matches(0..=49, &[0, 10, 20, 30, 40]),
                grid_matches(50..=99, &[50, 60, 70, 80, 90]),
                grid_matches(100..=105, &[100]),
                // A later dirty-range result.
                grid_matches(100..=120, &[100, 110, 120]),
            ],
        );

        let expected_rows = (0..=120).step_by(10).collect::<Vec<u64>>();
        assert_eq!(stored_match_rows(&mut app, &find_model), expected_rows);
        let (match_count, focused_match_index) = find_model.update(&mut app, |model, _ctx| {
            let controller = model.async_find_controller.as_ref().unwrap();
            (controller.match_count(), controller.focused_match_index())
        });
        assert_eq!(match_count, expected_rows.len());
        assert_eq!(focused_match_index, Some(0));
    });
}

#[test]
fn test_empty_full_block_chunk_clears_stale_matches_in_its_rows() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let find_model = add_find_model_with_active_find(&mut app);

        process_messages(
            &mut app,
            &find_model,
            vec![
                // Dirty-range results that landed after the block was cleared.
                grid_matches(70..=84, &[70, 80]),
                grid_matches(150..=160, &[150]),
                // An empty full-rescan chunk.
                grid_matches(0..=99, &[]),
            ],
        );

        assert_eq!(stored_match_rows(&mut app, &find_model), vec![150]);
        let match_count = find_model.update(&mut app, |model, _ctx| {
            model.async_find_controller.as_ref().unwrap().match_count()
        });
        assert_eq!(match_count, 1);
        assert_eq!(
            focused_match_row(&mut app, &find_model),
            Some((BlockIndex(0), 150))
        );
    });
}

fn focused_match_row(
    app: &mut App,
    find_model: &ModelHandle<TerminalFindModel>,
) -> Option<(BlockIndex, u64)> {
    find_model.update(app, |model, _ctx| {
        model
            .async_find_controller
            .as_ref()
            .unwrap()
            .focused_terminal_match()
            .map(|focused| (focused.block_index, focused.range.start_row()))
    })
}

#[test]
fn test_matches_delivered_only_by_dirty_ranges_are_focusable() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let find_model = add_find_model_with_active_find(&mut app);

        process_messages(&mut app, &find_model, vec![grid_matches(0..=9, &[3, 7])]);

        let match_count = find_model.update(&mut app, |model, _ctx| {
            model.async_find_controller.as_ref().unwrap().match_count()
        });
        assert_eq!(match_count, 2);
        // MostRecentLast focuses the bottom-most match first.
        assert_eq!(
            focused_match_row(&mut app, &find_model),
            Some((BlockIndex(0), 7))
        );

        find_model.update(&mut app, |model, _ctx| {
            model
                .async_find_controller
                .as_mut()
                .unwrap()
                .focus_next_match(FindDirection::Up);
        });
        assert_eq!(
            focused_match_row(&mut app, &find_model),
            Some((BlockIndex(0), 3))
        );
    });
}

#[test]
fn test_empty_full_block_chunk_for_block_without_matches_leaves_results_untouched() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let find_model = add_find_model_with_active_find(&mut app);

        process_messages(&mut app, &find_model, vec![grid_matches(0..=999, &[])]);

        let (block_grid_matches, focused_match_index) =
            find_model.update(&mut app, |model, _ctx| {
                let controller = model.async_find_controller.as_ref().unwrap();
                (
                    controller
                        .matches_for_block_grid(BlockIndex(0), GridType::Output)
                        .cloned(),
                    controller.focused_match_index(),
                )
            });
        assert_eq!(block_grid_matches, None);
        assert_eq!(focused_match_index, None);
    });
}

/// Scrollback truncation can remove stored matches while every new chunk is empty and overlaps
/// none of them, e.g. when a long run of non-matching output follows an early match.
#[test]
fn test_empty_non_overlapping_chunk_prunes_matches_truncated_from_scrollback() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let max_block_scroll_limit = 20;
        let terminal_model = Arc::new(FairMutex::new(
            TerminalModel::mock_with_max_block_scroll_limit(max_block_scroll_limit),
        ));
        let block_index = {
            let mut model = terminal_model.lock();
            model.simulate_long_running_block("cmd", "foo\r\n");
            model.block_list().active_block_index()
        };
        let find_model = add_find_model_with_active_find_on(&mut app, terminal_model.clone());
        let grid_matches_for_block =
            |rows: RangeInclusive<u64>, match_rows: &[u64]| FindTaskMessage::BlockGridMatches {
                block_index,
                grid_type: GridType::Output,
                row_range: rows,
                matches: match_rows.iter().copied().map(make_match).collect(),
            };
        process_messages(
            &mut app,
            &find_model,
            vec![grid_matches_for_block(0..=0, &[0])],
        );
        let focused_match_index = find_model.update(&mut app, |model, _ctx| {
            model
                .async_find_controller
                .as_ref()
                .unwrap()
                .focused_match_index()
        });
        assert_eq!(focused_match_index, Some(0));

        let num_lines_truncated = {
            let mut model = terminal_model.lock();
            model.process_bytes("x\r\n".repeat(3 * max_block_scroll_limit).as_str());
            model
                .block_list()
                .active_block()
                .output_grid()
                .grid_handler()
                .num_lines_truncated()
        };
        assert!(num_lines_truncated > 0);

        process_messages(
            &mut app,
            &find_model,
            vec![grid_matches_for_block(
                num_lines_truncated + 10..=num_lines_truncated + 15,
                &[],
            )],
        );

        let (stored_matches, match_count, focused_match_index) =
            find_model.update(&mut app, |model, _ctx| {
                let controller = model.async_find_controller.as_ref().unwrap();
                (
                    controller
                        .matches_for_block_grid(block_index, GridType::Output)
                        .cloned()
                        .unwrap_or_default(),
                    controller.match_count(),
                    controller.focused_match_index(),
                )
            });
        assert_eq!(stored_matches, vec![]);
        assert_eq!(match_count, 0);
        assert_eq!(focused_match_index, None);
    });
}
