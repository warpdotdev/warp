use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::elements::tui::{
    TuiBuffer, TuiBufferExt, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
};

fn buffer(width: u16, height: u16) -> TuiBuffer {
    TuiBuffer::empty(TuiRect::new(0, 0, width, height))
}

#[test]
fn to_lines_renders_rows_and_pads_with_blanks() {
    let mut b = buffer(5, 1);
    b.set_string(0, 0, "abc", Style::default());
    assert_eq!(b.to_lines(), vec!["abc  "]);
}

#[test]
fn to_lines_reads_text_written_at_an_offset() {
    let mut b = buffer(5, 2);
    b.set_string(2, 1, "hi", Style::default());
    assert_eq!(b.to_lines(), vec!["     ".to_owned(), "  hi ".to_owned()]);
}

#[test]
fn to_lines_collapses_a_wide_grapheme_trailing_column() {
    let mut b = buffer(4, 1);
    b.set_string(0, 0, "界a", Style::default());

    assert_eq!(b.to_lines(), vec!["界a "]);
    assert_eq!(b[(0, 0)].symbol(), "界");
    assert_eq!(b[(2, 0)].symbol(), "a");
}

#[test]
fn to_lines_keeps_a_combining_grapheme_in_one_cell() {
    let mut b = buffer(4, 1);
    b.set_string(0, 0, "e\u{301}x", Style::default());

    assert_eq!(b.to_lines(), vec!["e\u{301}x  "]);
    assert_eq!(b[(0, 0)].symbol(), "e\u{301}");
    assert_eq!(b[(1, 0)].symbol(), "x");
}

#[test]
fn set_string_drops_a_wide_grapheme_that_would_cross_the_edge() {
    let mut b = buffer(3, 1);
    b.set_string(0, 0, "ab界", Style::default());
    assert_eq!(b.to_lines(), vec!["ab "]);
}

#[test]
fn styled_writes_round_trip_through_cells() {
    let mut b = buffer(2, 1);
    b.set_string(0, 0, "x", Style::default().fg(Color::Red));

    assert_eq!(b[(0, 0)].symbol(), "x");
    assert_eq!(b[(0, 0)].fg, Color::Red);
}

/// Maps negative absolute positions onto a scratch buffer without exposing its origin.
#[test]
fn mapped_surface_writes_negative_absolute_positions() {
    let mut b = buffer(2, 1);
    {
        let mut surface = TuiPaintSurface::mapped(&mut b, TuiScreenPosition::new(-5, -2));
        surface
            .cell_mut(TuiScreenPosition::new(-4, -2))
            .unwrap()
            .set_symbol("x");
    }

    assert_eq!(b.to_lines(), vec![" x"]);
}

/// Rejects positions outside the active buffer instead of clamping them.
#[test]
fn surface_writes_outside_the_mapping_fail_closed() {
    let mut b = buffer(2, 1);
    {
        let mut surface = TuiPaintSurface::mapped(&mut b, TuiScreenPosition::new(-5, -2));
        assert!(surface.cell_mut(TuiScreenPosition::new(-6, -2)).is_none());
        assert!(
            surface
                .cell_mut(TuiScreenPosition::new(i32::MAX, -2))
                .is_none()
        );
    }

    assert_eq!(b.to_lines(), vec!["  "]);
}

#[test]
fn widget_renders_only_visible_rows() {
    let mut b = buffer(3, 2);
    let mut surface = TuiPaintSurface::new(&mut b);
    assert!(surface.render_widget(
        TuiScreenPosition::new(0, -2),
        TuiSize::new(3, 4),
        Paragraph::new(vec![
            Line::from("a"),
            Line::from("b"),
            Line::from("c"),
            Line::from("d"),
        ]),
    ));

    assert_eq!(b.to_lines(), vec!["c  ", "d  "]);
}
#[test]
fn widget_renders_visible_columns_when_horizontally_clipped() {
    let mut b = buffer(4, 2);
    let mut surface = TuiPaintSurface::new(&mut b);
    assert_eq!(
        surface.with_clip(
            TuiScreenPosition::new(1, 0),
            TuiSize::new(2, 2),
            |surface| {
                surface.render_widget(
                    TuiScreenPosition::new(0, -1),
                    TuiSize::new(4, 3),
                    Paragraph::new(vec![
                        Line::from("abcd"),
                        Line::from("efgh"),
                        Line::from("ijkl"),
                    ]),
                )
            },
        ),
        Some(true),
    );
    assert_eq!(b.to_lines(), vec![" fg ", " jk "]);
}

#[test]
fn set_style_clips_negative_screen_bounds() {
    let mut b = buffer(2, 2);
    let mut surface = TuiPaintSurface::new(&mut b);

    surface.set_style(
        TuiScreenPosition::new(0, -1),
        TuiSize::new(2, 2),
        Style::default().fg(Color::Red),
    );

    assert_eq!(b[(0, 0)].fg, Color::Red);
    assert_eq!(b[(0, 1)].fg, Color::Reset);
}

#[test]
fn nested_surface_clip_contains_cells_styles_and_widgets() {
    let mut b = buffer(3, 4);
    let mut surface = TuiPaintSurface::new(&mut b);

    surface.with_clip(
        TuiScreenPosition::new(0, 1),
        TuiSize::new(3, 2),
        |surface| {
            assert!(surface.cell_mut(TuiScreenPosition::new(0, 0)).is_none());
            surface
                .cell_mut(TuiScreenPosition::new(0, 1))
                .unwrap()
                .set_symbol("x");
            surface.set_style(
                TuiScreenPosition::new(0, 0),
                TuiSize::new(3, 4),
                Style::default().fg(Color::Red),
            );
            assert!(surface.render_widget(
                TuiScreenPosition::new(0, 0),
                TuiSize::new(3, 4),
                Paragraph::new(vec![
                    Line::from("a"),
                    Line::from("b"),
                    Line::from("c"),
                    Line::from("d"),
                ]),
            ));
        },
    );

    assert_eq!(b.to_lines(), vec!["   ", "b  ", "c  ", "   "]);
    assert_eq!(b[(0, 0)].fg, Color::Reset);
    assert_eq!(b[(0, 1)].fg, Color::Red);
    assert_eq!(b[(0, 2)].fg, Color::Red);
    assert_eq!(b[(0, 3)].fg, Color::Reset);
}

#[test]
fn visible_rows_are_relative_to_the_element_origin() {
    let mut b = buffer(3, 4);
    let mut surface = TuiPaintSurface::new(&mut b);

    assert_eq!(
        surface.with_clip(
            TuiScreenPosition::new(0, 1),
            TuiSize::new(3, 2),
            |surface| surface.visible_rows(TuiScreenPosition::new(0, -2), TuiSize::new(3, 6),),
        ),
        Some(Some(3..5)),
    );
}
