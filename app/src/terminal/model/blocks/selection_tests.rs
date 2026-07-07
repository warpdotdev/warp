use float_cmp::assert_approx_eq;
use warpui::App;

use super::*;
use crate::ai::blocklist::agent_view::AgentViewState;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self, Handler as _, PreexecValue};
use crate::terminal::model::blocks::insert_block;
use crate::terminal::model::blocks::tests::{
    command_finished_and_precmd, input_string, new_bootstrapped_block_list,
};
use crate::terminal::model::test_utils;

#[test]
pub fn test_selection_range_cleared_when_block_finishes() {
    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    let bootstrapped_block_list_len = blocks.blocks().len();

    // Add two lines to the command grid and three to the output grid.
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());
    blocks.linefeed();
    blocks.linefeed();
    blocks.linefeed();

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 8.5);
    let semantic_selection = SemanticSelection::mock(false, "");

    let block_index = blocks.active_block_index();
    let block = blocks.block_at(block_index).expect("block should exist");
    let command_grid_offset = block.command_grid_offset();
    let output_grid_end_offset = block.output_grid_offset() + block.output_grid_displayed_height();

    // Create a selection that spans the command and output grids.
    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 0),
        SelectionType::Simple,
        Side::Right,
    );
    blocks.update_selection(BlockListPoint::new(output_grid_end_offset, 0), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 1)
    );
    // We subtract one because we want to target the last row of the grid, not
    // the line after it.
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_end_offset - 1., 0)
    );

    // Move the cursor up 2 lines and finish the block.
    blocks.move_up(2);
    command_finished_and_precmd(&mut blocks);

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 5.5);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    assert_eq!(
        blocks.blocks[bootstrapped_block_list_len - 1].prompt_and_command_number_of_rows(),
        2
    );
    assert_eq!(
        blocks.blocks[bootstrapped_block_list_len - 1]
            .output_grid()
            .len(),
        1
    );

    // Recompute the offset of the end of the output grid.  When we moved the
    // cursor up two lines, then finished the block, the block was truncated to
    // the position of the cursor.
    let block = blocks.block_at(block_index).expect("block should exist");
    let output_grid_end_offset = block.output_grid_offset() + block.output_grid_displayed_height();

    // The selection should shrink back down to the end of the output grid.  (We
    // subtract one because we want to target the last row of the grid, not the
    // line after it.)
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 1)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_end_offset - 1., 7)
    );
}

#[test]
pub fn test_selection_ranges_single_command_grid() {
    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    let size = *blocks.size();

    // Add two lines to the command grid and output grid in the first block.
    let block_index = insert_block(&mut blocks, "a\nb\n", "a\nb\n");

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 6.5);
    let semantic_selection = SemanticSelection::mock(false, "");

    let block = blocks.block_at(block_index).expect("block should exist");
    let command_grid_offset = block.command_grid_offset();

    // Create a selection that just spans the command grid in the second block.
    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 0),
        SelectionType::Simple,
        Side::Right,
    );

    blocks.update_selection(
        BlockListPoint::new(command_grid_offset + 1.01, 0),
        Side::Right,
    );

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 1)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(command_grid_offset + 1., 0)
    );

    blocks.clear_selection();

    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 3),
        SelectionType::Lines,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(command_grid_offset, 4), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the whole line.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 0)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(command_grid_offset, size.columns - 1)
    );

    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Add two lines to the command grid and three to the output grid in the
    // second block.
    let block_index = insert_block(&mut blocks, "foo\nbar\n", "foo\nbar\nbazz\n");
    assert_lines_approx_eq!(blocks.block_heights.summary().height, 7.5);

    let block = blocks.block_at(block_index).expect("block should exist");
    let command_grid_offset = block.command_grid_offset();

    let semantic_selection = SemanticSelection::mock(false, "");
    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 1),
        SelectionType::Semantic,
        Side::Right,
    );

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the semantic region.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 0)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(command_grid_offset, 2)
    );
}

#[test]
pub fn test_selection_ranges_single_output_grid() {
    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    let block_size = blocks.block_size();

    // Add two lines to the command grid and output grid in the first block.
    let block_index = insert_block(&mut blocks, "a\nb\n", "a\nb\n");

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 6.5);
    let semantic_selection = SemanticSelection::mock(false, "");

    let block = blocks.block_at(block_index).expect("block should exist");
    let output_grid_offset = block.output_grid_offset();

    // Create a selection that just spans the output grid.
    blocks.start_selection(
        BlockListPoint::new(output_grid_offset, 0),
        SelectionType::Simple,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(output_grid_offset + 1., 0), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(output_grid_offset, 1)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset + 1., 0)
    );

    blocks.clear_selection();

    blocks.start_selection(
        BlockListPoint::new(output_grid_offset, 3),
        SelectionType::Lines,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(output_grid_offset, 4), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the whole line.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(output_grid_offset, 0)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset, block_size.size.columns - 1)
    );

    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    let block_index = insert_block(&mut blocks, "foo\nbar\n", "foo\nbar\nbazz\n");

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 7.5);

    let block = blocks.block_at(block_index).expect("block should exist");
    assert_eq!(block.prompt_and_command_number_of_rows(), 2);
    assert_eq!(block.output_grid().len(), 3);

    let output_grid_offset = block.output_grid_offset();

    blocks.start_selection(
        BlockListPoint::new(output_grid_offset, 1),
        SelectionType::Semantic,
        Side::Right,
    );

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the semantic region.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(output_grid_offset, 0)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset, 2)
    );
}

#[test]
pub fn test_selection_ranges_across_grids() {
    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    let block_size = blocks.block_size();

    // Add three lines to the command grid and output grid in the first block.
    let block_index = insert_block(&mut blocks, "a\nb\n", "a\nb\n");

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 6.5);
    let semantic_selection = SemanticSelection::mock(false, "");

    let block = blocks.block_at(block_index).expect("block should exist");
    let command_grid_offset = block.command_grid_offset();
    let output_grid_offset = block.output_grid_offset();

    // Create a selection that just starts on the first row of the command grid and ends on the
    // second row of the output grid.
    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 0),
        SelectionType::Simple,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(output_grid_offset + 1., 5), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 1)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset + 1., 5)
    );

    blocks.clear_selection();

    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 3),
        SelectionType::Lines,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(output_grid_offset, 5), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the whole line on both ends.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 0)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset, block_size.size.columns - 1)
    );

    let mut blocks = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Add two lines to the command grid and three to the output grid in the
    // active block.
    let block_index = insert_block(&mut blocks, "foo\nbar\n", "foo\nbar\nbazz\n");

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 7.5);

    let block = blocks.block_at(block_index).expect("block should exist");
    let command_grid_offset = block.command_grid_offset();
    let output_grid_offset = block.output_grid_offset();

    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 1),
        SelectionType::Semantic,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(output_grid_offset, 1), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the semantic region.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 0)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset, 2)
    );

    blocks.clear_selection();

    blocks.start_selection(
        BlockListPoint::new(command_grid_offset, 5),
        SelectionType::Simple,
        Side::Right,
    );

    blocks.update_selection(BlockListPoint::new(output_grid_offset, 2), Side::Right);

    let selection_range = blocks.renderable_selection(&semantic_selection, false);
    assert!(selection_range.is_some());

    // Ensure the selection was expanded out to include the semantic region.
    let selection_range = selection_range.unwrap();
    assert_eq!(
        selection_range.first().start,
        BlockListPoint::new(command_grid_offset, 6)
    );
    assert_eq!(
        selection_range.first().end,
        BlockListPoint::new(output_grid_offset, 2)
    );
}

// Test that a selection outside of the active block doesn't change, even if the active block
// is truncated.
#[test]
fn test_selection_doesnt_change_if_not_within_active_truncated_block() {
    // Use a smaller scroll limit to test truncation.
    let mut block_size = test_utils::block_size();
    block_size.max_block_scroll_limit = 10;

    let mut blocks =
        new_bootstrapped_block_list(Some(block_size), None, ChannelEventListener::new_for_test());

    // Add two lines to the command grid and output grid in the first block.
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());
    blocks.linefeed();
    blocks.linefeed();

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 7.5);
    let semantic_selection = SemanticSelection::mock(false, "");

    // Start a selection within the command grid.
    blocks.start_selection(
        BlockListPoint::new(1., 0),
        SelectionType::Simple,
        Side::Left,
    );
    blocks.update_selection(BlockListPoint::new(1., 5), Side::Right);

    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(1., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(1., 5)
    );

    // Add a new block with enough lines that it's truncated.
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());
    for _ in 1..1000 {
        blocks.linefeed();
    }

    // Ensure selection remains unchanged.
    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(1., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(1., 5)
    );
}

// Test that a selection where the end is just contained within the active block is truncated.
#[test]
fn test_selection_changes_if_end_within_active_truncated_block() {
    // Use a smaller scroll limit to test truncation.
    let mut block_size = test_utils::block_size();
    block_size.max_block_scroll_limit = 10;

    let mut blocks =
        new_bootstrapped_block_list(Some(block_size), None, ChannelEventListener::new_for_test());

    // Add two lines to the command grid and output grid in the first block.
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());
    blocks.linefeed();
    blocks.linefeed();

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 7.5);
    let semantic_selection = SemanticSelection::mock(false, "");

    // Start a selection within the command grid.
    blocks.start_selection(
        BlockListPoint::new(1., 0),
        SelectionType::Simple,
        Side::Left,
    );
    blocks.update_selection(BlockListPoint::new(1., 5), Side::Right);

    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(1., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(1., 5)
    );

    // Add a new block with enough lines that it's truncated.
    command_finished_and_precmd(&mut blocks);
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());

    // Fill up the visible screen with blocks in the line.
    for _ in 1..18 {
        blocks.linefeed();
    }

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 29.);

    blocks.update_selection(BlockListPoint::new(27., 1), Side::Right);

    // Ensure selection remains unchanged.
    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(1., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(27., 1)
    );

    // Add 5 more lines to the block--this means the first lines will be truncated.
    for _ in 0..5 {
        blocks.linefeed();
    }

    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(1., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(24., 1)
    );

    // Add 20 more lines--at this point the selection will no longer be contained within the
    // output grid and should be at the end of the command grid instead.
    for _ in 0..20 {
        blocks.linefeed();
    }

    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(1., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(8.5, 6)
    );
}

#[test]
fn test_selection_is_removed_if_contained_within_active_truncated_block() {
    // Use a smaller scroll limit to test truncation.
    let mut block_size = test_utils::block_size();
    block_size.max_block_scroll_limit = 10;

    let mut blocks =
        new_bootstrapped_block_list(Some(block_size), None, ChannelEventListener::new_for_test());

    // Add two lines to the command grid and output grid in the first block.
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());
    blocks.linefeed();
    blocks.linefeed();

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_lines_approx_eq!(blocks.block_heights.summary().height, 7.5);

    // Add a new block with enough lines that it's truncated.
    command_finished_and_precmd(&mut blocks);
    blocks.start_active_block();

    blocks.linefeed();
    blocks.linefeed();

    blocks.preexec(PreexecValue::default());

    // Fill up the visible screen with blocks in the line.
    for _ in 1..18 {
        blocks.linefeed();
    }

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // The first block has a height of 6.5 and the second has a height of 22.5.
    assert_lines_approx_eq!(blocks.block_heights.summary().height, 29.);
    let semantic_selection = SemanticSelection::mock(false, "");

    // Start a selection within the command grid.
    blocks.start_selection(
        BlockListPoint::new(10., 0),
        SelectionType::Simple,
        Side::Left,
    );
    blocks.update_selection(BlockListPoint::new(12., 0), Side::Right);

    let expanded_selection = blocks
        .renderable_selection(&semantic_selection, false)
        .unwrap();
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().start,
        BlockListPoint::new(10., 0)
    );
    assert_approx_eq!(
        BlockListPoint,
        expanded_selection.first().end,
        BlockListPoint::new(12., 0)
    );

    // Add more lines to the block so none of the selection is contained within the block--the
    // selection should no longer exixt
    for _ in 1..18 {
        blocks.linefeed();
    }

    assert!(blocks
        .renderable_selection(&semantic_selection, false)
        .is_none());
}

#[test]
fn test_smart_selection_in_single_block() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            let block_index = insert_block(
                &mut block_list,
                "echo https://warp.dev/about hello/world.js\n",
                "https://warp.dev/about hello/world.js\n",
            );
            let block = block_list
                .block_at(block_index)
                .expect("block should exist");

            let command_grid_offset = block.command_grid_offset();

            let semantic_selection = SemanticSelection::mock(true, "");

            // Start a selection at the second "t" in "https", which has wrapped to the second
            // line of the command grid.
            block_list.start_selection(
                BlockListPoint::new(command_grid_offset + 1., 0),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(command_grid_offset + 1., 0),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("https://warp.dev/about".to_string())
            );
            block_list.clear_selection();

            // Start a selection at the "p" in "warp", which has wrapped to the third
            // line of the command grid.
            block_list.start_selection(
                BlockListPoint::new(command_grid_offset + 2.0, 2),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(command_grid_offset + 2.0, 2),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("https://warp.dev/about".to_string())
            );
            block_list.clear_selection();

            // Start a selection at the "a" in "about" and drag to the "e" in "hello";
            // this spans the 4th and 5th lines of the command grid.
            block_list.start_selection(
                BlockListPoint::new(command_grid_offset + 3.0, 1),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(command_grid_offset + 4.0, 1),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("https://warp.dev/about hello".to_string())
            );
            block_list.clear_selection();

            // Start a selection at the "e" in "hello" and drag to the "o" in "about";
            // this goes from the 5th line of the command grid back to the 4th.
            block_list.start_selection(
                BlockListPoint::new(command_grid_offset + 4.0, 1),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(command_grid_offset + 3.0, 3),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("about hello/world.js".to_string())
            );
            block_list.clear_selection();
        })
    })
}

#[test]
fn test_smart_selection_in_multiple_blocks() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            let first_block_index = insert_block(
                &mut block_list,
                "echo https://warp.dev/about hello/world.js\n",
                "https://warp.dev/about hello/world.js\n",
            );
            let second_block_index =
                insert_block(&mut block_list, "echo 192.168.0.1\n", "192.168.0.1\n");

            let first_block = block_list
                .block_at(first_block_index)
                .expect("block should exist");
            let second_block = block_list
                .block_at(second_block_index)
                .expect("block should exist");

            let first_command_grid_offset = first_block.command_grid_offset();
            let first_output_grid_offset = first_block.output_grid_offset();
            let first_block_height = first_block.height(&AgentViewState::Inactive);
            let second_command_grid_offset =
                first_block_height + second_block.command_grid_offset();
            let second_output_grid_offset = first_block_height + second_block.output_grid_offset();

            let semantic_selection = SemanticSelection::mock(true, "");

            // Start a selection at the second "t" in "https" in the 1st command (which
            // has wrapped to the second line of the command grid) to the "h" in the
            // "https" in the 1st output grid.
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset + 1., 0),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(first_output_grid_offset, 0),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("https://warp.dev/about hello/world.js\nhttps".to_string())
            );
            block_list.clear_selection();

            // Start a selection at "e" in "hello" in the 1st command (which has wrapped
            // to the third line of the command grid) to the "6" in the "168" in
            // the 2nd output.
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset + 4.0, 1),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(second_output_grid_offset, 5),
                Side::Right,
            );

            assert_eq!(
        block_list.selection_to_string(&semantic_selection, false, ctx),
        Some(
            "hello/world.js\nhttps://warp.dev/about hello/world.js\necho 192.168.0.1\n192.168"
                .to_string()
        )
    );
            block_list.clear_selection();

            // Start a selection at "0" in the 2nd command to the "e" in "dev" in the
            // 1st output (which has wrapped to the third line of the output grid).
            block_list.start_selection(
                BlockListPoint::new(second_command_grid_offset, 5),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(first_output_grid_offset + 2., 0),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("dev/about hello/world.js\necho 192.168.0.1".to_string())
            );
            block_list.clear_selection();
        })
    })
}

#[test]
fn test_semantic_selection_with_custom_boundaries() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            let block_index = insert_block(
                &mut block_list,
                "echo localhost:3000/foo/bar",
                "localhost:3000/foo/bar",
            );

            let semantic_selection = SemanticSelection::mock(false, ":");

            let output_grid_offset = block_list
                .block_at(block_index)
                .expect("created a block above")
                .output_grid_offset();

            // Start a selection at the "c" in "localhost"
            block_list.start_selection(
                BlockListPoint::new(output_grid_offset, 0),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(BlockListPoint::new(output_grid_offset, 0), Side::Right);

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("localhost:3000".to_string())
            );
            block_list.clear_selection();
        })
    })
}

#[test]
fn test_smart_selection_override() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            let block_index = insert_block(
                &mut block_list,
                "echo https://warp.dev/about hello/world",
                "https://warp.dev/about hello/world",
            );

            // the override wraps "https://warp.dev/about hello/world"
            block_list.set_smart_select_override(WithinBlock::new(
                Point::new(0, 5)..=Point::new(5, 3),
                block_index,
                GridType::PromptAndCommand,
            ));

            let semantic_selection = SemanticSelection::mock(true, "");

            // Start a selection at the "w" in "warp"
            // TODO(vorporeal): this comment doesn't seem to match the code
            block_list.start_selection(
                BlockListPoint::new(2.0, 6),
                SelectionType::Semantic,
                Side::Left,
            );
            block_list.update_selection(BlockListPoint::new(2.0, 0), Side::Right);

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("https://warp.dev/about hello/world".to_string())
            );
        })
    })
}

#[test]
pub fn test_selection_to_string() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
            let bootstrapped_block_list_len = block_list.blocks().len();

            // Create two blocks, each with 3 command lines and 3 output lines.
            let first_block_index =
                insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");
            let second_block_index =
                insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");

            let first_block = block_list
                .block_at(first_block_index)
                .expect("block should exist");
            let second_block = block_list
                .block_at(second_block_index)
                .expect("block should exist");

            // We created two blocks.
            assert_eq!(block_list.blocks.len(), bootstrapped_block_list_len + 2);

            assert_eq!(first_block.prompt_and_command_number_of_rows(), 3);
            assert_eq!(first_block.output_grid().len(), 3);

            assert_eq!(second_block.prompt_and_command_number_of_rows(), 3);
            assert_eq!(second_block.output_grid().len(), 3);

            assert_lines_approx_eq!(first_block.height(&AgentViewState::Inactive), 8.5);
            assert_lines_approx_eq!(second_block.height(&AgentViewState::Inactive), 8.5);
            let semantic_selection = SemanticSelection::mock(false, "");

            // Save some positions for later use.
            let first_command_grid_offset = first_block.command_grid_offset();
            let first_output_grid_offset = first_block.output_grid_offset();
            let first_block_height = first_block.height(&AgentViewState::Inactive);
            let second_command_grid_offset =
                first_block_height + second_block.command_grid_offset();
            let second_output_grid_offset = first_block_height + second_block.output_grid_offset();

            // Create a selection that just spans the first command grid.
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset, 0),
                SelectionType::Simple,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(first_command_grid_offset, 3),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("foo".to_string())
            );

            // Create a selection that just starts at the first command grid and ends at the output
            // grid.
            block_list.clear_selection();
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset, 0),
                SelectionType::Simple,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(first_output_grid_offset, 3),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("foo\nbar\nbazz\nfoo".to_string())
            );

            // Create a selection that spans from command grid of the first block to command grid of the
            // second block.
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset, 0),
                SelectionType::Simple,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(second_command_grid_offset, 3),
                Side::Right,
            );
            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("foo\nbar\nbazz\nfoo\nbar\nbazz\nfoo".to_string())
            );

            // Create a selection that spans from command grid of the first block to output grid of the
            // second block.
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset, 0),
                SelectionType::Simple,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(second_output_grid_offset, 3),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some(format!("{}{}", "foo\nbar\nbazz\n".repeat(3), "foo"))
            );

            // Create a selection that starts in the output grid of the first block and ends in the
            // command grid of the second block.
            block_list.start_selection(
                BlockListPoint::new(first_output_grid_offset, 0),
                SelectionType::Simple,
                Side::Left,
            );
            block_list.update_selection(
                BlockListPoint::new(second_command_grid_offset, 3),
                Side::Right,
            );
            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("foo\nbar\nbazz\nfoo".to_string())
            )
        })
    })
}

#[test]
pub fn test_selection_to_string_inverted_blocklist() {
    App::test((), |app| async move {
        app.read(|ctx| {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Create 4 blocks, A to D
    insert_block(&mut block_list, "block A input\n", "block A output\n");
    insert_block(&mut block_list, "block B input\n", "block B output\n");
    insert_block(&mut block_list, "block C input\n", "block C output\n");
    insert_block(&mut block_list, "block D input\n", "block D output\n");

    let semantic_selection = SemanticSelection::mock(false, "");

    // Create a selection that spans the first two blocks (the bottommost ones)
    let start = BlockListPoint::from_within_block_point(
        &WithinBlock::<Point> {
            block_index: 3.into(),
            grid: GridType::PromptAndCommand,
            inner: Point { row: 0, col: 0 },
        },
        &block_list,
    );
    let end = BlockListPoint::from_within_block_point(
        &WithinBlock::<Point> {
            block_index: 2.into(),
            grid: GridType::Output,
            inner: Point { row: 1, col: 8 },
        },
        &block_list,
    );

    block_list.start_selection(start, SelectionType::Simple, Side::Left);
    block_list.update_selection(end, Side::Right);

    assert_eq!(
        block_list.selection_to_string(&semantic_selection, true, ctx),
        Some("block B input\nblock B output\nblock A input\nblock A output".to_string())
    );

    // Create a selection that spans all blocks
    let start = BlockListPoint::from_within_block_point(
        &WithinBlock::<Point> {
            block_index: 5.into(),
            grid: GridType::PromptAndCommand,
            inner: Point { row: 0, col: 0 },
        },
        &block_list,
    );
    let end = BlockListPoint::from_within_block_point(
        &WithinBlock::<Point> {
            block_index: 2.into(),
            grid: GridType::Output,
            inner: Point { row: 1, col: 8 },
        },
        &block_list,
    );

    block_list.start_selection(start, SelectionType::Simple, Side::Left);
    block_list.update_selection(end, Side::Right);

    assert_eq!(
            block_list.selection_to_string(&semantic_selection, true, ctx),
            Some("block D input\nblock D output\nblock C input\nblock C output\nblock B input\nblock B output\nblock A input\nblock A output".to_string())
        );
    })
    })
}

#[test]
pub fn test_select_left_select_right() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Create two blocks, each with 3 command lines and 3 output lines.
    insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");
    insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\na鳥a鸟a\n");

    let semantic_selection = SemanticSelection::mock(false, "");
    // Within a row
    block_list.start_selection(
        BlockListPoint::new(1.0, 0),
        SelectionType::Simple,
        Side::Left,
    );
    block_list.update_selection(BlockListPoint::new(1.0, 3), Side::Right);

    block_list.select_text_left(&semantic_selection, false);
    assert_eq!(block_list.selection().unwrap().head.point.column, 0);
    assert_eq!(block_list.selection().unwrap().tail.point.column, 2);
    block_list.select_text_right(&semantic_selection, false);
    assert_eq!(block_list.selection().unwrap().head.point.column, 0);
    assert_eq!(block_list.selection().unwrap().tail.point.column, 3);
    block_list.clear_selection();

    // At row boundaries
    block_list.start_selection(
        BlockListPoint::new(5.0, 1),
        SelectionType::Simple,
        Side::Left,
    );
    block_list.update_selection(BlockListPoint::new(5.0, 2), Side::Right);
    for _ in 0..5 {
        block_list.select_text_right(&semantic_selection, false);
    }
    assert_eq!(block_list.selection().unwrap().tail.point.column, 0);
    assert_lines_approx_eq!(block_list.selection().unwrap().tail.point.row, 6.0);
    block_list.select_text_left(&semantic_selection, false);
    assert_eq!(block_list.selection().unwrap().tail.point.column, 6);
    assert_lines_approx_eq!(block_list.selection().unwrap().tail.point.row, 5.0);

    // Across wide characters
    block_list.start_selection(
        BlockListPoint::new(12.0, 0),
        SelectionType::Simple,
        Side::Left,
    );
    block_list.update_selection(BlockListPoint::new(15.5, 1), Side::Right);
    block_list.select_text_right(&semantic_selection, false);
    assert_eq!(block_list.selection().unwrap().tail.point.column, 3);
    block_list.select_text_left(&semantic_selection, false);
    assert_eq!(block_list.selection().unwrap().tail.point.column, 1);
}

#[test]
pub fn test_selection_to_string_hidden_blocks() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
            let bootstrapped_block_list_len = block_list.blocks().len();

            // Create a regular block with 1 command line and 1 output line
            block_list.start_active_block();
            input_string(&mut block_list, "before");
            block_list.carriage_return();
            block_list.linefeed();
            block_list.preexec(Default::default());
            input_string(&mut block_list, "foo");
            block_list.carriage_return();
            block_list.linefeed();

            // Simulate creating an SSH session
            block_list.reinit_shell();
            // Write some data to the bootstrap block, which should be hidden.
            input_string(&mut block_list, "this should be hidden and not copied");
            block_list.carriage_return();
            block_list.linefeed();
            command_finished_and_precmd(&mut block_list);

            // Simulate the login block
            block_list.start_active_block();
            block_list.preexec(Default::default());
            command_finished_and_precmd(&mut block_list);

            // Create another regular block with 1 command line and 1 output line
            insert_block(&mut block_list, "after\n", "bar\n");

            // There are 4 additional blocks: the block with command "before", the SSH
            // bootstrap block, the login block, and the final block.
            assert_eq!(block_list.blocks.len(), bootstrapped_block_list_len + 4);

            assert_eq!(
                block_list.blocks[bootstrapped_block_list_len - 1]
                    .prompt_and_command_number_of_rows(),
                1
            );
            assert_eq!(
                block_list.blocks[bootstrapped_block_list_len - 1]
                    .output_grid()
                    .len(),
                1
            );

            assert_eq!(
                block_list.blocks[bootstrapped_block_list_len + 2]
                    .prompt_and_command_number_of_rows(),
                1
            );
            assert_eq!(
                block_list.blocks[bootstrapped_block_list_len + 2]
                    .output_grid()
                    .len(),
                1
            );

            let semantic_selection = SemanticSelection::mock(false, "");
            // Create a selection that spans from command grid of the before block to output grid of the
            // after block.
            block_list.start_selection(
                BlockListPoint::new(1.0, 0),
                SelectionType::Simple,
                Side::Left,
            );
            block_list.update_selection(BlockListPoint::new(7., 3), Side::Right);

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("before\nfoo\nafter\nbar".into())
            );
        })
    })
}

#[test]
pub fn test_rect_selection_single_block() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut blocks =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            let block_index = insert_block(&mut blocks, "bef", "before\nfoo\nafter\nbar");
            let semantic_selection = SemanticSelection::mock(false, "");

            let block = blocks.block_at(block_index).expect("block should exist");
            let output_grid_offset = block.output_grid_offset();

            // The simple selection should go from the right of the first character in output grid (Side::Right) to the right of the end character
            // of the third row in the output grid.
            blocks.start_selection(
                BlockListPoint::new(output_grid_offset, 0),
                SelectionType::Simple,
                Side::Right,
            );

            blocks.update_selection(BlockListPoint::new(output_grid_offset + 2., 6), Side::Right);

            assert_eq!(
                blocks.selection_to_string(&semantic_selection, false, ctx),
                Some("efore\nfoo\nafter".to_string())
            );

            // Reversing the selection should still work.
            blocks.start_selection(
                BlockListPoint::new(output_grid_offset + 2., 6),
                SelectionType::Simple,
                Side::Right,
            );

            blocks.update_selection(BlockListPoint::new(output_grid_offset, 0), Side::Right);

            assert_eq!(
                blocks.selection_to_string(&semantic_selection, false, ctx),
                Some("efore\nfoo\nafter".to_string())
            );

            blocks.clear_selection();

            // The rect selection should go from the right of the first character in output grid (Side::Right) to the right of the end character
            // of the third row in the output grid.
            blocks.start_selection(
                BlockListPoint::new(output_grid_offset, 0),
                SelectionType::Rect,
                Side::Right,
            );

            blocks.update_selection(BlockListPoint::new(output_grid_offset + 2., 6), Side::Right);

            let selection_range = blocks.renderable_selection(&semantic_selection, false);
            assert!(selection_range.is_some());

            let selection_range = selection_range.unwrap();
            assert_eq!(selection_range.len(), 3);

            // No clamping needed for rect selection. The selection should span three rows with equal ending column.
            assert_eq!(
                selection_range.first().start,
                BlockListPoint::new(output_grid_offset, 1)
            );
            assert_eq!(
                selection_range.first().end,
                BlockListPoint::new(output_grid_offset, 6)
            );
            assert_eq!(
                selection_range[1].start,
                BlockListPoint::new(output_grid_offset + 1., 1)
            );
            assert_eq!(
                selection_range[1].end,
                BlockListPoint::new(output_grid_offset + 1., 6)
            );
            assert_eq!(
                selection_range[2].start,
                BlockListPoint::new(output_grid_offset + 2., 1)
            );
            assert_eq!(
                selection_range[2].end,
                BlockListPoint::new(output_grid_offset + 2., 6)
            );

            assert_eq!(
                blocks.selection_to_string(&semantic_selection, false, ctx),
                Some("efore\noo\nfter".to_string())
            );

            blocks.clear_selection();

            // The rect selection should go from the left of the fourth character in output grid (Side::Right) to the right of the fifth character
            // of the third row in the output grid.
            blocks.start_selection(
                BlockListPoint::new(output_grid_offset, 3),
                SelectionType::Rect,
                Side::Left,
            );

            blocks.update_selection(BlockListPoint::new(output_grid_offset + 2., 4), Side::Right);

            let selection_range = blocks.renderable_selection(&semantic_selection, false);
            assert!(selection_range.is_some());

            let selection_range = selection_range.unwrap();
            assert_eq!(selection_range.len(), 3);

            // No clamping needed for rect selection. The selection should span the three rows with equal ending column.
            assert_eq!(
                selection_range.first().start,
                BlockListPoint::new(output_grid_offset, 3)
            );
            assert_eq!(
                selection_range.first().end,
                BlockListPoint::new(output_grid_offset, 4)
            );
            assert_eq!(
                selection_range[1].start,
                BlockListPoint::new(output_grid_offset + 1., 3)
            );
            assert_eq!(
                selection_range[1].end,
                BlockListPoint::new(output_grid_offset + 1., 4)
            );
            assert_eq!(
                selection_range[2].start,
                BlockListPoint::new(output_grid_offset + 2., 3)
            );
            assert_eq!(
                selection_range[2].end,
                BlockListPoint::new(output_grid_offset + 2., 4)
            );

            assert_eq!(
                blocks.selection_to_string(&semantic_selection, false, ctx),
                Some("or\n\ner".to_string())
            );

            blocks.clear_selection();

            // The rect selection this time should go from bottom left to top right. The selected content should remain the same.
            blocks.start_selection(
                BlockListPoint::new(output_grid_offset + 2., 3),
                SelectionType::Rect,
                Side::Right,
            );

            blocks.update_selection(BlockListPoint::new(output_grid_offset, 4), Side::Left);

            let selection_range = blocks.renderable_selection(&semantic_selection, false);
            assert!(selection_range.is_some());

            let selection_range = selection_range.unwrap();
            assert_eq!(selection_range.len(), 3);

            // No clamping needed for rect selection. The selection should span the three rows with equal ending column.
            assert_eq!(
                selection_range.first().start,
                BlockListPoint::new(output_grid_offset, 3)
            );
            assert_eq!(
                selection_range.first().end,
                BlockListPoint::new(output_grid_offset, 4)
            );
            assert_eq!(
                selection_range[1].start,
                BlockListPoint::new(output_grid_offset + 1., 3)
            );
            assert_eq!(
                selection_range[1].end,
                BlockListPoint::new(output_grid_offset + 1., 4)
            );
            assert_eq!(
                selection_range[2].start,
                BlockListPoint::new(output_grid_offset + 2., 3)
            );
            assert_eq!(
                selection_range[2].end,
                BlockListPoint::new(output_grid_offset + 2., 4)
            );

            assert_eq!(
                blocks.selection_to_string(&semantic_selection, false, ctx),
                Some("or\n\ner".to_string())
            );
        })
    })
}

#[test]
pub fn test_rect_selection_multi_block() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            // Create two blocks.
            let first_block_index = insert_block(&mut block_list, "first\n", "line\n");
            let second_block_index = insert_block(&mut block_list, "second\n", "line\n");

            let first_block = block_list
                .block_at(first_block_index)
                .expect("block should exist");
            let second_block = block_list
                .block_at(second_block_index)
                .expect("block should exist");
            let semantic_selection = SemanticSelection::mock(false, "");

            // Save some positions for later use.
            let first_command_grid_offset = first_block.command_grid_offset();
            let first_block_height = first_block.height(&AgentViewState::Inactive);
            let second_output_grid_offset = first_block_height + second_block.output_grid_offset();

            // Start a selection at the start of the line in the first command grid.
            block_list.start_selection(
                BlockListPoint::new(first_command_grid_offset, 0),
                SelectionType::Rect,
                Side::Left,
            );
            // Select four characters.
            block_list.update_selection(
                BlockListPoint::new(second_output_grid_offset, 3),
                Side::Right,
            );

            assert_eq!(
                block_list.selection_to_string(&semantic_selection, false, ctx),
                Some("firs\nline\nseco\nline".to_string())
            );
        })
    })
}

#[test]
pub fn test_rect_selection_inverted_multi_block() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

            // Create 4 blocks, A to D
            insert_block(&mut block_list, "block A input\n", "block A output\n");
            insert_block(&mut block_list, "block B input\n", "block B output\n");
            insert_block(&mut block_list, "block C input\n", "block C output\n");
            insert_block(&mut block_list, "block D input\n", "block D output\n");

            let start = BlockListPoint::from_within_block_point(
                &WithinBlock::<Point> {
                    block_index: 3.into(),
                    grid: GridType::PromptAndCommand,
                    inner: Point { row: 0, col: 4 },
                },
                &block_list,
            );
            let end = BlockListPoint::from_within_block_point(
                &WithinBlock::<Point> {
                    block_index: 2.into(),
                    grid: GridType::Output,
                    inner: Point { row: 1, col: 6 },
                },
                &block_list,
            );

            let semantic_selection = SemanticSelection::mock(false, "");

            block_list.start_selection(start, SelectionType::Rect, Side::Left);
            block_list.update_selection(end, Side::Right);

            // Blocks are softwrapped so the rect selection will include multiple segment of a logical line. Notice that the blocks are reversed.
            // bloc|k B|
            //  inp|ut |
            // bloc|k B|
            //  out|put|
            // bloc|k A|
            //  inp|ut |
            // bloc|k A|
            //  out|put|
            assert_eq!(
                block_list.selection_to_string(&semantic_selection, true, ctx),
                Some("k B\nut\nk B\nput\nk A\nut\nk A\nput".to_string())
            );
        })
    })
}
