use super::TuiInputLine;
use crate::elements::tui::{TuiBuffer, TuiBufferExt, TuiElement, TuiRect, TuiSize};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, size.width, size.height));
    element.render(TuiRect::new(0, 0, size.width, size.height), &mut buffer);
    buffer.to_lines()
}

#[test]
fn is_always_a_single_row() {
    assert_eq!(TuiInputLine::new("hello", 0).desired_height(10), 1);
    assert_eq!(TuiInputLine::new("", 0).desired_height(10), 1);
}

#[test]
fn renders_text_and_reports_cursor_at_end() {
    let line = TuiInputLine::new("hello", 5);
    assert_eq!(
        render_to_lines(&line, TuiSize::new(10, 1)),
        vec!["hello     "]
    );
    assert_eq!(
        line.cursor_position(TuiRect::new(0, 0, 10, 1)),
        Some((5, 0))
    );
}

#[test]
fn reports_cursor_in_the_middle() {
    let line = TuiInputLine::new("hello", 2);
    assert_eq!(
        line.cursor_position(TuiRect::new(0, 0, 10, 1)),
        Some((2, 0))
    );
}

#[test]
fn wide_glyphs_advance_two_cells() {
    // Each CJK glyph occupies two columns, so a cursor after both sits at col 4.
    let line = TuiInputLine::new("世界", 2);
    assert_eq!(
        render_to_lines(&line, TuiSize::new(10, 1)),
        vec!["世界      "]
    );
    assert_eq!(
        line.cursor_position(TuiRect::new(0, 0, 10, 1)),
        Some((4, 0))
    );
}

#[test]
fn scrolls_horizontally_to_keep_the_cursor_visible() {
    // "abcdef" with the cursor at the end cannot fit in three cells, so the
    // view scrolls and the cursor anchors to the last column.
    let line = TuiInputLine::new("abcdef", 6);
    assert_eq!(render_to_lines(&line, TuiSize::new(3, 1)), vec!["ef "]);
    assert_eq!(line.cursor_position(TuiRect::new(0, 0, 3, 1)), Some((2, 0)));
}

#[test]
fn renders_placeholder_when_empty() {
    let line = TuiInputLine::new("", 0)
        .with_placeholder("hint", crate::elements::tui::TuiStyle::default());
    assert_eq!(render_to_lines(&line, TuiSize::new(8, 1)), vec!["hint    "]);
    // The cursor sits at the start even while the placeholder shows.
    assert_eq!(line.cursor_position(TuiRect::new(0, 0, 8, 1)), Some((0, 0)));
}
