use std::rc::Rc;

use ratatui::style::{Color, Modifier, Style};

use super::TuiText;
use crate::elements::tui::test_support::{render_to_frame, render_to_lines};
use crate::elements::tui::{
    TuiBufferExt, TuiConstraint, TuiElement, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiScreenPosition, TuiSize,
};
use crate::{App, AppContext, EntityIdMap};

#[test]
fn renders_a_single_short_line() {
    let text = TuiText::new("hello");
    assert_eq!(
        render_to_lines(text, TuiSize::new(10, 1)),
        vec!["hello     "],
    );
}
#[test]
fn ellipsis_stays_inside_the_assigned_width() {
    assert_eq!(
        render_to_lines(
            TuiText::new("infrastructure").truncate_with_ellipsis(),
            TuiSize::new(8, 1),
        ),
        vec!["infra..."],
    );
    assert_eq!(
        render_to_lines(
            TuiText::new("abcdef").truncate_with_ellipsis(),
            TuiSize::new(2, 1),
        ),
        vec![".."],
    );
}

#[test]
fn ellipsis_preserves_graphemes_and_span_style() {
    let yellow = Style::default().fg(Color::Yellow);
    let buffer = render_to_frame(
        TuiText::from_spans([
            ("e\u{301}cl".to_owned(), yellow),
            ("air".to_owned(), Style::default()),
        ])
        .truncate_with_ellipsis(),
        TuiSize::new(5, 1),
    )
    .buffer;

    assert_eq!(buffer.to_lines(), vec!["e\u{301}c..."]);
    assert_eq!(buffer[(2, 0)].fg, Color::Yellow);
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
fn truncation_invalidates_a_cached_wrapped_measurement() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let constraint = TuiConstraint::loose(TuiSize::new(5, 10));
            for truncate in [
                TuiText::truncate as fn(TuiText) -> TuiText,
                TuiText::truncate_with_ellipsis,
            ] {
                let mut text = TuiText::new("hello world");
                let mut rendered_views = EntityIdMap::default();
                let mut ctx = TuiLayoutContext {
                    rendered_views: &mut rendered_views,
                };
                assert_eq!(
                    text.layout(constraint, &mut ctx, app_ctx),
                    TuiSize::new(5, 2)
                );

                text = truncate(text);
                assert_eq!(
                    text.layout(constraint, &mut ctx, app_ctx),
                    TuiSize::new(5, 1)
                );
            }
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

/// Style carrying the hyperlink sentinel index 0 (see `TuiText::with_hyperlinks`).
fn sentinel_style() -> Style {
    Style::default().underline_color(Color::Indexed(0))
}

/// Asserts no cell in `buffer`'s `width`-wide, single row leaked the sentinel
/// `underline_color` visibly.
fn assert_no_leaked_sentinel(buffer: &crate::elements::tui::TuiBuffer, width: u16) {
    for x in 0..width {
        assert_eq!(
            buffer[(x, 0)].underline_color,
            Color::Reset,
            "column {x} must not leak the hyperlink sentinel color"
        );
    }
}

#[test]
fn hyperlink_sentinel_is_recorded_and_cleared_from_the_painted_cell() {
    let text = TuiText::from_spans([("link".to_owned(), sentinel_style())])
        .with_hyperlinks(vec![Rc::from("https://warp.dev")]);
    let frame = render_to_frame(text, TuiSize::new(4, 1));

    assert_eq!(frame.buffer.to_lines(), vec!["link"]);
    assert_no_leaked_sentinel(&frame.buffer, 4);
    let url: Rc<str> = Rc::from("https://warp.dev");
    for x in 0..4 {
        assert_eq!(frame.hyperlinks.get(&(x, 0)), Some(&url));
    }
}

#[test]
fn hyperlink_sentinel_is_cleared_from_both_columns_of_a_wide_grapheme() {
    let text = TuiText::from_spans([("界".to_owned(), sentinel_style())])
        .with_hyperlinks(vec![Rc::from("https://warp.dev")]);
    let frame = render_to_frame(text, TuiSize::new(2, 1));

    assert_eq!(frame.buffer.to_lines(), vec!["界"]);
    assert_no_leaked_sentinel(&frame.buffer, 2);
    let url: Rc<str> = Rc::from("https://warp.dev");
    assert_eq!(
        frame.hyperlinks.get(&(0, 0)),
        Some(&url),
        "the wide grapheme's leading column should be linked"
    );
}

#[test]
fn hyperlink_sentinel_is_cleared_from_ellipsized_text() {
    let text = TuiText::from_spans([("infrastructure".to_owned(), sentinel_style())])
        .with_hyperlinks(vec![Rc::from("https://warp.dev")])
        .truncate_with_ellipsis();
    let frame = render_to_frame(text, TuiSize::new(8, 1));

    assert_eq!(frame.buffer.to_lines(), vec!["infra..."]);
    assert_no_leaked_sentinel(&frame.buffer, 8);
    let url: Rc<str> = Rc::from("https://warp.dev");
    for x in 0..5 {
        assert_eq!(
            frame.hyperlinks.get(&(x, 0)),
            Some(&url),
            "column {x} (\"infra\") should be linked"
        );
    }
}

/// A single-child wrapper that narrows the active clip to a fixed window,
/// forcing `TuiPaintSurface::render_widget`'s internal scratch-buffer copy
/// path whenever the child's own rendered width exceeds that window.
struct ClippedWindow {
    child: Box<dyn TuiElement>,
    clip_start_x: i32,
    clip_width: u16,
}

impl TuiElement for ClippedWindow {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child.layout(constraint, ctx, app);
        TuiSize::new(self.clip_width, constraint.max.height)
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        let clip_origin = origin.offset(self.clip_start_x, 0);
        surface.with_clip(clip_origin, TuiSize::new(self.clip_width, 1), |surface| {
            self.child.render(origin, surface, ctx);
        });
    }

    fn size(&self) -> Option<TuiSize> {
        self.child.size()
    }
}

#[test]
fn hyperlink_recording_respects_an_active_clip_and_the_scratch_copy_path() {
    let text = TuiText::from_spans([("0123456789".to_owned(), sentinel_style())])
        .with_hyperlinks(vec![Rc::from("https://warp.dev")]);
    let clipped = ClippedWindow {
        child: text.finish(),
        clip_start_x: 2,
        clip_width: 6,
    };

    let frame = render_to_frame(clipped, TuiSize::new(10, 1));

    // Only the clipped window (columns 2..8) is actually painted; the rest
    // stays blank, proving `render_widget`'s scratch-buffer copy path (used
    // whenever only part of the row is visible) ran.
    assert_eq!(frame.buffer.to_lines(), vec!["  234567  "]);
    assert_no_leaked_sentinel(&frame.buffer, 10);
    let url: Rc<str> = Rc::from("https://warp.dev");
    for x in 2..8 {
        assert_eq!(
            frame.hyperlinks.get(&(x, 0)),
            Some(&url),
            "column {x} is inside the clip and should be linked"
        );
    }
    for x in [0, 1, 8, 9] {
        assert!(
            !frame.hyperlinks.contains_key(&(x, 0)),
            "column {x} was clipped and never painted, so it must not be linked"
        );
    }
}
