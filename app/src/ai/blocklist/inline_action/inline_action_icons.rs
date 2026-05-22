use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::{AppContext, SingletonEntity};

use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// Returns the size for icons in the AI block, scaled to the user's current font size.
pub fn icon_size(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);
    app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    )
}

pub fn green_check_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Check.into(),
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub fn red_x_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::X.into(),
        AnsiColorIdentifier::Red.to_ansi_color(&appearance.theme().terminal_colors().normal),
    )
}

pub fn cancelled_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::Cancelled.into(),
        blended_colors::neutral_6(appearance.theme()),
    )
}

pub fn reverted_icon(appearance: &Appearance) -> warpui::elements::Icon {
    warpui::elements::Icon::new(
        Icon::ReverseLeft.into(),
        blended_colors::neutral_6(appearance.theme()),
    )
}
