use warp_core::ui::appearance::Appearance;
use warpui::elements::Icon;
use warpui::Element;

use crate::search::result_renderer::ItemHighlightState;

/// Assumes the path is a file, not a folder
pub fn icon_from_file_path(
    path: &str,
    appearance: &Appearance,
    highlight_state: ItemHighlightState,
) -> Box<dyn Element> {
    let icon = crate::code::icon_from_file_path(path, appearance);
    match icon {
        Some(icon) => icon,
        None => Icon::new(
            "bundled/svg/completion-file.svg",
            highlight_state.icon_fill(appearance).into_solid(),
        )
        .finish(),
    }
}
