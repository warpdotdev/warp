use ratatui::style::{Color, Modifier, Style};

use super::TuiText;
use crate::elements::tui::test_support::{render_to_frame, render_to_lines};
use crate::elements::tui::{TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiSize};
use crate::{App, EntityIdMap};

#[test]
fn renders_a_single_short_line() {
    let text = TuiText::new("hello");
    assert_eq!(
        render_to_lines(text, TuiSize::new(10, 1)),
        vec!["hello     "],
    );
}

#[test]
fn layout_reports_content_width_and_row_count() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let mut text = TuiText::new("hello world foo");
            let mut rendered_views = EntityIdMap::default();
            let mut ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let size = text.layout(
                TuiConstraint::loose(TuiSize::new(11, 10)),
                &mut ctx,
                app_ctx,
            );
            // "hello world" packs onto row 1 (11 cols), "foo" wraps to row 2.
            assert_eq!(size, TuiSize::new(11, 2));
            assert_eq!(text.desired_height(11), 2);
        });
    });
}

#[test]
fn word_wraps_at_the_width_boundary() {
    let text = TuiText::new("hello world foo");
    assert_eq!(
        render_to_lines(text, TuiSize::new(11, 2)),
        vec!["hello world", "foo        "],
    );
}

#[test]
fn hard_breaks_a_token_wider_than_the_row() {
    let text = TuiText::new("abcdefgh");
    assert_eq!(text.desired_height(3), 3);
    assert_eq!(
        render_to_lines(text, TuiSize::new(3, 3)),
        vec!["abc", "def", "gh "],
    );
}

#[test]
fn wide_glyphs_occupy_two_columns_and_are_never_split() {
    // A wide glyph painted with one trailing column to spare drops whole: only
    // the leading "日" lands, proving it claimed two columns.
    let truncated = TuiText::new("日本").truncate();
    assert_eq!(render_to_lines(truncated, TuiSize::new(3, 1)), vec!["日 "]);

    // Given exactly four columns both wide glyphs fit.
    assert_eq!(
        render_to_lines(TuiText::new("日本"), TuiSize::new(4, 1)),
        vec!["日本"],
    );

    // Wrapping a wide pair into a two-column row puts one glyph per row
    // (ratatui only breaks once the row's width is reached).
    let wrapped = TuiText::new("日本");
    assert_eq!(wrapped.desired_height(2), 2);
    assert_eq!(
        render_to_lines(wrapped, TuiSize::new(2, 2)),
        vec!["日", "本"],
    );
}

#[test]
fn applies_its_style_to_painted_cells() {
    let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let text = TuiText::new("a").with_style(style);
    let buffer = render_to_frame(text, TuiSize::new(1, 1)).buffer;

    let cell = &buffer[(0, 0)];
    assert_eq!(cell.symbol(), "a");
    assert_eq!(cell.fg, Color::Red);
    assert!(cell.modifier.contains(Modifier::BOLD));
}

#[test]
fn truncation_keeps_one_row_per_hard_line() {
    let text = TuiText::new("a\nb\nc").truncate();
    assert_eq!(text.desired_height(10), 3);
    assert_eq!(
        render_to_lines(text, TuiSize::new(3, 3)),
        vec!["a  ", "b  ", "c  "],
    );
}

#[test]
fn spans_flow_as_one_paragraph_with_per_span_styles() {
    let green = Style::default().fg(Color::Green);
    let text = TuiText::from_spans([
        ("✓ ".to_owned(), green),
        ("done".to_owned(), Style::default()),
    ])
    .with_style(Style::default().fg(Color::White));

    let buffer = render_to_frame(text, TuiSize::new(6, 1)).buffer;

    assert_eq!(buffer.to_lines(), vec!["✓ done"]);
    // The span's style patches over the base style.
    assert_eq!(buffer[(0, 0)].fg, Color::Green);
    assert_eq!(buffer[(2, 0)].fg, Color::White);
}

#[test]
fn spans_wrap_across_span_boundaries() {
    // "aa bb cc" wraps at width 5 as "aa bb" / "cc", even though the wrap
    // point falls inside the second span.
    let text = TuiText::from_spans([
        ("aa ".to_owned(), Style::default()),
        ("bb cc".to_owned(), Style::default()),
    ]);
    assert_eq!(text.desired_height(5), 2);
    assert_eq!(
        render_to_lines(text, TuiSize::new(5, 2)),
        vec!["aa bb", "cc   "],
    );
}

#[test]
fn hard_newlines_inside_spans_split_lines() {
    let text = TuiText::from_spans([
        ("a\nb".to_owned(), Style::default()),
        ("c".to_owned(), Style::default()),
    ]);
    assert_eq!(text.desired_height(10), 2);
    assert_eq!(
        render_to_lines(text, TuiSize::new(3, 2)),
        vec!["a  ", "bc "],
    );
}

#[test]
fn all_empty_spans_occupy_no_rows() {
    let text = TuiText::from_spans([
        (String::new(), Style::default()),
        (String::new(), Style::default()),
    ]);
    assert_eq!(text.desired_height(10), 0);
}
