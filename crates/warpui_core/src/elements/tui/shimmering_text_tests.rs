use std::time::Duration;

use ratatui::style::{Color, Modifier};

use super::TuiShimmeringText;
use crate::color::ColorU;
use crate::elements::animation::AnimationClock;
use crate::elements::shimmer_math::ShimmerConfig;
use crate::elements::tui::test_support::with_paint_context;
use crate::elements::tui::{TuiBuffer, TuiElement, TuiRect};

const BASE: ColorU = ColorU {
    r: 254,
    g: 253,
    b: 194,
    a: 255,
};
const SHIMMER: ColorU = ColorU {
    r: 254,
    g: 255,
    b: 255,
    a: 255,
};

fn element(initial_elapsed: Duration) -> TuiShimmeringText {
    TuiShimmeringText::new(
        "Warping",
        BASE,
        SHIMMER,
        ShimmerConfig::default(),
        AnimationClock::starting_at(initial_elapsed),
    )
    .with_modifier(Modifier::BOLD)
}

/// Renders `element` into a 10x1 buffer, returning the buffer and whether a
/// repaint was requested.
fn render(element: &TuiShimmeringText) -> (TuiBuffer, bool) {
    let mut buffer = TuiBuffer::empty(TuiRect::new(0, 0, 10, 1));
    let requested_repaint = with_paint_context(|ctx| {
        element.render(TuiRect::new(0, 0, 10, 1), &mut buffer, ctx);
        ctx.requested_repaint_at().is_some()
    });
    (buffer, requested_repaint)
}

#[test]
fn paints_base_color_before_the_band_reaches_the_text() {
    // At t=0 the band center sits `padding` glyphs before the text, farther
    // than `shimmer_radius` from every glyph, so every cell is the base color.
    let (buffer, _) = render(&element(Duration::ZERO));
    for (index, char) in "Warping".chars().enumerate() {
        let cell = &buffer[(index as u16, 0)];
        assert_eq!(cell.symbol(), char.to_string());
        assert_eq!(cell.fg, Color::Rgb(BASE.r, BASE.g, BASE.b));
        assert!(cell.modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn paints_the_shimmer_color_at_the_band_center_mid_sweep() {
    let config = ShimmerConfig::default();
    // Half a period in: progress 0.5, so the center is at glyph
    // 0.5 * ((7 - 1) + 2 * padding) - padding = 3.
    let (buffer, _) = render(&element(config.period / 2));
    let center_cell = &buffer[(3, 0)];
    assert_eq!(center_cell.fg, Color::Rgb(SHIMMER.r, SHIMMER.g, SHIMMER.b));
    // A glyph at the band's edge is only partially lerped toward the shimmer.
    let edge_cell = &buffer[(0, 0)];
    assert_ne!(center_cell.fg, edge_cell.fg);
    assert_ne!(edge_cell.fg, Color::Rgb(BASE.r, BASE.g, BASE.b));
}

#[test]
fn requests_a_repaint_every_paint() {
    let (_, requested_repaint) = render(&element(Duration::ZERO));
    assert!(requested_repaint);
}
