use warp_core::ui::appearance::Appearance;
use warpui::fonts::Weight;
use warpui::ui_components::components::{Coords, UiComponentStyles};

pub const MODAL_WIDTH: f32 = 460.;
pub const MODAL_HEIGHT: f32 = 300.;
pub const DENIED_MODAL_WIDTH: f32 = 355.;
pub const MODAL_PADDING: f32 = 24.;
pub const MODAL_MARGIN: f32 = 16.;
pub const BUTTON_GAP: f32 = 4.;
const TEXT_FONT_SIZE: f32 = 14.;
const HEADER_FONT_SIZE: f32 = 16.;

pub fn modal_header_styles() -> UiComponentStyles {
    UiComponentStyles {
        padding: Some(Coords {
            top: 0.,
            bottom: 0.,
            left: MODAL_PADDING,
            right: MODAL_PADDING,
        }),
        font_size: Some(HEADER_FONT_SIZE),
        font_weight: Some(Weight::Bold),
        ..Default::default()
    }
}

pub fn modal_body_styles() -> UiComponentStyles {
    UiComponentStyles {
        padding: Some(Coords {
            top: 0.,
            bottom: MODAL_PADDING,
            left: MODAL_PADDING,
            right: MODAL_PADDING,
        }),
        ..Default::default()
    }
}

pub fn button_styles() -> UiComponentStyles {
    UiComponentStyles {
        font_size: Some(TEXT_FONT_SIZE),
        font_weight: Some(Weight::Bold),
        height: Some(40.),
        ..Default::default()
    }
}

pub fn radio_button_styles() -> UiComponentStyles {
    UiComponentStyles {
        padding: Some(Coords {
            top: 16.,
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn subheader_styles(appearance: &Appearance) -> UiComponentStyles {
    UiComponentStyles {
        font_size: Some(TEXT_FONT_SIZE),
        font_color: Some(
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into(),
        ),
        ..Default::default()
    }
}
