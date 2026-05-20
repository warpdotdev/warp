use std::{collections::HashMap, pin::pin, time::Duration};

use super::*;
use crate::{
    ai::blocklist::agent_view::AgentViewState,
    terminal::model::{
        ansi::{Attr, Handler},
        cell::Flags,
        header_grid::PromptEndPoint,
        session::SessionInfo,
        test_utils::{create_test_block_with_grids, TestBlockBuilder},
    },
    test_util::mock_blockgrid,
};
use chrono::TimeZone;
use float_cmp::assert_approx_eq;
use futures_lite::stream::StreamExt;

impl float_cmp::ApproxEq for BlockSection {
    type Margin = float_cmp::F64Margin;

    fn approx_eq<M: Into<Self::Margin>>(self, other: Self, margin: M) -> bool {
        // Use a minimum epsilon value of 3 * the smallest value that can be
        // represented in an f64.  This allows for a very small (but non-zero)
        // amount of _accumulated_ floating-point error.
        let mut margin: Self::Margin = margin.into();
        if margin.epsilon < f64::EPSILON * 3. {
            margin = margin.epsilon(f64::EPSILON * 3.);
        }

        match (self, other) {
            (BlockSection::PromptAndCommandGrid(r1), BlockSection::PromptAndCommandGrid(r2)) => {
                r1.approx_eq(r2, margin)
            }
            (BlockSection::OutputGrid(r1), BlockSection::OutputGrid(r2)) => {
                r1.approx_eq(r2, margin)
            }
            (a, b) => std::mem::discriminant(&a) == std::mem::discriminant(&b),
        }
    }
}

#[test]
pub fn test_find() {
    let mut block = TestBlockBuilder::new().build();

    block.precmd(PrecmdValue::default());
    block.start();
    assert_lines_approx_eq!(block.height(&AgentViewState::Inactive), 3.);

    assert_approx_eq!(
        BlockSection,
        block.find(Lines::zero()),
        BlockSection::PaddingTop
    );
    assert_approx_eq!(
        BlockSection,
        block.find(0.5.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(1.0.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(1.5.into_lines()),
        BlockSection::PromptAndCommandGrid(0.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(2.0.into_lines()),
        BlockSection::PaddingBottom
    );
    assert_approx_eq!(
        BlockSection,
        block.find(3.5.into_lines()),
        BlockSection::NotContained
    );

    // Add a few lines to the command grid.
    block.header_grid.command_grid_linefeed();
    block.header_grid.command_grid_linefeed();
    block.header_grid.command_grid_linefeed();

    assert_lines_approx_eq!(block.height(&AgentViewState::Inactive), 6.);

    assert_approx_eq!(
        BlockSection,
        block.find(Lines::zero()),
        BlockSection::PaddingTop
    );
    assert_approx_eq!(
        BlockSection,
        block.find(0.5.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(1.0.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(1.5.into_lines()),
        BlockSection::PromptAndCommandGrid(0.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(2.01.into_lines()),
        BlockSection::PromptAndCommandGrid(1.01.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(2.5.into_lines()),
        BlockSection::PromptAndCommandGrid(1.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(3.5.into_lines()),
        BlockSection::PromptAndCommandGrid(2.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(4.5.into_lines()),
        BlockSection::PromptAndCommandGrid(3.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(5.0.into_lines()),
        BlockSection::PaddingBottom
    );
    assert_approx_eq!(
        BlockSection,
        block.find(8.0.into_lines()),
        BlockSection::NotContained
    );

    // Add lines to the output grid. The command grid will now shrink to be 3 lines and the
    // output grid will be 2 lines.
    block.header_grid.finish_command_grid();
    block.output_grid.start();
    block.output_grid.linefeed();
    block.output_grid.linefeed();
    block
        .output_grid
        .on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(block.header_grid.prompt_and_command_number_of_rows(), 3);
    assert_eq!(block.output_grid.len(), 3);
    assert_lines_approx_eq!(block.height(&AgentViewState::Inactive), 8.5);

    assert_approx_eq!(
        BlockSection,
        block.find(Lines::zero()),
        BlockSection::PaddingTop
    );
    assert_approx_eq!(
        BlockSection,
        block.find(0.5.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(0.8.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(1.0.into_lines()),
        BlockSection::PromptAndCommandGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(1.5.into_lines()),
        BlockSection::PromptAndCommandGrid(0.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(2.01.into_lines()),
        BlockSection::PromptAndCommandGrid(1.01.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(2.5.into_lines()),
        BlockSection::PromptAndCommandGrid(1.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(3.5.into_lines()),
        BlockSection::PromptAndCommandGrid(2.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(4.0.into_lines()),
        BlockSection::PaddingMiddle
    );
    assert_approx_eq!(
        BlockSection,
        block.find(4.5.into_lines()),
        BlockSection::OutputGrid(Lines::zero())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(5.5.into_lines()),
        BlockSection::OutputGrid(1.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(6.0.into_lines()),
        BlockSection::OutputGrid(1.5.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(6.5.into_lines()),
        BlockSection::OutputGrid(2.into_lines())
    );
    assert_approx_eq!(
        BlockSection,
        block.find(7.5.into_lines()),
        BlockSection::PaddingBottom
    );
    assert_approx_eq!(
        BlockSection,
        block.find(8.5.into_lines()),
        BlockSection::NotContained
    );
    assert_approx_eq!(
        BlockSection,
        block.find(10.5.into_lines()),
        BlockSection::NotContained
    );

    let mut block = TestBlockBuilder::new().build();

    block.precmd(PrecmdValue::default());
    block.start();

    block.header_grid.command_grid_linefeed();
    block.header_grid.command_grid_linefeed();
    block.header_grid.finish_command_grid();

    block.output_grid.start();
    block.output_grid.linefeed();
    block.output_grid.linefeed();
    block
        .output_grid
        .on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(block.header_grid.prompt_and_command_number_of_rows(), 2);
    assert_eq!(block.output_grid.len(), 3);
    assert_lines_approx_eq!(block.height(&AgentViewState::Inactive), 7.5);

    assert_approx_eq!(
        BlockSection,
        block.find(3.5.into_lines()),
        BlockSection::OutputGrid(Lines::zero())
    );
}

#[test]
pub fn test_long_running_block_bottom_padding() {
    warpui::r#async::block_on(async {
        let mut block = TestBlockBuilder::new().build();

        block.precmd(Default::default());
        block.start();
        for c in "command".chars() {
            block.input(c);
        }
        block.preexec(Default::default());
        for c in "output".chars() {
            block.input(c);
        }

        // Initially, the padding is still there because we aren't yet in long-running mode
        assert!(!block.is_active_and_long_running());
        assert!(block.padding_bottom() > LONG_RUNNING_BOTTOM_PADDING_LINES.into_lines());

        // After the long running duration, the block should switch
        let duration = LONG_RUNNING_COMMAND_DURATION_MS + 1;
        warpui::r#async::Timer::after(Duration::from_millis(duration)).await;

        assert!(block.is_active_and_long_running());
        assert!(block.padding_bottom() == LONG_RUNNING_BOTTOM_PADDING_LINES.into_lines());
    });
}

// Tests that the command grid has a non-zero height even if `preexec` is never called.
#[test]
pub fn test_precmd_no_preexec() {
    let mut block = TestBlockBuilder::new().build();
    block.start();
    block.precmd(PrecmdValue::default());

    for c in "command".chars() {
        block.input(c);
    }
    block.carriage_return();
    block.linefeed();

    for c in "error".chars() {
        block.input(c);
    }
    block.finish(0);

    assert_eq!("command\nerror", block.contents_to_string());
}

// Tests that the command grid text is all bolded.
#[test]
pub fn test_command_grid_bold() {
    let mut block = TestBlockBuilder::new().build();
    block.start();
    block.precmd(PrecmdValue::default());

    // We should have the BOLD flag enabled for commands, once we've started the command grid.
    assert!(block
        .header_grid
        .command_cursor_flags()
        .contains(Flags::BOLD));

    for c in "command".chars() {
        block.input(c);
    }

    block.finish(0);

    assert_eq!("command", block.contents_to_string());
}

// Tests that the command grid text is still bolded after RESET attribute.
#[test]
pub fn test_command_grid_bold_after_reset() {
    let mut block = TestBlockBuilder::new().build();
    block.start();
    block.precmd(PrecmdValue::default());

    // We should have the BOLD flag enabled for commands, once we've started the command grid.
    assert!(block
        .header_grid
        .command_cursor_flags()
        .contains(Flags::BOLD));

    for c in "command".chars() {
        block.input(c);
    }
    // Even after a Reset, we should still re-enable the BOLD flag.
    block.terminal_attribute(Attr::Reset);
    assert!(block
        .header_grid
        .command_cursor_flags()
        .contains(Flags::BOLD));

    block.finish(0);

    assert_eq!("command", block.contents_to_string());
}

// Tests that the command grid has non-zero height even if the shell does not echo a linefeed
// after the user inputs an empty command (this happens in recent Bash versions)
#[test]
pub fn test_empty_command() {
    let mut block = TestBlockBuilder::new().build();
    block.start();
    block.precmd(PrecmdValue::default());

    block.finish(0);

    assert!(!block.is_command_empty());
}

#[test]
pub fn test_failed_block() {
    let mut block = TestBlockBuilder::new().build();

    block.precmd(PrecmdValue::default());
    block.preexec(Default::default());

    block.finish(1 /* exit_code */);
    assert!(block.has_failed());

    let mut block = TestBlockBuilder::new().build();
    block.precmd(PrecmdValue::default());
    block.finish(1 /* exit_code */);

    // The block should not be marked as failed since execution never started.
    assert!(!block.has_failed());
}

#[test]
fn test_non_error_exit_codes() {
    let mut block = TestBlockBuilder::new().build();

    block.precmd(Default::default());
    block.preexec(Default::default());

    block.finish(130 /* exit_code */);
    // The block should not be marked as failed since it was interrupted by user action (SIGINT)
    assert!(!block.has_failed());

    let mut block = TestBlockBuilder::new().build();

    block.precmd(Default::default());
    block.preexec(Default::default());

    block.finish(141 /* exit_code */);
    // The block should not be marked as failed since it was interrupted by a pipe closing (SIGPIPE)
    assert!(!block.has_failed());
}

#[test]
pub fn test_block_height_non_bootstrapped_block() {
    let mut block = TestBlockBuilder::new().build();

    // Add a few lines to the grid.
    block.linefeed();
    block.linefeed();
    block.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // The block is empty since it was never started.
    assert!(block.is_empty(&AgentViewState::Inactive));

    block.start();

    // The block should be non-empty even though it wasn't bootstrapped.
    assert_lines_approx_eq!(block.height(&AgentViewState::Inactive), 5.);
}

#[test]
fn test_background_block() {
    let mut block = TestBlockBuilder::new().build();
    block.set_honor_ps1(true);

    block.start_background(None);
    assert_eq!(block.state(), BlockState::Background);

    for c in "background output".chars() {
        block.input(c);
    }
    block.carriage_return();
    block.linefeed();

    block.finish(0);

    assert_eq!(
        block.output_grid.contents_to_string(true, None),
        "background output\r\n"
    );

    // Background blocks have the usual top and bottom padding, but no
    // between-grid padding because there's only one grid.
    assert_lines_approx_eq!(block.output_grid_displayed_height(), 3);
    assert_lines_approx_eq!(block.height(&AgentViewState::Inactive), 4.2);
}

#[test]
pub fn test_block_duration_formatting() {
    // .01 seconds
    let d1 = chrono::Duration::milliseconds(10);
    assert_eq!(Block::format_duration(d1), " (0.01s)");

    // .123 seconds
    let d2 = chrono::Duration::milliseconds(123);
    assert_eq!(Block::format_duration(d2), " (0.123s)");

    // 3.143 seconds
    let d3 = chrono::Duration::milliseconds(3143);
    assert_eq!(Block::format_duration(d3), " (3.143s)");

    // 10.44 seconds
    let d4 = chrono::Duration::milliseconds(10440);
    assert_eq!(Block::format_duration(d4), " (10.44s)");

    // 25m 23.423 seconds
    let d5 = chrono::Duration::milliseconds(1523423);
    assert_eq!(Block::format_duration(d5), " (25m 23.42s)");

    // 3h 25m 8.133 seconds
    let d6 = chrono::Duration::milliseconds(12308133);
    assert_eq!(Block::format_duration(d6), " (3h 25m 8s)");

    // 22h 51m 48.902 seconds
    let d7 = chrono::Duration::milliseconds(82308902);
    assert_eq!(Block::format_duration(d7), " (22h 51m 49s)");

    let d8 = chrono::Duration::milliseconds(3604114);
    assert_eq!(Block::format_duration(d8), " (1h 4s)");
}

#[test]
pub fn test_elapsed_duration_rounds_down_to_whole_seconds() {
    let mut block = TestBlockBuilder::new().build();
    // Move the block into the Executing state so `elapsed_duration` returns a value.
    block.precmd(PrecmdValue::default());
    block.preexec(Default::default());

    let start = chrono::Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    block.override_start_ts(start);

    // Sub-second elapsed time is treated as "not yet elapsed", so nothing is rendered.
    let now_sub_second = start + chrono::Duration::milliseconds(500);
    assert_eq!(block.elapsed_duration_whole_secs_at(now_sub_second), None);

    // Whole-second elapsed time rounds cleanly.
    let now_1s = start + chrono::Duration::milliseconds(1_200);
    assert_eq!(
        block.elapsed_duration_whole_secs_at(now_1s),
        Some(chrono::Duration::seconds(1))
    );

    // Fractional milliseconds inside a second are dropped, keeping the formatted string stable
    // between one-second repaint ticks.
    let now_7s = start + chrono::Duration::milliseconds(7_950);
    assert_eq!(
        block.elapsed_duration_whole_secs_at(now_7s),
        Some(chrono::Duration::seconds(7))
    );

    // Multi-minute elapsed times also round down to whole seconds.
    let now_long = start + chrono::Duration::milliseconds(125_999);
    assert_eq!(
        block.elapsed_duration_whole_secs_at(now_long),
        Some(chrono::Duration::seconds(125))
    );
}

#[test]
pub fn test_elapsed_duration_requires_executing_state() {
    let mut block = TestBlockBuilder::new().build();
    let start = chrono::Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let now = start + chrono::Duration::seconds(5);

    // With no start_ts and no execution, there's nothing to show live.
    assert_eq!(block.elapsed_duration_whole_secs_at(now), None);
    assert!(!block.is_duration_live());

    // `precmd` alone leaves the block in `BeforeExecution`; the duration is not yet live.
    block.precmd(PrecmdValue::default());
    block.override_start_ts(start);
    assert_eq!(block.elapsed_duration_whole_secs_at(now), None);
    assert!(!block.is_duration_live());

    // `preexec` transitions the block to `Executing`; the duration should now be live.
    block.preexec(Default::default());
    assert_eq!(
        block.elapsed_duration_whole_secs_at(now),
        Some(chrono::Duration::seconds(5))
    );
    assert!(block.is_duration_live());

    // Once the block finishes, completed_ts is set and the duration is no longer live.
    block.finish(0);
    assert_eq!(block.elapsed_duration_whole_secs_at(now), None);
    assert!(!block.is_duration_live());
}

#[test]
pub fn test_elapsed_duration_for_background_block_is_not_live() {
    let mut block = TestBlockBuilder::new().build();
    let start = chrono::Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let now = start + chrono::Duration::seconds(10);

    block.start_background(None);
    block.override_start_ts(start);

    // Background blocks are not in `Executing` state, so we don't render a live counter for them.
    assert_eq!(block.state(), BlockState::Background);
    assert_eq!(block.elapsed_duration_whole_secs_at(now), None);
    assert!(!block.is_duration_live());
}

#[test]
pub fn test_block_emits_block_completed_event_for_in_band_command() {
    let (events_tx, events_rx) = async_channel::unbounded();

    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();
    let mut block = TestBlockBuilder::new()
        .with_event_proxy(event_proxy)
        .build();

    block.start_for_in_band_command();
    block.precmd(Default::default());
    block.preexec(PreexecValue {
        command: "warp_run_generator_command 1234 foo".to_owned(),
    });
    block.finish(0);

    events_rx.close();
    let block_completed_event =
        warpui::r#async::block_on(pin!(events_rx).find(|event| match event {
            Event::BlockCompleted(event) => matches!(event.block_type, BlockType::InBandCommand),
            _ => false,
        }));
    assert!(block_completed_event.is_some());
}

/// Tests the multiline lprompt and multiline command case for selecting text across grids.
#[test]
fn test_selection_bounds_all_grids_multiline_lprompt_command() {
    let block_index = BlockIndex::zero();
    // Grid contents:
    // -----
    // lprompt1
    // lprompt2%cmd1 rprompt
    // cmd2
    // cmd3
    // output1
    // output2
    // -----

    let mut prompt_and_command_grid = mock_blockgrid("lprompt1\r\nlprompt2%cmd1\r\ncmd2\r\ncmd3");
    prompt_and_command_grid.finish();
    let mut rprompt_grid = mock_blockgrid("rprompt");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("output1\r\noutput2\r\n");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    block.set_raw_prompt_end_point(Some(PromptEndPoint::PromptEnd {
        point: Point::new(1, 8),
        has_extra_trailing_newline: false,
    }));

    // Test cross-over across all sections (lprompt to output).

    let all_grids = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(2, 0)),
    );
    assert_eq!(
        all_grids,
        "lprompt1\nlprompt2%cmd1\nrprompt\ncmd2\ncmd3\noutput1\noutput2"
    );

    // Test individual sections.

    let lprompt_only = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 2)),
        BlockGridPoint::PromptAndCommand(Point::new(1, 3)),
    );
    assert_eq!(lprompt_only, "rompt1\nlpro");

    let rprompt_only = block.bounds_to_string(
        BlockGridPoint::Rprompt(Point::new(0, 1)),
        BlockGridPoint::Rprompt(Point::new(0, 4)),
    );
    assert_eq!(rprompt_only, "prom");

    let bottom_command_only = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(2, 1)),
        BlockGridPoint::PromptAndCommand(Point::new(3, 2)),
    );
    assert_eq!(bottom_command_only, "md2\ncmd");

    let output_only = block.bounds_to_string(
        BlockGridPoint::Output(Point::new(0, 3)),
        BlockGridPoint::Output(Point::new(0, 6)),
    );
    assert_eq!(output_only, "put1");

    // Test cross-overs across sections.

    let lprompt_to_rprompt = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 2)),
        BlockGridPoint::PromptAndCommand(Point::new(1, 2)),
    );
    assert_eq!(lprompt_to_rprompt, "rompt1\nlpr");

    let lprompt_to_bottom_command = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 2)),
        BlockGridPoint::PromptAndCommand(Point::new(3, 2)),
    );
    assert_eq!(
        lprompt_to_bottom_command,
        "rompt1\nlprompt2%cmd1\nrprompt\ncmd2\ncmd"
    );

    let rprompt_to_bottom_command = block.bounds_to_string(
        BlockGridPoint::Rprompt(Point::new(0, 0)),
        BlockGridPoint::PromptAndCommand(Point::new(3, 2)),
    );
    assert_eq!(rprompt_to_bottom_command, "rprompt\ncmd2\ncmd");

    let rprompt_to_output = block.bounds_to_string(
        BlockGridPoint::Rprompt(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(0, 3)),
    );
    assert_eq!(rprompt_to_output, "rprompt\ncmd2\ncmd3\noutp");

    let bottom_command_to_output = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(2, 1)),
        BlockGridPoint::Output(Point::new(1, 1)),
    );
    assert_eq!(bottom_command_to_output, "md2\ncmd3\noutput1\nou");

    // Test a few "impossible cases" and ensure they return an empty string.
    let impossible_output_to_rprompt = block.bounds_to_string(
        BlockGridPoint::Output(Point::new(1, 0)),
        BlockGridPoint::Rprompt(Point::new(1, 1)),
    );
    assert_eq!(impossible_output_to_rprompt, "");

    let impossible_output_to_lprompt = block.bounds_to_string(
        BlockGridPoint::Output(Point::new(1, 0)),
        BlockGridPoint::PromptAndCommand(Point::new(0, 1)),
    );
    assert_eq!(impossible_output_to_lprompt, "");
}

/// Tests the single line lprompt and command case for text selection across grids.
#[test]
fn test_selection_bounds_all_grids_single_line_lprompt_command() {
    let block_index = BlockIndex::zero();
    // Grid contents:
    // -----
    // lprompt1%cmd1 rprompt
    // output1
    // output2
    // -----

    let mut prompt_and_command_grid = mock_blockgrid("lprompt%cmd1");
    prompt_and_command_grid.finish();
    let mut rprompt_grid = mock_blockgrid("rprompt");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("output1\r\noutput2\r\n");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    block.set_raw_prompt_end_point(Some(PromptEndPoint::PromptEnd {
        point: Point::new(0, 7),
        has_extra_trailing_newline: false,
    }));

    // Test cross-over across all sections (lprompt to output).
    let all_grids = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(2, 0)),
    );
    assert_eq!(all_grids, "lprompt%cmd1\nrprompt\noutput1\noutput2");
}

/// Tests the single line lprompt, with trailing newline, and command case for text selection across grids.
#[test]
fn test_selection_bounds_all_grids_single_line_lprompt_trailing_newline_command() {
    let block_index = BlockIndex::zero();
    // Grid contents (lprompt has trailing newline):
    // -----
    // lprompt1%
    // cmd1 rprompt
    // output1
    // output2
    // -----

    let mut prompt_and_command_grid = mock_blockgrid("lprompt%\r\ncmd1");
    prompt_and_command_grid.finish();
    let mut rprompt_grid = mock_blockgrid("rprompt");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("output1\r\noutput2\r\n");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid.clone(),
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    // We are including the \n in the lprompt, not the command!
    block.set_raw_prompt_end_point(Some(PromptEndPoint::PromptEnd {
        point: Point::new(0, 7),
        has_extra_trailing_newline: true,
    }));

    // Test cross-over across all sections (lprompt to output).
    let all_grids = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(2, 0)),
    );
    assert_eq!(all_grids, "lprompt%\ncmd1\nrprompt\noutput1\noutput2");
}

// Tests the single line lprompt and command, with starting newline, case for text selection across grids.
#[test]
fn test_selection_bounds_all_grids_single_line_lprompt_command_starting_newline() {
    let block_index = BlockIndex::zero();
    // Grid contents (command has starting newline):
    // -----
    // lprompt1% rprompt
    // cmd1
    // output1
    // output2
    // -----

    let mut prompt_and_command_grid = mock_blockgrid("lprompt%\r\ncmd1");
    prompt_and_command_grid.finish();
    let mut rprompt_grid = mock_blockgrid("rprompt");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("output1\r\noutput2\r\n");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid.clone(),
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    // We are including the \n in the command, not the lprompt!
    block.set_raw_prompt_end_point(Some(PromptEndPoint::PromptEnd {
        point: Point::new(0, 7),
        has_extra_trailing_newline: false,
    }));

    // Test cross-over across all sections (lprompt to output).
    let all_grids = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(2, 0)),
    );
    assert_eq!(all_grids, "lprompt%\nrprompt\ncmd1\noutput1\noutput2");
}

/// Tests the multiline lprompt case, with no command content (where prompt end point is NOT defined).
#[test]
fn test_selection_bounds_all_grids_multiline_lprompt_no_command() {
    let block_index = BlockIndex::zero();
    // Grid contents:
    // -----
    // lprompt1
    // lprompt2% rprompt
    // output1
    // output2
    // -----
    // Note: practically, this case is generally not possible, since a command needs to be run for output, but this is used for testing purposes and it
    // could happen in a case similar to background output block (though, practically, those blocks don't have prompts).

    let mut prompt_and_command_grid = mock_blockgrid("lprompt1\r\nlprompt2%");
    prompt_and_command_grid.finish();
    let mut rprompt_grid = mock_blockgrid("rprompt");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("output1\r\noutput2\r\n");
    output_grid.finish();

    let block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );

    // Test cross-over across all sections (lprompt to output).

    let all_grids = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(2, 0)),
    );
    assert_eq!(all_grids, "lprompt1\nlprompt2%\nrprompt\noutput1\noutput2");

    // Test individual sections.

    let lprompt_only = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 2)),
        BlockGridPoint::PromptAndCommand(Point::new(1, 3)),
    );
    assert_eq!(lprompt_only, "rompt1\nlpro");

    let rprompt_only = block.bounds_to_string(
        BlockGridPoint::Rprompt(Point::new(0, 1)),
        BlockGridPoint::Rprompt(Point::new(0, 4)),
    );
    assert_eq!(rprompt_only, "prom");

    let output_only = block.bounds_to_string(
        BlockGridPoint::Output(Point::new(0, 3)),
        BlockGridPoint::Output(Point::new(0, 6)),
    );
    assert_eq!(output_only, "put1");

    // Test cross-overs across sections.

    let lprompt_to_rprompt = block.bounds_to_string(
        BlockGridPoint::PromptAndCommand(Point::new(0, 2)),
        BlockGridPoint::PromptAndCommand(Point::new(1, 2)),
    );
    assert_eq!(lprompt_to_rprompt, "rompt1\nlpr");

    let rprompt_to_output = block.bounds_to_string(
        BlockGridPoint::Rprompt(Point::new(0, 0)),
        BlockGridPoint::Output(Point::new(0, 3)),
    );
    assert_eq!(rprompt_to_output, "rprompt\noutp");

    // Test a few "impossible cases" and ensure they return an empty string.
    let impossible_output_to_rprompt = block.bounds_to_string(
        BlockGridPoint::Output(Point::new(1, 0)),
        BlockGridPoint::Rprompt(Point::new(1, 1)),
    );
    assert_eq!(impossible_output_to_rprompt, "");

    let impossible_output_to_lprompt = block.bounds_to_string(
        BlockGridPoint::Output(Point::new(1, 0)),
        BlockGridPoint::PromptAndCommand(Point::new(0, 1)),
    );
    assert_eq!(impossible_output_to_lprompt, "");
}

/// Tests cloning "typeahead" content from background output grid into the combined grid.
/// Specifically, this tests the user behavior of typeahead when pressing ENTER after typing a command.
#[test]
fn test_clone_command_from_blockgrid() {
    let block_index = BlockIndex::zero();
    // Grid contents (copying INTO this Grid):
    // -----
    // lprompt1
    // lprompt2%
    // -----

    // Grid to copy contents FROM:
    // -----
    // clone1
    // clone2
    // clone3
    // -----

    // Final expected grid (internals):
    // -----
    // lprompt1
    // lprompt2% (soft-wrap)
    // clone1    (hard-wrap)
    // clone2    (hard-wrap)
    // clone3    (hard-wrap)
    // -----

    let prompt_and_command_grid = mock_blockgrid("lprompt1\r\nlprompt2%");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );

    let mut grid_to_clone = mock_blockgrid("clone1\r\nclone2\r\nclone3");
    grid_to_clone.finish();

    block
        .header_grid
        .prompt_and_command_grid_mut()
        .set_mode(ansi::Mode::LineFeedNewLine);

    block.copy_command_grid(&grid_to_clone);

    let new_content = block.prompt_and_command_grid().to_string(
        false,
        None,
        RespectObfuscatedSecrets::Yes,
        false,
        RespectDisplayedOutput::Yes,
    );
    assert_eq!(new_content, "lprompt1\nlprompt2%clone1\nclone2\nclone3");
}

#[test]
fn test_clone_command_from_blockgrid_long() {
    let block_index = BlockIndex::zero();
    // Grid contents (copying INTO this Grid):
    // -----
    // lprompt1
    // lprompt2%
    // -----

    // Grid to copy contents FROM:
    // -----
    // clone1_long
    // clone2_long
    // clone3_long
    // -----

    // Final expected grid (internals):
    // -----
    // lprompt1
    // lprompt2%    (soft-wrap)
    // clone1_lo    (soft-wrap)
    // ng           (hard-wrap)
    // clone2_lo    (soft-wrap)
    // ng           (hard-wrap)
    // clone3_lo    (soft-wrap)
    // ng
    // -----

    let prompt_and_command_grid = mock_blockgrid("lprompt1\r\nlprompt2%");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );

    let mut grid_to_clone = mock_blockgrid("clone1_long\r\nclone2_long\r\nclone3_long");
    grid_to_clone.finish();

    block
        .header_grid
        .prompt_and_command_grid_mut()
        .set_mode(ansi::Mode::LineFeedNewLine);

    block.copy_command_grid(&grid_to_clone);

    let new_content = block.prompt_and_command_grid().to_string(
        false,
        None,
        RespectObfuscatedSecrets::Yes,
        false,
        RespectDisplayedOutput::Yes,
    );
    assert_eq!(
        new_content,
        "lprompt1\nlprompt2%clone1_long\nclone2_long\nclone3_long"
    );
}

#[test]
fn test_clone_command_from_blockgrid_from_empty() {
    let block_index = BlockIndex::zero();
    // Grid contents (copying INTO this Grid):
    // -----
    // lprompt1
    // lprompt2%
    // -----

    // Grid to copy contents FROM:
    // -----
    // -----

    // Final expected grid (internals):
    // -----
    // lprompt1   (hard-wrap)
    // lprompt2%
    // -----

    let prompt_and_command_grid = mock_blockgrid("lprompt1\r\nlprompt2%");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );

    let mut grid_to_clone = mock_blockgrid("");
    grid_to_clone.finish();

    block
        .header_grid
        .prompt_and_command_grid_mut()
        .set_mode(ansi::Mode::LineFeedNewLine);

    block.copy_command_grid(&grid_to_clone);

    let new_content = block.prompt_and_command_grid().to_string(
        false,
        None,
        RespectObfuscatedSecrets::Yes,
        false,
        RespectDisplayedOutput::Yes,
    );
    assert_eq!(new_content, "lprompt1\nlprompt2%");
}

#[test]
fn test_clone_command_from_blockgrid_to_empty() {
    let block_index = BlockIndex::zero();
    // Grid contents (copying INTO this Grid):
    // -----
    // -----

    // Grid to copy contents FROM:
    // -----
    // clone1
    // clone2
    // clone3
    // -----

    // Final expected grid (internals):
    // -----
    // clone1 (hard-wrap)
    // clone2 (hard-wrap)
    // clone3
    // -----

    let prompt_and_command_grid = mock_blockgrid("");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );

    let mut grid_to_clone = mock_blockgrid("clone1\r\nclone2\r\nclone3");
    grid_to_clone.finish();

    block
        .header_grid
        .prompt_and_command_grid_mut()
        .set_mode(ansi::Mode::LineFeedNewLine);

    block.copy_command_grid(&grid_to_clone);

    let new_content = block.prompt_and_command_grid().to_string(
        false,
        None,
        RespectObfuscatedSecrets::Yes,
        false,
        RespectDisplayedOutput::Yes,
    );
    assert_eq!(new_content, "clone1\nclone2\nclone3");
}

/// Tests is_command_empty for an empty command with the combined grid.
#[test]
fn test_command_is_empty_combined_grid() {
    let block_index = BlockIndex::zero();
    // Grid contents:
    // -----
    // -----

    let prompt_and_command_grid = mock_blockgrid("");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    block.start();
    block.finish_command_grid();

    assert!(
        block.is_command_empty(),
        "Command should be considered empty!"
    );
}

/// Tests is_command_empty for an non-empty command with the combined grid.
#[test]
fn test_command_is_not_empty_combined_grid() {
    let block_index = BlockIndex::zero();
    // Grid contents:
    // -----
    // command1 (linefeed at the end from finishing the block)
    // -----

    let prompt_and_command_grid = mock_blockgrid("command1");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    block.start();
    block.finish(0);

    assert!(
        !block.is_command_empty(),
        "Command should not be considered empty!"
    );
}

/// Regression test (CORE-1947): checks Warp prompt case for is_command_empty. Specifically,
/// even if the combined grid's content _exactly_ matches the prompt grid's content (which is used
/// for PS1 preview), we should NOT consider the command to be empty. The underlying cursor check should
/// be against (0, 0) in the combined grid, for the Warp prompt case, rather than checking against the
/// prompt grid (which we do in the PS1 active case).
#[test]
fn test_command_is_empty_warp_prompt() {
    let block_index = BlockIndex::zero();
    // Combined grid contents:
    // -----
    // warp_prompt
    // abcde
    // -----

    // Prompt grid contents:
    // -----
    // abcde
    // -----

    let prompt_and_command_grid = mock_blockgrid("abcde");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();
    // Note that the prompt_grid's finished cursor will be at (0, 4) with the content below.
    // This will be the exact same as the combined grid's cursor at (0, 4), which will
    // check the regression case (we should NOT consider the command to be empty!).
    let mut prompt_grid = mock_blockgrid("abcde");
    prompt_grid.finish();

    // Note that we are indicating Warp prompt, not PS1 here!
    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        false, /* honor_ps1 */
    );
    block.set_prompt_grid(prompt_grid);
    block.start();
    block.finish_command_grid();

    assert!(
        !block.is_command_empty(),
        "Command should not be considered empty!"
    );
}

/// Tests is_command_empty behavior with PS1 prompt _and_ command with combined grid.
#[test]
fn test_command_is_empty_combined_grid_prompt_and_command() {
    let block_index = BlockIndex::zero();
    // Combined grid contents:
    // -----
    // prompt1command1
    // -----

    // Prompt grid contents:
    // -----
    // prompt1
    // -----

    let prompt_and_command_grid = mock_blockgrid("prompt1command1");
    let mut rprompt_grid = mock_blockgrid("");
    rprompt_grid.finish();
    let mut output_grid = mock_blockgrid("");
    output_grid.finish();
    let mut prompt_grid = mock_blockgrid("prompt1");
    prompt_grid.finish();

    let mut block = create_test_block_with_grids(
        block_index,
        prompt_and_command_grid,
        rprompt_grid,
        output_grid,
        true, /* honor_ps1 */
    );
    block.set_prompt_grid(prompt_grid);
    block.start();
    block.finish(0);

    assert!(
        !block.is_command_empty(),
        "Command should not be considered empty!"
    );
}

#[test]
fn test_top_level_command() {
    let mut sessions = Sessions::new_for_test();
    sessions.register_session_for_test(SessionInfo::new_for_test().with_id(0));

    let mut block = TestBlockBuilder::new()
        .with_size_info(SizeInfo::new_without_font_metrics(1, 20))
        .build();
    block.start();
    block.precmd(PrecmdValue {
        session_id: Some(0),
        ..Default::default()
    });
    for c in "git status".chars() {
        block.input(c);
    }

    assert_eq!(block.top_level_command(&sessions), Some("git".to_string()));

    let mut block = TestBlockBuilder::new()
        .with_size_info(SizeInfo::new_without_font_metrics(1, 20))
        .build();
    block.start();
    block.precmd(PrecmdValue {
        session_id: Some(0),
        ..Default::default()
    });
    for c in "PAGER=0 git status".chars() {
        block.input(c);
    }

    assert_eq!(block.top_level_command(&sessions), Some("git".to_string()));
}

#[test]
fn test_top_level_command_with_aliases() {
    let mut sessions = Sessions::new_for_test();
    sessions.register_session_for_test(SessionInfo::new_for_test().with_id(0).with_aliases(
        HashMap::from_iter([
            ("git".into(), String::from("git2")),
            ("gs".into(), String::from("git status")),
            ("gl".into(), String::from("PAGER=0 git log")),
        ]),
    ));

    let mut block = TestBlockBuilder::new()
        .with_size_info(SizeInfo::new_without_font_metrics(1, 20))
        .build();
    block.start();
    block.precmd(PrecmdValue {
        session_id: Some(0),
        ..Default::default()
    });
    for c in "git status".chars() {
        block.input(c);
    }
    assert_eq!(block.top_level_command(&sessions), Some("git2".to_string()));

    let mut block = TestBlockBuilder::new()
        .with_size_info(SizeInfo::new_without_font_metrics(1, 20))
        .build();
    block.start();
    block.precmd(PrecmdValue {
        session_id: Some(0),
        ..Default::default()
    });
    for c in "gs --ignored".chars() {
        block.input(c);
    }
    assert_eq!(block.top_level_command(&sessions), Some("git".to_string()));

    let mut block = TestBlockBuilder::new()
        .with_size_info(SizeInfo::new_without_font_metrics(1, 20))
        .build();
    block.start();
    block.precmd(PrecmdValue {
        session_id: Some(0),
        ..Default::default()
    });
    for c in "PAGER= gl --flag".chars() {
        block.input(c);
    }
    assert_eq!(block.top_level_command(&sessions), Some("git".to_string()));
}

#[test]
fn test_mark_end_of_prompt_with_some_rows_in_flat_storage() {
    use warp_terminal::model::grid::Dimensions as _;

    let mut block = TestBlockBuilder::new()
        // Set the number of visible rows to 1, so that the first row of the
        // prompt ends up in flat storage.
        .with_size_info(SizeInfo::new_without_font_metrics(1, 40))
        .with_honor_ps1(true)
        .build();

    block.precmd(PrecmdValue {
        ps1: Some(hex::encode("prompt1\r\n")),
        honor_ps1: Some(true),
        ..Default::default()
    });

    block.start();
    block.finish(0);

    // Resize the grid so that everything ends up in grid storage.
    let rows = block
        .header_grid
        .prompt_and_command_grid()
        .grid_handler()
        .total_rows();
    block.resize(SizeInfo::new_without_font_metrics(rows, 40));

    // Assert that the end-of-prompt marker is set where we expect it to be.
    // This ensures that we properly set it within flat storage when initially
    // setting the end-of-prompt marker during precmd().
    let prompt_end_point = block.header_grid.prompt_end_point();
    let Some(PromptEndPoint::PromptEnd { point, .. }) = prompt_end_point else {
        panic!("Expected to have a well-defined prompt end point; got: {prompt_end_point:?}");
    };
    assert_eq!(point, Point::new(0, 39));
    assert!(block.header_grid.prompt_and_command_grid().grid_storage()[point].is_end_of_prompt());
}

#[test]
fn test_restored_block_was_local() {
    // Test default value is None
    let mut block = TestBlockBuilder::new()
        .with_bootstrap_stage(BootstrapStage::RestoreBlocks)
        .build();
    assert_eq!(block.restored_block_was_local(), None);

    // Test setting to true
    block.set_restored_block_was_local(true);
    assert_eq!(block.restored_block_was_local(), Some(true));

    // Test changing value
    block.set_restored_block_was_local(false);
    assert_eq!(block.restored_block_was_local(), Some(false));

    // Test with another new block (default is still None)
    let block = TestBlockBuilder::new()
        .with_bootstrap_stage(BootstrapStage::RestoreBlocks)
        .build();
    assert_eq!(block.restored_block_was_local(), None);
}

#[test]
fn test_deserialize_legacy_agent_view_visibility_agent_variant() {
    let origin_conversation_id = AIConversationId::new();
    let json = format!("{{\"Agent\":{{\"conversation_id\":\"{origin_conversation_id}\"}}}}");

    let visibility: SerializedAgentViewVisibility = serde_json::from_str(&json).unwrap();
    match visibility {
        SerializedAgentViewVisibility::Agent {
            origin_conversation_id: parsed_origin_conversation_id,
            pending_other_conversation_ids,
            other_conversation_ids,
        } => {
            assert_eq!(parsed_origin_conversation_id, origin_conversation_id);
            assert!(pending_other_conversation_ids.is_empty());
            assert!(other_conversation_ids.is_empty());
        }
        _ => panic!("Expected agent visibility"),
    }
}

#[test]
fn test_calculate_optimal_row_counts_wide_terminal() {
    // Terminal width >= 150 should return default values
    let (top, bottom) = calculate_optimal_row_counts(150, 100, 200);
    assert_eq!(top, 100);
    assert_eq!(bottom, 200);

    let (top, bottom) = calculate_optimal_row_counts(200, 50, 100);
    assert_eq!(top, 50);
    assert_eq!(bottom, 100);
}

#[test]
fn test_calculate_optimal_row_counts_narrow_terminal() {
    // Terminal width < 150 should optimize based on target total cells
    let (top, bottom) = calculate_optimal_row_counts(75, 100, 200);

    // Target total cells = (100 + 200) * 150 = 45,000
    // Total rows for width 75 = 45,000 / 75 = 600
    // Top rows = (600 * 100) / 300 = 200
    // Bottom rows = 600 - 200 = 400
    assert_eq!(top, 200);
    assert_eq!(bottom, 400);
}

#[test]
fn test_calculate_optimal_row_counts_very_narrow_terminal() {
    // Test edge case with very narrow terminal
    let (top, bottom) = calculate_optimal_row_counts(30, 100, 200);

    // Target total cells = (100 + 200) * 150 = 45,000
    // Total rows for width 30 = 45,000 / 30 = 1,500
    // Top rows = (1,500 * 100) / 300 = 500
    // Bottom rows = 1,500 - 500 = 1,000
    assert_eq!(top, 500);
    assert_eq!(bottom, 1000);
}
