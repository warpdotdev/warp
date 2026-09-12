use warp_core::ui::theme::AnsiColor;

use super::{dark_mode_colors, light_mode_colors};

/// Regression: light-mode normal red/green/cyan must match the values sampled
/// from Erica Luzzi's Figma spec (APP-5029). Dark-mode equivalents must be
/// unchanged.
#[test]
fn light_theme_normal_colors_match_design_spec() {
    let light = light_mode_colors();
    assert_eq!(
        light.normal.red,
        AnsiColor::from_u32(0xB3276FFF),
        "light normal.red must match Figma spec"
    );
    assert_eq!(
        light.normal.green,
        AnsiColor::from_u32(0x4CA47BFF),
        "light normal.green must match Figma spec"
    );
    assert_eq!(
        light.normal.cyan,
        AnsiColor::from_u32(0x4FA3B7FF),
        "light normal.cyan must match Figma spec"
    );

    let dark = dark_mode_colors();
    assert_eq!(
        dark.normal.red,
        AnsiColor::from_u32(0xFF8272FF),
        "dark normal.red must be unchanged"
    );
    assert_eq!(
        dark.normal.green,
        AnsiColor::from_u32(0xB4FA72FF),
        "dark normal.green must be unchanged"
    );
    assert_eq!(
        dark.normal.cyan,
        AnsiColor::from_u32(0xD0D1FEFF),
        "dark normal.cyan must be unchanged"
    );
}
