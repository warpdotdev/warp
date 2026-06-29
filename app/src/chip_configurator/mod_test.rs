//! Tests for the configurator control-chip label color.
//!
//! These guard the readability fix for the Settings "Header toolbar layout"
//! chips: the old code colored the labels with `sub_text_color` (a 60%-opacity
//! sub-text color), which composited to a faint mid-grey on light themes and
//! could drop below WCAG AA. The contrast helpers are alpha-blind, so we assert
//! against the *composited* color (label blended over the chip surface) — i.e.
//! what the user actually sees.

use pathfinder_color::ColorU;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::contrast::{high_enough_contrast, MinimumAllowedContrast};
use warp_core::ui::theme::{mock_terminal_colors, Details, Fill, WarpTheme};

use super::control_chip_text_color;

/// Builds a solid-background/foreground theme (colors as `0xRRGGBBAA`). The
/// terminal palette does not affect text/surface contrast, so a mock is fine.
fn theme_with(background: u32, foreground: u32, details: Details) -> WarpTheme {
    WarpTheme::new(
        Fill::Solid(ColorU::from_u32(background)),
        ColorU::from_u32(foreground),
        Fill::Solid(ColorU::from_u32(0x2AA198FF)),
        None,
        Some(details),
        mock_terminal_colors(),
        None,
        Some("contrast-test".to_string()),
    )
}

/// Whether a chip label drawn with `label` (which may be translucent) meets
/// WCAG AA once composited over `surface` — the on-screen appearance.
fn label_is_readable(surface: Fill, label: ColorU) -> bool {
    let background = surface.into_solid();
    let rendered = background.blend(&label);
    high_enough_contrast(rendered, background, MinimumAllowedContrast::Text)
}

#[test]
fn legacy_sub_text_chip_label_was_unreadable_but_fix_is_readable() {
    // Gruvbox Light (recreated from the bundled definition to avoid depending on
    // `pub(super)` theme builders) is a real light theme where the old muted
    // 60%-opacity sub-text color composites below WCAG AA — the exact class of
    // theme that produced the reported faint/illegible chip labels.
    let theme = theme_with(0xFBF1C7FF, 0x3C3836FF, Details::Lighter);
    let surface = theme.surface_1();

    let legacy = theme.sub_text_color(surface).into_solid();
    assert!(
        !label_is_readable(surface, legacy),
        "precondition: the old sub_text chip label color should be sub-AA on Gruvbox Light"
    );

    let fixed = control_chip_text_color(&theme, surface);
    assert!(
        label_is_readable(surface, fixed),
        "the fixed chip label color must meet WCAG AA on Gruvbox Light"
    );
}

#[test]
fn chip_label_is_readable_across_themes_and_surfaces() {
    let themes = [
        theme_with(0xFFFFFFFF, 0x111111FF, Details::Lighter), // Light
        theme_with(0xFDF6E3FF, 0x586E75FF, Details::Lighter), // Solarized Light
        theme_with(0xFBF1C7FF, 0x3C3836FF, Details::Lighter), // Gruvbox Light
        theme_with(0x000000FF, 0xFFFFFFFF, Details::Darker),  // Dark
        theme_with(0x282A36FF, 0xF8F8F2FF, Details::Darker),  // Dracula
    ];
    for theme in themes {
        // surface_1 (default) and surface_2 (hover) are both used by chips.
        for surface in [theme.surface_1(), theme.surface_2()] {
            let label = control_chip_text_color(&theme, surface);
            assert!(
                label_is_readable(surface, label),
                "chip label color must meet WCAG AA on every theme and surface"
            );
        }
    }
}
