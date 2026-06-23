use warpui_core::elements::tui::{TuiBuffer, TuiBufferExt, TuiElement, TuiRect, TuiText};

use super::BottomAnchoredColumn;

fn text_children(lines: &[&str]) -> Vec<Box<dyn TuiElement>> {
    lines
        .iter()
        .map(|line| Box::new(TuiText::new(*line)) as Box<dyn TuiElement>)
        .collect()
}

fn render_to_lines(element: &dyn TuiElement, width: u16, height: u16) -> Vec<String> {
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, width, height));
    element.render(TuiRect::new(0, 0, width, height), &mut buffer);
    buffer.to_lines()
}

#[test]
fn bottom_aligns_when_content_is_shorter_than_the_area() {
    let column = BottomAnchoredColumn::new(text_children(&["A", "B"]));
    // Two rows of content in a four-row area sit flush against the bottom.
    assert_eq!(render_to_lines(&column, 1, 4), vec![" ", " ", "A", "B"],);
}

#[test]
fn clips_the_top_when_content_overflows() {
    let column = BottomAnchoredColumn::new(text_children(&["A", "B", "C", "D"]));
    // Only the newest (bottom-most) two rows are visible; the top is clipped.
    assert_eq!(render_to_lines(&column, 1, 2), vec!["C", "D"]);
}

#[test]
fn preserves_entry_order() {
    let column = BottomAnchoredColumn::new(text_children(&["first", "second"]));
    assert_eq!(render_to_lines(&column, 6, 2), vec!["first ", "second"],);
}
