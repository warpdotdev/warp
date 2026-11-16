use std::time::Duration;

use ratatui::style::{Color, Modifier};

use super::TuiShimmeringText;
use crate::color::ColorU;
use crate::elements::animation::AnimationClock;
use crate::elements::shimmer_math::ShimmerConfig;
use crate::elements::tui::test_support::render_to_frame;
use crate::elements::tui::{TuiBuffer, TuiBufferExt, TuiSize};

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
    element_with_text("Warping", initial_elapsed)
}

fn element_with_text(text: &str, initial_elapsed: Duration) -> TuiShimmeringText {
    TuiShimmeringText::new(
        text,
        BASE,
        SHIMMER,
        ShimmerConfig::default(),
        AnimationClock::starting_at(initial_elapsed),
    )
    .with_modifier(Modifier::BOLD)
}

/// Renders `element` into a 10x1 buffer, returning the buffer and whether a
/// repaint was requested.
fn render(element: TuiShimmeringText) -> (TuiBuffer, bool) {
    let frame = render_to_frame(element, TuiSize::new(10, 1));
    let requested_repaint = frame.repaint_at.is_some();
    (frame.buffer, requested_repaint)
}

#[test]
fn paints_base_color_before_the_band_reaches_the_text() {
    // At t=0 the band center sits `padding` glyphs before the text, farther
    // than `shimmer_radius` from every glyph, so every cell is the base color.
    let (buffer, _) = render(element(Duration::ZERO));
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
    let (buffer, _) = render(element(config.period / 2));
    let center_cell = &buffer[(3, 0)];
    assert_eq!(center_cell.fg, Color::Rgb(SHIMMER.r, SHIMMER.g, SHIMMER.b));
    // A glyph at the band's edge is only partially lerped toward the shimmer.
    let edge_cell = &buffer[(0, 0)];
    assert_ne!(center_cell.fg, edge_cell.fg);
    assert_ne!(edge_cell.fg, Color::Rgb(BASE.r, BASE.g, BASE.b));
}
#[test]
fn appends_and_paints_grouped_suffix_with_one_color_across_the_sweep() {
    let config = ShimmerConfig::default();
    for elapsed in [
        config.period / 2,
        config.period * 2 / 3,
        config.period * 3 / 4,
    ] {
        let (buffer, _) = render(element_with_text("Warping", elapsed).with_grouped_suffix("..."));
        assert_eq!(buffer.to_lines()[0], "Warping...");
        let first_dot_color = buffer[(7, 0)].fg;
        assert_eq!(buffer[(8, 0)].fg, first_dot_color);
        assert_eq!(buffer[(9, 0)].fg, first_dot_color);
    }

    let (buffer, _) =
        render(element_with_text("Warping", config.period / 2).with_grouped_suffix("..."));
    assert_ne!(buffer[(7, 0)].fg, Color::Rgb(BASE.r, BASE.g, BASE.b));
}

#[test]
fn leaves_trailing_ellipsis_ungrouped_by_default() {
    let config = ShimmerConfig::default();
    let (buffer, _) = render(element_with_text("Warping...", config.period / 2));
    assert_ne!(buffer[(7, 0)].fg, buffer[(8, 0)].fg);
    assert_ne!(buffer[(8, 0)].fg, buffer[(9, 0)].fg);
}

#[test]
fn requests_a_repaint_every_paint() {
    let (_, requested_repaint) = render(element(Duration::ZERO));
    assert!(requested_repaint);
}
