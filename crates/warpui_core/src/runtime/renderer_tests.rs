use ratatui::buffer::CellWidth;
use ratatui::style::{Color, Modifier};

use super::TuiFrameRenderer;
use crate::elements::tui::{TuiBuffer, TuiBufferExt, TuiRect, TuiStyle};

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

/// Clears a wide grapheme when a write starts in one of its occupied columns.
fn clear_wide_grapheme_covering(screen: &mut TuiBuffer, x: u16, y: u16) {
    let area = screen.area;
    for column in (area.x..x).rev() {
        let cell = &screen[(column, y)];
        if column.saturating_add(cell.cell_width()) > x {
            screen[(column, y)].reset();
            break;
        }
    }
}

/// Replays the glyph-placement CSI emitted by `TuiFrameRenderer`.
///
/// Writing into a wide grapheme's continuation column clears the grapheme,
/// matching terminals that expose the continuation-cell ordering bug.
fn render_to_screen(output: &str, width: u16) -> TuiBuffer {
    let mut screen = TuiBuffer::empty(TuiRect::new(0, 0, width, 1));
    let (mut cx, mut cy) = (0u16, 0u16);
    let chars: Vec<char> = output.chars().collect();
    let mut run = String::new();
    let mut i = 0;
    let flush = |screen: &mut TuiBuffer, run: &mut String, cx: u16, cy: u16| {
        if !run.is_empty() {
            clear_wide_grapheme_covering(screen, cx, cy);
            let max_width = usize::from(width.saturating_sub(cx));
            screen.set_stringn(cx, cy, run.as_str(), max_width, TuiStyle::default());
            run.clear();
        }
    };
    while i < chars.len() {
        let ch = chars[i];
        if ch != '\u{1b}' {
            run.push(ch);
            i += 1;
            continue;
        }
        flush(&mut screen, &mut run, cx, cy);
        // Parse a CSI sequence: ESC '[' params final-byte.
        i += 1;
        if i >= chars.len() || chars[i] != '[' {
            continue;
        }
        i += 1;
        let mut params = String::new();
        while i < chars.len() && !('@'..='~').contains(&chars[i]) {
            params.push(chars[i]);
            i += 1;
        }
        let final_byte = chars.get(i).copied().unwrap_or('\0');
        i += 1;
        match final_byte {
            'H' => {
                let mut parts = params.split(';');
                let y = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1u16);
                let x = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1u16);
                cy = y.saturating_sub(1);
                cx = x.saturating_sub(1);
            }
            'J' if params.contains('2') => {
                screen = TuiBuffer::empty(TuiRect::new(0, 0, width, 1));
                cx = 0;
                cy = 0;
            }
            _ => {}
        }
    }
    flush(&mut screen, &mut run, cx, cy);
    screen
}

/// The column of the first cell in the single-row `screen` whose symbol equals
/// `glyph`, if any.
fn column_of(screen: &TuiBuffer, glyph: &str) -> Option<u16> {
    let area = screen.area;
    (0..area.width).find(|&col| screen[(area.x + col, area.y)].symbol() == glyph)
}

#[test]
fn wide_grapheme_does_not_shift_following_cells() {
    // Regression test for stray/stale characters in a scrolled TUI transcript.
    //
    // A VS16 emoji (⌨️ = U+2328 U+FE0F, display width 2) makes the buffer diff
    // emit an explicit trailing-clear cell at `wide_x + 1`. The crossterm
    // backend's "the cursor already advanced by one column" optimization would
    // then suppress the needed cursor move and print that cell — and everything
    // after it on the line — one column too far right, so the `g` following the
    // emoji lands on column 3 instead of 2. Because scrolling only redraws the
    // per-frame diff (no full repaint), the misplaced glyph is never repaired
    // and sticks around. The renderer must keep the following cells on their
    // intended columns.
    let mut renderer = TuiFrameRenderer::new();
    let _ = draw_to_string(&mut renderer, &line_buffer("ABCDE"));

    // Same size as the previous frame, so this is an incremental diff, not a
    // full repaint: emoji at column 0 (spans 0..=1), then `g`, `Y`, `Z`.
    let mut next = TuiBuffer::empty(TuiRect::new(0, 0, 5, 1));
    next.set_stringn(0, 0, "\u{2328}\u{fe0f}gYZ", 5, TuiStyle::default());
    let output = draw_to_string(&mut renderer, &next);

    let screen = render_to_screen(&output, 5);
    assert_eq!(
        column_of(&screen, "\u{2328}\u{fe0f}"),
        Some(0),
        "the trailing clear must not erase the wide grapheme; rendered screen: {:?}",
        screen.to_lines(),
    );
    assert_eq!(
        column_of(&screen, "g"),
        Some(2),
        "the glyph after a wide VS16 emoji must stay on column 2, not shift right; rendered screen: {:?}",
        screen.to_lines(),
    );
}
#[test]
fn wide_grapheme_styles_continuation_before_glyph() {
    let mut renderer = TuiFrameRenderer::new();
    let mut unselected = TuiBuffer::empty(TuiRect::new(0, 0, 5, 1));
    unselected.set_stringn(0, 0, "\u{2328}\u{fe0f}gYZ", 5, TuiStyle::default());
    let _ = draw_to_string(&mut renderer, &unselected);

    let mut selected = unselected.clone();
    selected.set_style(
        TuiRect::new(0, 0, 2, 1),
        TuiStyle::default().add_modifier(Modifier::REVERSED),
    );
    let output = draw_to_string(&mut renderer, &selected);

    let continuation_move = output
        .find(&move_to(1, 0))
        .expect("selection must style the wide grapheme's continuation cell");
    let grapheme = output
        .find("\u{2328}\u{fe0f}")
        .expect("selection must redraw the wide grapheme");
    assert!(
        continuation_move < grapheme,
        "the continuation style must be emitted before the wide grapheme"
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
