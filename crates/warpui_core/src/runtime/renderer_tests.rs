use ratatui::style::Color;

use super::TuiFrameRenderer;
use crate::elements::tui::{TuiBuffer, TuiRect, TuiStyle};

/// Builds a single-row buffer from `line`, sized to the line's column width.
fn line_buffer(line: &str) -> TuiBuffer {
    let width = u16::try_from(line.chars().count()).unwrap();
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, width, 1));
    buffer.set_stringn(0, 0, line, usize::from(width), TuiStyle::default());
    buffer
}

fn draw_to_string(renderer: &mut TuiFrameRenderer, buffer: &TuiBuffer) -> String {
    let mut output = Vec::new();
    renderer.draw(&mut output, buffer, None).unwrap();
    String::from_utf8(output).unwrap()
}

/// The CSI sequence crossterm emits to move the cursor to `(x, y)` (1-based).
fn move_to(x: u16, y: u16) -> String {
    format!("\u{1b}[{};{}H", y + 1, x + 1)
}

#[test]
fn first_paint_clears_and_writes_all_cells() {
    let mut renderer = TuiFrameRenderer::new();
    let output = draw_to_string(&mut renderer, &line_buffer("abc"));

    // Full repaint clears the screen and prints every non-blank cell.
    assert!(
        output.contains("\u{1b}[2J"),
        "first paint should clear screen"
    );
    assert!(output.contains("abc"), "first paint should write all cells");
}

#[test]
fn unchanged_frame_emits_no_text() {
    let mut renderer = TuiFrameRenderer::new();
    let buffer = line_buffer("abc");
    let _ = draw_to_string(&mut renderer, &buffer);

    let output = draw_to_string(&mut renderer, &buffer);
    assert!(
        !output.contains("abc"),
        "an unchanged frame should not re-emit any cell text"
    );
}

#[test]
fn diff_emits_only_changed_run() {
    let mut renderer = TuiFrameRenderer::new();
    let _ = draw_to_string(&mut renderer, &line_buffer("abcde"));

    let output = draw_to_string(&mut renderer, &line_buffer("abXYe"));

    assert!(output.contains("XY"), "diff should emit the changed run");
    assert!(
        output.contains(&move_to(2, 0)),
        "diff should move the cursor to the first changed column"
    );
    assert!(
        !output.contains("abcde") && !output.contains("abc"),
        "diff should not re-emit unchanged cells"
    );
}

#[test]
fn size_change_triggers_full_repaint() {
    let mut renderer = TuiFrameRenderer::new();
    let _ = draw_to_string(&mut renderer, &line_buffer("abc"));

    let output = draw_to_string(&mut renderer, &line_buffer("wxyz!"));
    // A resize repaints authoritatively (clear + redraw) so no stale content is
    // left from the previous, differently-wrapped frame. The clear is wrapped
    // in a synchronized update by `draw`, so it is applied atomically.
    assert!(
        output.contains("\u{1b}[2J"),
        "a size change should force a full repaint"
    );
    assert!(output.contains("wxyz!"));
}

#[test]
fn changed_wide_grapheme_is_emitted_whole() {
    let mut renderer = TuiFrameRenderer::new();
    let _ = draw_to_string(&mut renderer, &line_buffer("ab "));

    // Replace the two leading columns with a single wide (CJK) grapheme.
    let mut next = TuiBuffer::empty(TuiRect::new(0, 0, 3, 1));
    next.set_stringn(0, 0, "界 ", 3, TuiStyle::default());
    let output = draw_to_string(&mut renderer, &next);

    assert!(output.contains('界'), "the wide grapheme should be emitted");
    assert!(output.contains(&move_to(0, 0)));
}

#[test]
fn styled_run_changes_byte_stream() {
    // A styled cell must add an SGR color escape that the same text painted with
    // the default style does not, so the byte streams differ.
    let styled = {
        let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 3, 1));
        buffer.set_stringn(0, 0, "ab", 2, TuiStyle::default());
        buffer.set_stringn(2, 0, "C", 1, TuiStyle::default().fg(Color::Yellow));
        draw_to_string(&mut TuiFrameRenderer::new(), &buffer)
    };
    let plain = draw_to_string(&mut TuiFrameRenderer::new(), &line_buffer("abC"));

    assert!(
        styled.contains('C'),
        "styled run should still print its text"
    );
    assert_ne!(
        styled, plain,
        "a foreground color should change the byte stream"
    );
}

#[test]
fn cursor_is_shown_when_present_and_hidden_otherwise() {
    let mut renderer = TuiFrameRenderer::new();
    let buffer = line_buffer("abc");

    let mut shown = Vec::new();
    renderer.draw(&mut shown, &buffer, Some((1, 0))).unwrap();
    let shown = String::from_utf8(shown).unwrap();
    assert!(shown.contains("\u{1b}[?25h"), "cursor should be shown");
    assert!(shown.contains(&move_to(1, 0)));

    let mut hidden = Vec::new();
    renderer.draw(&mut hidden, &buffer, None).unwrap();
    let hidden = String::from_utf8(hidden).unwrap();
    assert!(hidden.contains("\u{1b}[?25l"), "cursor should be hidden");
}
