//! Tests for [`readable_chip_label_color`], the shared chip label/icon color
//! used by the agent input footer context chips and by the configurator control
//! chips (agent toolbelt + header toolbar editors).
//!
//! The old code colored chip labels with `sub_text_color` (a 60%-opacity
//! sub-text color). The contrast helpers are alpha-blind, so that muted color
//! composited to a faint mid-grey that could drop below WCAG AA on light
//! themes, making labels hard to read. We assert against the *composited* color
//! (label blended over the chip surface) — what the user actually sees.

use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::contrast::{MinimumAllowedContrast, high_enough_contrast};
use warp_core::ui::theme::{Details, Fill, WarpTheme, mock_terminal_colors};
use warpui::color::ColorU;

use super::readable_chip_label_color;

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

/// Whether `label` (which may be translucent) meets WCAG AA once composited over
/// `background` — i.e. the on-screen appearance.
fn composited_is_readable(background: Fill, label: ColorU) -> bool {
    let bg = background.into_solid();
    high_enough_contrast(bg.blend(&label), bg, MinimumAllowedContrast::Text)
}

#[test]
fn chip_label_color_recovers_readability_on_low_contrast_light_theme() {
    // Gruvbox Light (recreated from the bundled definition) is a real light
    // theme where the muted 60%-opacity sub-text color composites below WCAG AA
    // — the class of theme that produced the reported faint chip labels.
    let theme = theme_with(0xFBF1C7FF, 0x3C3836FF, Details::Lighter);
    let background = theme.surface_1();

    let legacy = theme.sub_text_color(background).into_solid();
    assert!(
        !composited_is_readable(background, legacy),
        "precondition: the muted sub_text color should be sub-AA on Gruvbox Light"
    );

    let fixed = readable_chip_label_color(&theme, background);
    assert!(
        composited_is_readable(background, fixed),
        "the chip label color must meet WCAG AA on Gruvbox Light"
    );
}

#[test]
fn chip_label_color_is_readable_across_themes_and_surfaces() {
    let themes = [
        theme_with(0xFFFFFFFF, 0x111111FF, Details::Lighter), // Light
        theme_with(0xFDF6E3FF, 0x586E75FF, Details::Lighter), // Solarized Light
        theme_with(0xFBF1C7FF, 0x3C3836FF, Details::Lighter), // Gruvbox Light
        theme_with(0x000000FF, 0xFFFFFFFF, Details::Darker),  // Dark
        theme_with(0x282A36FF, 0xF8F8F2FF, Details::Darker),  // Dracula
    ];
    for theme in themes {
        // surface_1 (default) and surface_2 (hover) are both used by chips.
        for background in [theme.surface_1(), theme.surface_2()] {
            let label = readable_chip_label_color(&theme, background);
            assert!(
                composited_is_readable(background, label),
                "chip label color must meet WCAG AA on every theme and surface"
            );
        }
    }
}

#[test]
fn chip_label_color_preserves_muted_color_where_already_legible() {
    // On a dark theme the muted sub_text color already passes AA, so the helper
    // must keep it unchanged — the fix should not boldening dark-theme chips.
    let theme = theme_with(0x000000FF, 0xFFFFFFFF, Details::Darker);
    let background = theme.surface_1();
    let muted = theme.sub_text_color(background).into_solid();

    assert_eq!(
        readable_chip_label_color(&theme, background),
        muted,
        "dark-theme chips should keep their muted (unchanged) color"
    );
}
