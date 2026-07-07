use pathfinder_color::ColorU;
use warpui_core::elements::Align;
use warpui_core::Element;

use super::onboarding_visual::{OnboardingVisual, Pill, RectPct};

pub(crate) fn intention_visual(
    panel_background: ColorU,
    neutral: ColorU,
    blue: ColorU,
    green: ColorU,
    yellow: ColorU,
) -> Box<dyn Element> {
    // X is in percent of the inner panel width; negative values intentionally protrude.
    const LEFT_PROTRUSION_PCT: f32 = -0.06;
    const RIGHT_PROTRUSION_PCT: f32 = 0.03;
    const BAR_H_PCT: f32 = 0.040;

    let mut pills = Vec::new();

    // Top-left neutral bars.
    let top_left = [(0.38, 0.18), (0.55, 0.24), (0.42, 0.30), (0.48, 0.36)];
    pills.extend(top_left.into_iter().map(|(w_pct, y_pct)| Pill {
        rect: RectPct::new(LEFT_PROTRUSION_PCT, y_pct, w_pct, BAR_H_PCT),
        color: neutral,
    }));

    // Upper-right blue bars.
    let blue_bars = [(0.26, 0.12), (0.16, 0.18), (0.34, 0.48), (0.26, 0.54)];
    pills.extend(blue_bars.into_iter().map(|(w_pct, y_pct)| Pill {
        rect: RectPct::new(1.0 - w_pct + RIGHT_PROTRUSION_PCT, y_pct, w_pct, BAR_H_PCT),
        color: blue,
    }));

    // Mid section.
    pills.push(Pill {
        rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.60, 0.70, BAR_H_PCT),
        color: neutral,
    });
    pills.push(Pill {
        rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.66, 0.52, BAR_H_PCT),
        color: neutral,
    });
    pills.push(Pill {
        rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.60, 0.32, BAR_H_PCT),
        color: neutral,
    });

    // Bottom container + rows.
    const BOTTOM_Y_PCT: f32 = 0.80;
    const ROW_GAP_PCT: f32 = 0.012;

    let bottom_w = 1.0 + (-LEFT_PROTRUSION_PCT) + RIGHT_PROTRUSION_PCT;

    let row_x = LEFT_PROTRUSION_PCT + 0.02;
    let row_w = bottom_w - 0.04;
    let mut row_y = BOTTOM_Y_PCT + 0.03;

    // Row 1: full green.
    pills.push(Pill {
        rect: RectPct::new(row_x, row_y, row_w, BAR_H_PCT),
        color: green,
    });
    row_y += BAR_H_PCT + ROW_GAP_PCT;

    // Row 2: mostly green.
    pills.push(Pill {
        rect: RectPct::new(row_x, row_y, row_w * 0.82, BAR_H_PCT),
        color: green,
    });
    row_y += BAR_H_PCT + ROW_GAP_PCT;

    // Row 3: yellow.
    pills.push(Pill {
        rect: RectPct::new(row_x, row_y, row_w * 0.90, BAR_H_PCT),
        color: yellow,
    });
    row_y += BAR_H_PCT + ROW_GAP_PCT;

    // Row 4: shorter green.
    pills.push(Pill {
        rect: RectPct::new(row_x, row_y, row_w * 0.28, BAR_H_PCT),
        color: green,
    });

    Align::new(OnboardingVisual::new(panel_background, pills, true).finish()).finish()
}
