use std::collections::HashMap;

use itertools::Itertools;
use warpui::App;

use crate::terminal::{
    block_filter::BlockFilterQuery,
    find::{
        model::{block_list::run_find_on_block_list, FindOptions},
        BlockGridMatch,
    },
    model::{
        index::Point,
        terminal_model::{BlockIndex, BlockSortDirection},
    },
    GridType, TerminalModel,
};

use super::{BlockListFindRun, BlockListMatch};
use crate::view_components::find::FindDirection;

impl BlockListFindRun {
    fn all_matches(&self) -> &[BlockListMatch] {
        &self.matches[..]
    }
}

#[test]
fn test_run_find_on_block_list() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("foobar", "foo\r\nbar\r\n");
        mock_terminal_model.simulate_block("barbaz", "bar baz\r\n");

        let run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // Matches should be sorted by recency at the block level, then "bottom to top" within each
        // block.
        assert_eq!(
            run.matches().collect_vec(),
            vec![
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 3 }..=Point { row: 0, col: 5 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
            ]
        );

        assert_eq!(run.focused_match_index(), Some(0));
    });
}

#[test]
fn test_run_find_on_block_list_pin_to_top() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("foobar", "foo\r\nbar\r\n");
        mock_terminal_model.simulate_block("barbaz", "bar baz\r\n");

        let run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentFirst,
                ctx,
            )
        });

        // Matches should be sorted by recency at the block level, then "top to bottom" within each
        // block.
        assert_eq!(
            run.matches().collect_vec(),
            vec![
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 3 }..=Point { row: 0, col: 5 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
            ]
        );
    });
}

#[test]
fn test_run_find_on_block_list_case_sensitive() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("foobar", "foo\r\nbar\r\n");
        mock_terminal_model.simulate_block("Barbaz", "Bar baz\r\n");

        let run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("Bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: true,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // Matches should be sorted by recency at the block level, then "bottom to top" within each
        // block.
        assert_eq!(
            run.matches().collect_vec(),
            vec![
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
            ]
        );
        assert_eq!(run.focused_match_index(), Some(0));
    });
}

#[test]
fn test_run_find_on_block_list_regex_enabled() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("foobar", "foo\r\nbar\r\n");
        mock_terminal_model.simulate_block("barbaz", "bar baz\r\n");

        let run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("ba[rz]".to_owned().into()),
                    is_regex_enabled: true,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // Matches should be sorted by recency at the block level, then "bottom to top" within each
        // block.
        assert_eq!(
            run.matches().collect_vec(),
            vec![
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 0, col: 4 }..=Point { row: 0, col: 6 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 3 }..=Point { row: 0, col: 5 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 3 }..=Point { row: 0, col: 5 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
            ]
        );
        assert_eq!(run.focused_match_index(), Some(0));
    });
}

#[test]
fn test_run_find_on_block_list_with_filtered_block() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("foobar", "foo\r\nbar\r\n");
        mock_terminal_model.simulate_block("barbaz filter", "bar baz\r\nbar bat\r\nbar baz\r\n");

        // Filter the second block to only show lines containing "bat".
        mock_terminal_model
            .update_filter_on_block(2.into(), BlockFilterQuery::new_for_test("bat".to_owned()));

        let run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // Matches should be sorted by recency at the block level, then "bottom to top" within each
        // block.
        //
        // The first and third rows of the most recent block contain baz, and should be filtered.
        assert_eq!(
            run.matches().collect_vec(),
            vec![
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
                &BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 3 }..=Point { row: 0, col: 5 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
            ]
        );

        // Check `all_matches` so we can ensure the filtered matches exist, but were filtered.
        assert_eq!(
            run.all_matches(),
            &[
                BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 2, col: 0 }..=Point { row: 2, col: 2 },
                    block_index: 2.into(),
                    is_filtered: true,
                }),
                BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: true,
                }),
                BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                    block_index: 2.into(),
                    is_filtered: false,
                }),
                BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::Output,
                    range: Point { row: 1, col: 0 }..=Point { row: 1, col: 2 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
                BlockListMatch::CommandBlock(BlockGridMatch {
                    grid_type: GridType::PromptAndCommand,
                    range: Point { row: 0, col: 3 }..=Point { row: 0, col: 5 },
                    block_index: 1.into(),
                    is_filtered: false,
                }),
            ]
        );
    });
}

/// Regression test for https://github.com/warpdotdev/warp/issues/9542
///
/// When the active block's output is still streaming and the find results are refreshed,
/// the focused match must remain on the same text span even though new matches are
/// inserted before it in the match vector.
#[test]
fn test_rerun_on_block_preserves_focused_match_in_active_block() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        // Block 1: a finished block.
        mock_terminal_model.simulate_block("echo bar", "bar\r\n");
        // Block 2: a long-running block whose command also matches so there are both
        // output and prompt matches.  This lets us navigate to the prompt match and then
        // verify it stays focused after new output matches are spliced in before it.
        mock_terminal_model.simulate_long_running_block("barserver", "request bar\r\n");

        let last_block_index: BlockIndex = 2.into();

        let mut run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // In MostRecentLast the match order for block 2 is:
        //   [0] Output  row 0, col 8..=10   ("bar" in "request bar")
        //   [1] Prompt  row 0, col 0..=2    ("bar" in "barserver")
        // Navigate "Up" once to move from the output match to the prompt match.
        run.focus_next_match(FindDirection::Up, BlockSortDirection::MostRecentLast);
        let focused_before = run.focused_match().cloned();
        assert_eq!(
            focused_before,
            Some(BlockListMatch::CommandBlock(BlockGridMatch {
                grid_type: GridType::PromptAndCommand,
                range: Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
                block_index: last_block_index,
                is_filtered: false,
            }))
        );

        // Simulate more streaming output that introduces new output matches before the
        // prompt match in the match vector.
        mock_terminal_model.process_bytes("request bar\r\nrequest bar\r\n");

        let block = mock_terminal_model
            .block_list()
            .block_at(last_block_index)
            .unwrap();
        let run = run.rerun_on_block(block, last_block_index, BlockSortDirection::MostRecentLast);

        // The focused match must still be the same prompt span, even though new output
        // matches were inserted before it in the active block's match slice.
        assert_eq!(run.focused_match().cloned(), focused_before);
    });
}

/// Regression test for https://github.com/warpdotdev/warp/issues/9542
///
/// When the user is focused on a match in an older (finished) block and the active block
/// receives new streaming output, the focus must not drift to a different match.
#[test]
fn test_rerun_on_block_preserves_focused_match_in_older_block() {
    App::test((), |mut app| async move {
        let mut mock_terminal_model = TerminalModel::mock(None, None);
        mock_terminal_model.simulate_block("echo bar", "bar\r\n");
        mock_terminal_model.simulate_long_running_block("server", "request bar\r\n");

        let last_block_index: BlockIndex = 2.into();
        let older_block_index: BlockIndex = 1.into();

        let mut run = app.update(|ctx| {
            run_find_on_block_list(
                FindOptions {
                    query: Some("bar".to_owned().into()),
                    is_regex_enabled: false,
                    is_case_sensitive: false,
                    ..Default::default()
                },
                mock_terminal_model.block_list(),
                &HashMap::new(),
                BlockSortDirection::MostRecentLast,
                ctx,
            )
        });

        // Navigate past the active block's matches to reach block 1's output match.
        // Matches order (MostRecentLast): block 2 output, block 2 prompt ("server" has no
        // match), block 1 output, block 1 prompt.
        // Initial focus is at index 0 (block 2 output row 0).
        // "Up" in MostRecentLast moves toward older blocks (higher index).
        let match_count = run.all_matches().len();
        for _ in 0..match_count {
            if run
                .focused_match()
                .is_some_and(|m| m.matches_block(older_block_index))
            {
                break;
            }
            run.focus_next_match(FindDirection::Up, BlockSortDirection::MostRecentLast);
        }

        let focused_before = run.focused_match().cloned();
        assert!(
            focused_before
                .as_ref()
                .is_some_and(|m| m.matches_block(older_block_index)),
            "expected focus on block 1, got {focused_before:?}"
        );
        let ui_index_before = run.focused_match_index();

        // Simulate new streaming output in the active block.
        mock_terminal_model.process_bytes("request bar\r\nrequest bar\r\n");

        let block = mock_terminal_model
            .block_list()
            .block_at(last_block_index)
            .unwrap();
        let run = run.rerun_on_block(block, last_block_index, BlockSortDirection::MostRecentLast);

        // The focused match must still be the same span in block 1.
        assert_eq!(run.focused_match().cloned(), focused_before);
        // The UI index should have shifted to account for the newly inserted matches.
        assert_ne!(
            run.focused_match_index(),
            ui_index_before,
            "UI index should change when new matches are inserted before the focused match"
        );
    });
}
