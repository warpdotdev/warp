use warp_core::ui::theme::Fill;

use super::{OZ_AMBIENT_BACKGROUND_COLOR, warp_agent_circle_colors};
use crate::themes::default_themes::{dark_theme, light_theme};

#[test]
fn local_warp_agent_circle_uses_white_glyph_on_black_for_dark_themes() {
    assert_eq!(
        warp_agent_circle_colors(&dark_theme(), false),
        (Fill::black(), Fill::white())
    );
}

#[test]
fn local_warp_agent_circle_uses_black_glyph_on_white_for_light_themes() {
    assert_eq!(
        warp_agent_circle_colors(&light_theme(), false),
        (Fill::white(), Fill::black())
    );
}

#[test]
fn ambient_warp_agent_circle_keeps_purple_background_in_all_themes() {
    let expected = (Fill::Solid(OZ_AMBIENT_BACKGROUND_COLOR), Fill::black());

    assert_eq!(warp_agent_circle_colors(&dark_theme(), true), expected);
    assert_eq!(warp_agent_circle_colors(&light_theme(), true), expected);
}
