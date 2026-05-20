//! Tests for async find functionality.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::App;

use crate::terminal::block_list_element::GridType;
use crate::terminal::find::model::block_list::run_find_on_block_list;
use crate::terminal::find::model::{FindOptions, TerminalFindModel};
use crate::terminal::find::BlockListMatch;
use crate::terminal::model::grid::grid_handler::AbsolutePoint;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::{BlockIndex, BlockSortDirection};
use crate::terminal::model::TerminalModel;

use super::{
    is_query_refinement, AbsoluteMatch, AsyncFindConfig, AsyncFindController, AsyncFindStatus,
    BlockFindResults, FindTaskMessage,
};

/// Helper to create an AbsoluteMatch at a given row with default column span.
fn make_match(row: u64) -> AbsoluteMatch {
    AbsoluteMatch {
        start: AbsolutePoint { row, col: 0 },
        end: AbsolutePoint { row, col: 5 },
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
    }
}

#[test]
fn test_async_find_produces_same_results_as_sync_find() {
    App::test((), |mut app| async move {
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
        let test_model = app.add_model(|_| {
            let mut model = TerminalFindModel::new(terminal_model.clone());
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
        for _ in 0..100 {
            let is_complete = test_model.update(&mut app, |model, _ctx| {
                model
                    .async_find_controller
                    .as_ref()
                    .map(|c| matches!(c.status(), AsyncFindStatus::Complete))
                    .unwrap_or(false)
            });
            if is_complete {
                break;
            }
            // Small delay to let background task and stream delivery run.
            warpui::r#async::Timer::after(std::time::Duration::from_millis(10)).await;
        }

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
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        // Create some blocks with content.
        mock_terminal_model.simulate_block("cmd1", "line1\r\nline2\r\n");
        mock_terminal_model.simulate_block("cmd2", "line3\r\nline4\r\n");

        let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));
        let test_model = app.add_model(|_| {
            let mut model = TerminalFindModel::new(terminal_model.clone());
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
        let mock_terminal_model = TerminalModel::mock(None, None);
        let terminal_model = Arc::new(FairMutex::new(mock_terminal_model));

        let test_model = app.add_model(|_| {
            let mut model = TerminalFindModel::new(terminal_model.clone());
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
    use crate::view_components::find::FindDirection;

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
    assert!(results
        .terminal_matches
        .contains_key(&(BlockIndex(1), GridType::Output)));
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

        let test_model = app.add_model(|_| {
            let mut model = TerminalFindModel::new(terminal_model.clone());
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

        for _ in 0..100 {
            let is_complete = test_model.update(&mut app, |model, _ctx| {
                model
                    .async_find_controller
                    .as_ref()
                    .map(|c| matches!(c.status(), AsyncFindStatus::Complete))
                    .unwrap_or(false)
            });
            if is_complete {
                break;
            }
            warpui::r#async::Timer::after(std::time::Duration::from_millis(10)).await;
        }

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
