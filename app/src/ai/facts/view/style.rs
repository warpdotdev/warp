use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{CornerRadius, Radius};
use warpui::fonts::Weight;
use warpui::ui_components::components::{Coords, UiComponentStyles};

pub const HEADER_FONT_SIZE: f32 = 16.;
pub const TEXT_FONT_SIZE: f32 = 14.;
pub const SUBTEXT_FONT_SIZE: f32 = 12.;
pub const ICON_SIZE: f32 = 16.;

pub const BANNER_ICON_SIZE: f32 = 14.;
pub const BANNER_PADDING: f32 = 12.;

pub const ICON_MARGIN: f32 = 8.;
pub const ROW_ICON_MARGIN: f32 = 4.;

pub const RULE_VERTICAL_PADDING: f32 = 12.;
pub const ROW_HORIZONTAL_PADDING: f32 = 12.;
pub const ITEM_BOTTOM_MARGIN: f32 = 12.;

pub const EDITOR_HORIZONTAL_PADDING: f32 = 16.;
pub const EDITOR_VERTICAL_PADDING: f32 = 10.;
pub const EDITOR_MIN_HEIGHT: f32 = 240.;
pub const EDITOR_MAX_HEIGHT: f32 = 320.;

pub const SECTION_MARGIN: f32 = 16.;
pub const PANE_PADDING: f32 = 16.;
pub const PANE_WIDTH: f32 = 800.;
pub const ZERO_STATE_HEIGHT: f32 = 643.;

pub fn search_bar(appearance: &Appearance) -> UiComponentStyles {
    UiComponentStyles {
        background: Some(internal_colors::neutral_2(appearance.theme()).into()),
        border_color: Some(internal_colors::neutral_4(appearance.theme()).into()),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
        padding: Some(Coords {
            top: 8.,
            bottom: 8.,
            left: 12.,
            right: 12.,
        }),
        ..Default::default()
    }
}

pub fn header_text() -> UiComponentStyles {
    UiComponentStyles {
        font_size: Some(HEADER_FONT_SIZE),
        font_weight: Some(Weight::Bold),
        ..Default::default()
    }
}

pub fn description_text(appearance: &Appearance) -> UiComponentStyles {
    UiComponentStyles {
        font_size: Some(TEXT_FONT_SIZE),
        font_color: Some(
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into(),
        ),
        margin: Some(Coords {
            bottom: 8.,
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn fact_row_subtext(appearance: &Appearance) -> UiComponentStyles {
    UiComponentStyles {
        font_size: Some(SUBTEXT_FONT_SIZE),
        font_color: Some(
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into(),
        ),
        ..Default::default()
    }
}

pub fn fact_row_text(appearance: &Appearance) -> UiComponentStyles {
    UiComponentStyles {
        font_size: Some(TEXT_FONT_SIZE),
        font_color: Some(
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
                .into(),
        ),
        ..Default::default()
    }
}

pub fn fact_project_based_row_text(appearance: &Appearance) -> UiComponentStyles {
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
