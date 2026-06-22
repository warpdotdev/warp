use crossterm::style::Color;

use super::TuiText;
use crate::elements::tui::{TuiBuffer, TuiConstraint, TuiElement, TuiRect, TuiSize, TuiStyle};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::new(size);
    element.render(TuiRect::from_size(size), &mut buffer);
    buffer.to_lines()
}

#[test]
fn renders_a_single_short_line() {
    let text = TuiText::new("hello");
    assert_eq!(
        render_to_lines(&text, TuiSize::new(10, 1)),
        vec!["hello     "],
    );
}

#[test]
fn layout_reports_content_width_and_row_count() {
    let mut text = TuiText::new("hello world foo");
    let size = text.layout(TuiConstraint::loose(TuiSize::new(11, 10)));
    // "hello world" packs onto row 1 (11 cols), "foo" wraps to row 2.
    assert_eq!(size, TuiSize::new(11, 2));
    assert_eq!(text.desired_height(11), 2);
}

#[test]
fn word_wraps_at_the_width_boundary() {
    let text = TuiText::new("hello world foo");
    assert_eq!(
        render_to_lines(&text, TuiSize::new(11, 2)),
        vec!["hello world", "foo        "],
    );
}

#[test]
fn hard_breaks_a_token_wider_than_the_row() {
    let text = TuiText::new("abcdefgh");
    assert_eq!(text.desired_height(3), 3);
    assert_eq!(
        render_to_lines(&text, TuiSize::new(3, 3)),
        vec!["abc", "def", "gh "],
    );
}

#[test]
fn wide_glyphs_occupy_two_columns_and_are_never_split() {
    // A wide glyph painted with one trailing column to spare drops whole: only
    // the leading "日" lands, proving it claimed two columns.
    let truncated = TuiText::new("日本").truncate();
    assert_eq!(render_to_lines(&truncated, TuiSize::new(3, 1)), vec!["日 "],);

    // Given exactly four columns both wide glyphs fit.
    assert_eq!(
        render_to_lines(&TuiText::new("日本"), TuiSize::new(4, 1)),
        vec!["日本"],
    );

    // Wrapping a wide pair into a three-column row splits between glyphs.
    let wrapped = TuiText::new("日本");
    assert_eq!(wrapped.desired_height(3), 2);
    assert_eq!(
        render_to_lines(&wrapped, TuiSize::new(3, 2)),
        vec!["日 ", "本 "],
    );
}

#[test]
fn applies_its_style_to_every_painted_cell() {
    let style = TuiStyle::default()
        .with_foreground(Color::Red)
        .with_bold(true);
    let text = TuiText::new("a").with_style(style);

    let mut buffer = TuiBuffer::new(TuiSize::new(1, 1));
    text.render(TuiRect::new(0, 0, 1, 1), &mut buffer);

    let cell = buffer.get(0, 0).expect("cell in bounds");
    assert_eq!(cell.symbol(), "a");
    assert_eq!(cell.style(), style);
}

#[test]
fn truncation_keeps_one_row_per_hard_line() {
    let text = TuiText::new("a\nb\nc").truncate();
    assert_eq!(text.desired_height(10), 3);
    assert_eq!(
        render_to_lines(&text, TuiSize::new(3, 3)),
        vec!["a  ", "b  ", "c  "],
    );
}
