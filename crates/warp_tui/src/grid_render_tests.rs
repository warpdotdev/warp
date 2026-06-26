use warp::tui_export::{TerminalColorList, TerminalColors};
use warp_terminal::model::ansi::{Color, NamedColor};
use warp_terminal::model::grid::cell::{Cell, Flags};
use warpui_core::elements::tui::{Color as TuiColor, Modifier};

use super::cell_to_style;

fn test_colors() -> TerminalColorList {
    TerminalColorList::from(&TerminalColors::default())
}

#[test]
fn terminal_cell_style_preserves_theme_colors() {
    let colors = test_colors();
    let cell = Cell::default();
    let style = cell_to_style(&cell, &colors);
    let foreground = colors[NamedColor::Foreground.into_color_index()];
    let background = colors[NamedColor::Background.into_color_index()];
    assert_eq!(
        style.fg,
        Some(TuiColor::Rgb(foreground.r, foreground.g, foreground.b))
    );
    assert_eq!(
        style.bg,
        Some(TuiColor::Rgb(background.r, background.g, background.b))
    );
}

#[test]
fn terminal_cell_style_preserves_flags_and_rgb() {
    let colors = test_colors();
    let mut cell = Cell::default();
    cell.fg = Color::Spec(pathfinder_color::ColorU::new(0xff, 0x00, 0x00, 0xff));
    cell.flags.insert(Flags::BOLD | Flags::UNDERLINE);
    let style = cell_to_style(&cell, &colors);
    assert_eq!(style.fg, Some(TuiColor::Rgb(0xff, 0x00, 0x00)));
    assert!(style.add_modifier.contains(Modifier::BOLD));
    assert!(style.add_modifier.contains(Modifier::UNDERLINED));
}
