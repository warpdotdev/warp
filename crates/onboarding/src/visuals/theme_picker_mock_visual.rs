use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::Align;
use warpui_core::Element;

use super::onboarding_visual::{OnboardingVisual, Pill, RectPct};

pub(crate) fn theme_picker_visual(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();

    let panel_background = internal_colors::neutral_2(theme);
    let neutral = internal_colors::neutral_4(theme);

    let yellow = theme.ansi_fg_yellow();
    let red = theme.ansi_fg_red();
    let blue = theme.ansi_fg_blue();
    let green = theme.ansi_fg_green();
    let magenta = theme.ansi_fg_magenta();

    // X is in percent of the inner panel width; negative values intentionally protrude.
    const LEFT_PROTRUSION_PCT: f32 = -0.06;
    const BAR_H_PCT: f32 = 0.038;

    let pills = [
        // Top-left neutral bars.
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.18, 0.30, BAR_H_PCT),
            color: neutral,
        },
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.24, 0.50, BAR_H_PCT),
            color: neutral,
        },
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.30, 0.36, BAR_H_PCT),
            color: neutral,
        },
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.36, 0.42, BAR_H_PCT),
            color: neutral,
        },
        // Upper-right colored bars.
        Pill {
            rect: RectPct::new(0.52 + LEFT_PROTRUSION_PCT, 0.24, 0.28, BAR_H_PCT),
            color: yellow,
        },
        Pill {
            rect: RectPct::new(0.44 + LEFT_PROTRUSION_PCT, 0.36, 0.14, BAR_H_PCT),
            color: red,
        },
        // Mid section.
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.62, 0.68, BAR_H_PCT),
            color: blue,
        },
        Pill {
            rect: RectPct::new(0.70 + LEFT_PROTRUSION_PCT, 0.62, 0.18, BAR_H_PCT),
            color: green,
        },
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.68, 0.40, BAR_H_PCT),
            color: neutral,
        },
        // Bottom section.
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.80, 0.44, BAR_H_PCT),
            color: neutral,
        },
        Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.86, 0.28, BAR_H_PCT),
            color: neutral,
        },
        Pill {
            rect: RectPct::new(0.30 + LEFT_PROTRUSION_PCT, 0.86, 0.30, BAR_H_PCT),
            color: magenta,
        },
    ];

    Align::new(OnboardingVisual::new(panel_background, pills.to_vec(), false).finish()).finish()
}
