use warp_core::ui::theme::Fill;

use super::warp_agent_circle_colors;
use crate::themes::default_themes::{dark_theme, light_theme};

#[test]
fn warp_agent_circle_uses_white_glyph_on_black_for_dark_themes() {
    assert_eq!(
        warp_agent_circle_colors(&dark_theme()),
        (Fill::black(), Fill::white())
    );
}

#[test]
fn warp_agent_circle_uses_black_glyph_on_white_for_light_themes() {
    assert_eq!(
        warp_agent_circle_colors(&light_theme()),
        (Fill::white(), Fill::black())
    );
}
