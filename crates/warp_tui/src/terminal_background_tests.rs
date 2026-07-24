use warp::settings::TuiTheme;
use warpui_core::runtime::{ProbedRgb, ProbedTerminalColors};

use super::select_theme;

#[test]
fn automatic_theme_follows_the_probed_background() {
    let light = ProbedTerminalColors {
        bg: Some(ProbedRgb {
            r: 255,
            g: 255,
            b: 255,
        }),
        ..Default::default()
    };
    let dark = ProbedTerminalColors {
        bg: Some(ProbedRgb { r: 0, g: 0, b: 0 }),
        ..Default::default()
    };

    assert_eq!(
        select_theme(TuiTheme::Auto, light).name().as_deref(),
        Some("Light")
    );
    assert_eq!(
        select_theme(TuiTheme::Auto, dark).name().as_deref(),
        Some("Dark")
    );
}

#[test]
fn explicit_theme_overrides_the_probed_background() {
    let light = ProbedTerminalColors {
        bg: Some(ProbedRgb {
            r: 255,
            g: 255,
            b: 255,
        }),
        ..Default::default()
    };

    assert_eq!(
        select_theme(TuiTheme::Dark, light).name().as_deref(),
        Some("Dark")
    );
}
