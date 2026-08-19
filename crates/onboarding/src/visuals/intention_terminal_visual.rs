use pathfinder_color::ColorU;
use warpui_core::Element;
use warpui_core::elements::Align;

use super::onboarding_visual::{OnboardingVisual, Pill, RectPct};
use crate::visuals::onboarding_visual::Rect;

pub(crate) fn intention_terminal_visual(
    panel_background: ColorU,
    neutral: ColorU,
    neutral_highlight: ColorU,
    accent: ColorU,
) -> Box<dyn Element> {
    // X is in percent of the inner panel width; negative values intentionally protrude.
    const LEFT_PROTRUSION_PCT: f32 = -0.06;
    const BAR_H_PCT: f32 = 0.040;
    // ROW_GAP_PCT: f32 = 0.012; <- This is the gap between rows.

    // All bars are neutral_4 and overhang to the left.
    let rows = [
        (0.25, 0.15),
        (0.50, 0.202),
        (0.35, 0.254),
        (0.45, 0.306),
        (0.70, 0.386),
        (0.50, 0.438),
        (0.80, 0.490),
    ];

    let mut pills = rows
        .into_iter()
        .map(|(w_pct, y_pct)| Pill {
            rect: RectPct::new(LEFT_PROTRUSION_PCT, y_pct, w_pct, BAR_H_PCT),
            color: neutral,
        })
        .collect::<Vec<_>>();

    pills.push(Pill {
        rect: RectPct::new(LEFT_PROTRUSION_PCT, 0.08, 0.6, BAR_H_PCT),
        color: neutral_highlight,
    });

    let rects = vec![Rect {
        rect: RectPct::new(0.75, 0.481, 0.01, 0.058),
        color: accent,
    }];

    Align::new(
        OnboardingVisual::new(panel_background, pills, false)
            .with_rects(rects)
            .finish(),
    )
    .finish()
}
