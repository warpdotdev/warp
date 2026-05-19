use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self, Handler};
use crate::terminal::model::blockgrid::{BlockGrid, CursorDisplayPoint};
use crate::terminal::model::grid::grid_handler::PerformResetGridChecks;
use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::index::{Point, VisibleRow};
use crate::terminal::model::secrets::ObfuscateSecrets;
use crate::terminal::SizeInfo;
use crate::test_util::mock_blockgrid;

#[test]
pub fn test_finish_truncates_grid_basic() {
    let size = SizeInfo::new_without_font_metrics(10, 7);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for c in "hello".chars() {
        block_grid.input(c);
    }
    block_grid.linefeed();
    block_grid.finish();

    assert_eq!(block_grid.len(), 2);
    assert_eq!(block_grid.grid_handler().total_rows(), 2);
    assert_eq!(block_grid.grid_handler().columns(), size.columns);
}

#[test]
pub fn test_finish_truncates_grid_cursor_at_bottom() {
    let size = SizeInfo::new_without_font_metrics(10, 7);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for _ in 0..300 {
        block_grid.input('a');
        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.finish();

    assert_eq!(block_grid.len(), 300);
    assert_eq!(block_grid.grid_handler().total_rows(), 301);
    assert_eq!(block_grid.grid_handler().columns(), size.columns);
}

#[test]
pub fn test_resize_finished_block() {
    let size = SizeInfo::new_without_font_metrics(10, 7);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for _ in 0..5 {
        for c in "hello".chars() {
            block_grid.input(c);
        }

        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.finish();

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);

    block_grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    // The len and total rows of the grid should be unchanged even after resize.
    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);

    // Resize so that the contents of can no longer fit on one line. The length of the grid
    // should increase because each row is now soft-wrapped.
    block_grid.resize(SizeInfo::new_without_font_metrics(10, 3));
    assert_eq!(block_grid.len(), 10);
    // Each of the 5 "hello" rows should be split into "hel" and "lo" (for a total
    // of 10 rows), plus one final empty row at the end.
    assert_eq!(block_grid.grid_handler().total_rows(), 11);

    // Resizing the height in either direction should have no effect on the total grid height.
    block_grid.resize(SizeInfo::new_without_font_metrics(3, 7));
    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);

    block_grid.resize(SizeInfo::new_without_font_metrics(300, 7));
    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);
}

#[test]
pub fn test_resize_finished_softwrapped_block() {
    let size = SizeInfo::new_without_font_metrics(10, 3);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for _ in 0..5 {
        for c in "hello".chars() {
            block_grid.input(c);
        }

        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.finish();

    assert_eq!(block_grid.len(), 10);
    assert_eq!(block_grid.grid_handler().total_rows(), 11);

    // Resize so each item can only fit on a given line.
    block_grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);
}

#[test]
pub fn test_content_summary() {
    let blockgrid = mock_blockgrid("1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9\r\n10\r\n");

    // Test a summary that omits the middle of the grid.
    let summary = blockgrid.content_summary(1, 2, false);
    assert_eq!(summary, "1\n...(truncated)...\n9\n10\n");

    // Test a summary that perfectly covers the entire grid.
    let summary = blockgrid.content_summary(7, 3, false);
    assert_eq!(summary, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");

    // Test a summary where the total range exceeds the grid size.
    let summary = blockgrid.content_summary(7, 6, false);
    assert_eq!(summary, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
}

#[test]
pub fn test_trim_trailing_blank_rows_uses_active_floor_for_blank_started_grid() {
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.goto(VisibleRow(4), 0);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    block_grid.set_trim_trailing_blank_rows(true);

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::HiddenCache(Point::new(0, 0)))
    );
}

#[test]
pub fn test_cursor_display_point_hidden_when_cursor_below_trimmed_content() {
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.input('a');
    block_grid.input('b');
    block_grid.input('c');
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    block_grid.set_trim_trailing_blank_rows(true);

    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::Visible(Point::new(0, 3)))
    );

    block_grid.goto(VisibleRow(4), 2);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::HiddenCache(Point::new(0, 2)))
    );
}

#[test]
pub fn test_cursor_display_point_not_clipped_when_trimming_disabled() {
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.input('a');
    block_grid.input('b');
    block_grid.input('c');
    block_grid
        .grid_handler_mut()
        .set_marked_text("12345678", &(0..0));

    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::Visible(Point::new(1, 1)))
    );

    block_grid.set_trim_trailing_blank_rows(true);

    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::HiddenCache(Point::new(0, 1)))
    );
}

/// Regression test for #10144: when an inline interactive command (e.g.
/// `aws configure`) writes a series of prompts and the inferior moves the
/// cursor back up — for example to redraw the current input line after a
/// terminal resize, or simply because the program walked its cursor down for
/// some intermediate operation — the block's `max_cursor_point` watermark
/// stays at the deepest row ever reached. Without trimming, `len_displayed`
/// returns that watermark and the block height balloons past its actual
/// content, leaving a large blank gap above the pinned-bottom input.
///
/// With trim enabled, `len_displayed` must track the last row with visible
/// content, which is what the user actually sees.
#[test]
pub fn test_trim_trailing_blank_rows_handles_inline_interactive_prompt_pattern() {
    let size = SizeInfo::new_without_font_metrics(40, 80);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.set_trim_trailing_blank_rows(true);

    // Simulate the inferior writing four `aws configure`-style prompts on
    // successive rows, advancing the cursor as it goes.
    for prompt in ["AWS Access Key ID [None]: ", "AWS Secret Access Key [None]: "] {
        for c in prompt.chars() {
            block_grid.input(c);
        }
        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // Sanity: two prompt rows of content, plus the (empty) cursor row.
    let height_after_prompts = block_grid.len_displayed();
    assert!(
        height_after_prompts <= 3,
        "expected displayed height to track content (<=3 rows), got {height_after_prompts}",
    );

    // Now reproduce the bug trigger: the inferior walks the cursor *down*
    // far past the last prompt (e.g. an inline redraw routine that probes
    // the terminal) and then comes back up to the current input row. This
    // is exactly what a misbehaving readline prompt can do, and what bumps
    // `max_cursor_point` past the visible content.
    block_grid.goto(VisibleRow(20), 0);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // `len()` reflects the high-water mark — that's the bug surface.
    assert_eq!(
        block_grid.len(),
        21,
        "len() should track max_cursor_point and reflect the inferior's deepest cursor row",
    );

    // Return the cursor to the actual prompt row. The bug surface persists:
    // `len()` is still anchored to the watermark.
    block_grid.goto(VisibleRow(1), 26);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(block_grid.len(), 21);

    // With `trim_trailing_blank_rows` enabled, `len_displayed` ignores the
    // ghost trailing rows and matches the real content height. This is the
    // height the blocklist viewport uses to position the active block, so
    // the visible cursor stays at the bottom of the pinned-bottom layout
    // instead of being pushed upward by ~18 ghost rows.
    let displayed = block_grid.len_displayed();
    assert!(
        displayed <= 3,
        "trim should cap displayed height at actual content (<=3 rows), got {displayed}",
    );
}

/// Companion to the regression test above: without trim, `len_displayed`
/// falls through to `len()` and the watermark bug is fully observable.
/// This guards against accidentally weakening the trim contract.
#[test]
pub fn test_without_trim_inline_interactive_prompt_pattern_balloons_height() {
    let size = SizeInfo::new_without_font_metrics(40, 80);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    // NB: trim intentionally *not* enabled — this asserts current behavior
    // without the fix from #10144.

    for c in "AWS Access Key ID [None]: ".chars() {
        block_grid.input(c);
    }
    block_grid.linefeed();
    block_grid.carriage_return();

    block_grid.goto(VisibleRow(20), 0);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // Walk cursor back up to the actual input row.
    block_grid.goto(VisibleRow(1), 26);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // Without trim, displayed height equals the high-water mark, even
    // though only one row of visible content exists.
    assert_eq!(block_grid.len(), 21);
    assert_eq!(block_grid.len_displayed(), 21);
}
