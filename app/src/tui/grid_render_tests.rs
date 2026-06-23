//! Tests for `cell_to_style` and `render_grid`.

use warp_terminal::model::ansi::{Color, NamedColor};
use warp_terminal::model::grid::cell::{Cell, Flags};
use warpui_core::elements::tui::{Color as TuiColor, Modifier};

use super::cell_to_style;
use crate::terminal::color;
use crate::terminal::color::Colors;

/// Returns a default `color::List` for testing.
fn test_colors() -> color::List {
    color::List::from(&Colors::default())
}

/// A default cell (NUL char, default fg/bg, no flags) should produce a style
/// with the theme's foreground and background colors.
#[test]
fn default_cell_uses_theme_colors() {
    let colors = test_colors();
    let cell = Cell::default();
    let style = cell_to_style(&cell, &colors);
    let fg = colors[NamedColor::Foreground.into_color_index()];
    let bg = colors[NamedColor::Background.into_color_index()];
    assert_eq!(style.fg, Some(TuiColor::Rgb(fg.r, fg.g, fg.b)));
    assert_eq!(style.bg, Some(TuiColor::Rgb(bg.r, bg.g, bg.b)));
}

/// A cell with BOLD flag should have the BOLD modifier.
#[test]
fn bold_flag_adds_bold_modifier() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.flags.insert(Flags::BOLD);
    let style = cell_to_style(&cell, &colors);
    assert!(style.add_modifier.contains(Modifier::BOLD));
}

/// A cell with ITALIC flag should have the ITALIC modifier.
#[test]
fn italic_flag_adds_italic_modifier() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.flags.insert(Flags::ITALIC);
    let style = cell_to_style(&cell, &colors);
    assert!(style.add_modifier.contains(Modifier::ITALIC));
}

/// A cell with UNDERLINE flag should have the UNDERLINE modifier.
#[test]
fn underline_flag_adds_underline_modifier() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.flags.insert(Flags::UNDERLINE);
    let style = cell_to_style(&cell, &colors);
    assert!(style.add_modifier.contains(Modifier::UNDERLINED));
}

/// A cell with INVERSE flag should have the REVERSED modifier.
#[test]
fn inverse_flag_adds_reversed_modifier() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.flags.insert(Flags::INVERSE);
    let style = cell_to_style(&cell, &colors);
    assert!(style.add_modifier.contains(Modifier::REVERSED));
}

/// A cell with a Spec (RGB) foreground color should map to TuiColor::Rgb.
#[test]
fn spec_color_maps_to_rgb() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.fg = Color::Spec(pathfinder_color::ColorU::new(0xff, 0x00, 0x00, 0xff));
    let style = cell_to_style(&cell, &colors);
    assert_eq!(style.fg, Some(TuiColor::Rgb(0xff, 0x00, 0x00)));
}

/// A cell with a Named color should resolve to the theme's RGB for that name.
#[test]
fn named_color_resolves_to_theme_rgb() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.fg = Color::Named(NamedColor::Red);
    let style = cell_to_style(&cell, &colors);
    let expected = colors[NamedColor::Red.into_color_index()];
    assert_eq!(
        style.fg,
        Some(TuiColor::Rgb(expected.r, expected.g, expected.b))
    );
}

/// A cell with HIDDEN flag should have the HIDDEN modifier.
#[test]
fn hidden_flag_adds_hidden_modifier() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.flags.insert(Flags::HIDDEN);
    let style = cell_to_style(&cell, &colors);
    assert!(style.add_modifier.contains(Modifier::HIDDEN));
}

/// A cell with STRIKEOUT flag should have the CROSSED_OUT modifier.
#[test]
fn strikeout_flag_adds_crossed_out_modifier() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.flags.insert(Flags::STRIKEOUT);
    let style = cell_to_style(&cell, &colors);
    assert!(style.add_modifier.contains(Modifier::CROSSED_OUT));
}
