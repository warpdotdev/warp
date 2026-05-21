use pathfinder_color::ColorU;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{CornerRadius, MouseState, Radius};
use warpui::Element;

use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// Shared item highlight state for left-panel style lists (file tree, global search results,
/// warp drive rows, etc.).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum ItemHighlightState {
    None,
    Selected,
    Hovered,
}

impl ItemHighlightState {
    pub fn new(is_selected: bool, mouse_state: &MouseState) -> Self {
        if is_selected {
            ItemHighlightState::Selected
        } else if mouse_state.is_hovered() {
            ItemHighlightState::Hovered
        } else {
            ItemHighlightState::None
        }
    }

    pub fn text_and_icon_color(&self, appearance: &Appearance) -> ColorU {
        match self {
            ItemHighlightState::None => {
                blended_colors::text_sub(appearance.theme(), appearance.theme().background())
            }
            ItemHighlightState::Selected => appearance.theme().foreground().into(),
            ItemHighlightState::Hovered => {
                blended_colors::text_main(appearance.theme(), appearance.theme().background())
            }
        }
    }

    pub fn background_color(&self, appearance: &Appearance) -> Option<Fill> {
        match self {
            ItemHighlightState::None => None,
            ItemHighlightState::Selected => Some(internal_colors::fg_overlay_4(appearance.theme())),
            ItemHighlightState::Hovered => Some(internal_colors::fg_overlay_2(appearance.theme())),
        }
    }

    pub fn corner_radius(&self) -> Option<CornerRadius> {
        match self {
            ItemHighlightState::None => None,
            ItemHighlightState::Selected | ItemHighlightState::Hovered => {
                Some(CornerRadius::with_all(Radius::Pixels(4.)))
            }
        }
    }
}

pub(crate) enum ImageOrIcon {
    Icon(Icon),
    Image(Box<dyn Element>),
}
