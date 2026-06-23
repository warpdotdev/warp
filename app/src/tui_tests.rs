//! Headless integration tests for [`RootTuiView`]: they stand up the composed
//! transcript + input views in a test [`App`], drive editing through the same
//! typed actions the input's key handlers dispatch, and assert the frame the
//! [`TuiPresenter`] paints (buffer contents + cursor) reacts end-to-end —
//! submission routing from the input through the root into the transcript, the
//! empty-state placeholder, focus, and the transcript's bottom-anchored
//! stacking with top-clipping.

use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::platform::WindowStyle;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{AddWindowOptions, App, EntityId, WindowId};

use super::input_view::InputAction;
use super::{RootTuiView, INPUT_ROWS};

fn window_options() -> AddWindowOptions {
    AddWindowOptions {
        window_style: WindowStyle::NotStealFocus,
        ..Default::default()
    }
}

/// Types `text` into the input one scalar at a time, exactly as the input's
/// key-down fallback does for printable input.
fn type_text(app: &App, window_id: WindowId, input_id: EntityId, text: &str) {
    for ch in text.chars() {
        app.dispatch_typed_action(window_id, &[input_id], &InputAction::Insert(ch.to_string()));
    }
}

/// Presses Enter on the input, as its `enter` key binding does.
fn submit(app: &App, window_id: WindowId, input_id: EntityId) {
    app.dispatch_typed_action(window_id, &[input_id], &InputAction::Submit);
}

/// The first row index whose painted line contains `needle`.
fn row_with(lines: &[String], needle: &str) -> Option<usize> {
    lines.iter().position(|line| line.contains(needle))
}

#[test]
fn submitting_moves_text_into_the_transcript_and_clears_the_focused_input() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), RootTuiView::new));
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));

        // The input is focused at construction, so the cursor is owned by it.
        assert_eq!(app.focused_view_id(window_id), Some(input_id));

        let mut presenter = TuiPresenter::new();
        // 8 rows tall: the input frame is the bottom `INPUT_ROWS` (3) rows, so
        // the inner text row sits at `height - 2` and the transcript fills the
        // rows above it.
        let width = 40;
        let height = 8;
        let area = TuiRect::new(0, 0, width, height);
        let input_text_row = height - 2;
        let first_input_row = height - INPUT_ROWS;

        type_text(&app, window_id, input_id, "hello world");

        // Before submission the draft renders inside the input frame and the
        // cursor trails the typed text (one cell past it, offset by the border).
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        assert!(
            lines[input_text_row as usize].contains("hello world"),
            "the draft should render in the input frame's text row:\n{}",
            lines.join("\n")
        );
        // 11 chars typed, +1 for the left border cell.
        assert_eq!(frame.cursor, Some((12, input_text_row)));

        submit(&app, window_id, input_id);

        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();

        // The submitted text moved into the transcript, which sits above the
        // input frame.
        let transcript_row =
            row_with(&lines, "hello world").expect("the transcript should show the submitted text");
        assert!(
            transcript_row < first_input_row as usize,
            "the submitted text should render above the input frame (row {transcript_row}):\n{}",
            lines.join("\n")
        );

        // The input is cleared back to its placeholder empty-state, and the
        // submitted text is no longer in the input frame.
        let input_region = lines[first_input_row as usize..].join("\n");
        assert!(
            input_region.contains("Warp anything"),
            "the cleared input should show its placeholder:\n{input_region}"
        );
        assert!(
            !input_region.contains("hello world"),
            "the submitted text should no longer be in the input frame:\n{input_region}"
        );

        // Focus stays on the input and the cursor resets to the frame's start.
        assert_eq!(app.focused_view_id(window_id), Some(input_id));
        assert_eq!(frame.cursor, Some((1, input_text_row)));
    });
}

#[test]
fn transcript_anchors_newest_entry_to_the_bottom_and_clips_the_top() {
    App::test((), |mut app| async move {
        let (window_id, root) =
            app.update(|ctx| ctx.add_tui_window(window_options(), RootTuiView::new));
        let input_id = app.read(|ctx| root.read(ctx, |view, _| view.input.id()));

        for entry in ["one", "two", "three", "four", "five"] {
            type_text(&app, window_id, input_id, entry);
            submit(&app, window_id, input_id);
        }

        // 7 rows tall leaves only 4 transcript rows above the 3-row input
        // frame, too few to show all five entries (each entry takes a text row
        // plus a spacer), so the oldest are clipped off the top.
        let mut presenter = TuiPresenter::new();
        let area = TuiRect::new(0, 0, 40, 7);
        let frame = app.update(|ctx| presenter.present(ctx, &root, area));
        let lines = frame.buffer.to_lines();
        let rendered = lines.join("\n");

        // The two newest entries are visible, oldest-above-newest (the newest
        // sits closest to the input).
        let four_row = row_with(&lines, "four").expect("the second-newest entry should be visible");
        let five_row = row_with(&lines, "five").expect("the newest entry should be visible");
        assert!(
            four_row < five_row,
            "newer entries should sit below older ones:\n{rendered}"
        );

        // The entries that overflow the top are clipped.
        for clipped in ["one", "two", "three"] {
            assert!(
                row_with(&lines, clipped).is_none(),
                "{clipped:?} should be clipped off the top:\n{rendered}"
            );
        }
    });
}
