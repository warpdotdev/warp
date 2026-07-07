use pathfinder_color::ColorU;
use warp_core::ui::Icon;
use warpui_core::elements::Align;
use warpui_core::Element;

use super::onboarding_visual::{IconPct, OnboardingVisual, Pill, RectPct};

pub(crate) fn project_visual(
    panel_background: ColorU,
    pill_color: ColorU,
    center_icon_color: ColorU,
    side_icon_color: ColorU,
) -> Box<dyn Element> {
    const PILL_W_PCT: f32 = 0.23;
    const PILL_H_PCT: f32 = 0.34;

    let x = (1.0 - PILL_W_PCT) / 2.0;
    let y = (1.0 - PILL_H_PCT) / 2.0;

    let pills = vec![Pill {
        rect: RectPct::new(x, y, PILL_W_PCT, PILL_H_PCT),
        color: pill_color,
    }];

    // Five folder icons, all vertically centered.
    // - 1 centered in the middle, fitting within the pill.
    // - 2 on each side, 70% the size of the center icon.
    // - furthest left/right centers are at 0% and 100%.
    const CENTER_Y: f32 = 0.5;

    let center_icon_w = PILL_W_PCT * 0.8;
    let side_icon_w = center_icon_w * 0.7;

    let icons = vec![
        IconPct {
            icon: Icon::Folder,
            color: side_icon_color,
            center_x: 0.0,
            center_y: CENTER_Y,
            width_pct: side_icon_w,
        },
        IconPct {
            icon: Icon::Folder,
            color: side_icon_color,
            center_x: 0.25,
            center_y: CENTER_Y,
            width_pct: side_icon_w,
        },
        IconPct {
            icon: Icon::Folder,
            color: center_icon_color,
            center_x: 0.5,
            center_y: CENTER_Y,
            width_pct: center_icon_w,
        },
        IconPct {
            icon: Icon::Folder,
            color: side_icon_color,
            center_x: 0.75,
            center_y: CENTER_Y,
            width_pct: side_icon_w,
        },
        IconPct {
            icon: Icon::Folder,
            color: side_icon_color,
            center_x: 1.0,
            center_y: CENTER_Y,
            width_pct: side_icon_w,
        },
    ];

    Align::new(
        OnboardingVisual::new(panel_background, pills, false)
            .with_icons(icons)
            .finish(),
    )
    .finish()
}
