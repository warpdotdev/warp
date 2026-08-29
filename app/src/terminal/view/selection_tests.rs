use warpui::App;
use warpui::text::SelectionType;

use super::*;
use crate::assert_lines_approx_eq;
use crate::terminal::GridType;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::Handler as _;
use crate::terminal::model::blocks::{
    BlockListPoint, command_finished_and_precmd, input_string, insert_block,
    new_bootstrapped_block_list, start_active_block,
};
use crate::terminal::model::index::{Point, Side};
use crate::terminal::model::terminal_model::WithinBlock;

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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
            let first_block_height =
                first_block.height(&crate::terminal::model::block::TranscriptScope::Terminal);
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
        block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
            assert_eq!(block_list.blocks().len(), bootstrapped_block_list_len + 2);

            assert_eq!(first_block.prompt_and_command_number_of_rows(), 3);
            assert_eq!(first_block.output_grid().len(), 3);

            assert_eq!(second_block.prompt_and_command_number_of_rows(), 3);
            assert_eq!(second_block.output_grid().len(), 3);

            assert_lines_approx_eq!(
                first_block.height(&crate::terminal::model::block::TranscriptScope::Terminal),
                8.5
            );
            assert_lines_approx_eq!(
                second_block.height(&crate::terminal::model::block::TranscriptScope::Terminal),
                8.5
            );
            let semantic_selection = SemanticSelection::mock(false, "");

            // Save some positions for later use.
            let first_command_grid_offset = first_block.command_grid_offset();
            let first_output_grid_offset = first_block.output_grid_offset();
            let first_block_height =
                first_block.height(&crate::terminal::model::block::TranscriptScope::Terminal);
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
        block_list_selection_to_string(&block_list, &semantic_selection, true, ctx),
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
            block_list_selection_to_string(&block_list, &semantic_selection, true, ctx),
            Some("block D input\nblock D output\nblock C input\nblock C output\nblock B input\nblock B output\nblock A input\nblock A output".to_string())
        );
    })
    })
}

#[test]
pub fn test_selection_to_string_hidden_blocks() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut block_list =
                new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
            let bootstrapped_block_list_len = block_list.blocks().len();

            // Create a regular block with 1 command line and 1 output line
            start_active_block(&mut block_list);
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
            start_active_block(&mut block_list);
            block_list.preexec(Default::default());
            command_finished_and_precmd(&mut block_list);

            // Create another regular block with 1 command line and 1 output line
            insert_block(&mut block_list, "after\n", "bar\n");

            // There are 4 additional blocks: the block with command "before", the SSH
            // bootstrap block, the login block, and the final block.
            assert_eq!(block_list.blocks().len(), bootstrapped_block_list_len + 4);

            assert_eq!(
                block_list.blocks()[bootstrapped_block_list_len - 1]
                    .prompt_and_command_number_of_rows(),
                1
            );
            assert_eq!(
                block_list.blocks()[bootstrapped_block_list_len - 1]
                    .output_grid()
                    .len(),
                1
            );

            assert_eq!(
                block_list.blocks()[bootstrapped_block_list_len + 2]
                    .prompt_and_command_number_of_rows(),
                1
            );
            assert_eq!(
                block_list.blocks()[bootstrapped_block_list_len + 2]
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&blocks, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&blocks, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&blocks, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&blocks, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&blocks, &semantic_selection, false, ctx),
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
            let first_block_height =
                first_block.height(&crate::terminal::model::block::TranscriptScope::Terminal);
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
                block_list_selection_to_string(&block_list, &semantic_selection, false, ctx),
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
                block_list_selection_to_string(&block_list, &semantic_selection, true, ctx),
                Some("k B\nut\nk B\nput\nk A\nut\nk A\nput".to_string())
            );
        })
    })
}
