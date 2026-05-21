use warpui::color::ColorU;

use crate::ui::color::blend::Blend;

use super::Fill;

const PHENOMENON_BACKGROUND: u32 = 0x121212FF;
const PHENOMENON_FOREGROUND: u32 = 0xFAF9F6FF;
const PHENOMENON_ACCENT: u32 = 0x2E5D9EFF;
const PHENOMENON_BLUE: u32 = 0x3780E9FF;
const PHENOMENON_BODY_TEXT: u32 = 0xFAF9F6E5;
const PHENOMENON_LABEL_TEXT: u32 = 0xFAF9F699;
const PHENOMENON_DISABLED_LABEL_TEXT: u32 = 0xFAF9F680;
const PHENOMENON_SUBTLE_BORDER: u32 = 0xFAF9F633;
const PHENOMENON_MODAL_BACKGROUND: u32 = 0x2A2A2AFF;
const PHENOMENON_MODAL_BADGE_BACKGROUND: u32 = 0xBF409D1A;
const PHENOMENON_MODAL_BADGE_TEXT: u32 = 0xBF409DFF;
const PHENOMENON_MODAL_TITLE_TEXT: u32 = 0xFFFFFFFF;
const PHENOMENON_MODAL_FEATURE_TITLE_TEXT: u32 = 0xE6E6E6FF;
const PHENOMENON_MODAL_FEATURE_DESCRIPTION_TEXT: u32 = 0x9B9B9BFF;
const PHENOMENON_MODAL_BUTTON_BACKGROUND: u32 = 0xFFFFFFFF;
const PHENOMENON_MODAL_BUTTON_TEXT: u32 = 0x050505FF;
const PHENOMENON_MODAL_BUTTON_HOVER_OVERLAY: u32 = 0x0505051F;
const PHENOMENON_MODAL_CLOSE_BUTTON_TEXT: u32 = 0xFFFFFFFF;
const PHENOMENON_MODAL_CLOSE_BUTTON_HOVER: u32 = 0x050505BF;

pub struct PhenomenonStyle;

impl PhenomenonStyle {
    pub fn background() -> ColorU {
        ColorU::from_u32(PHENOMENON_BACKGROUND)
    }

    pub fn foreground() -> ColorU {
        ColorU::from_u32(PHENOMENON_FOREGROUND)
    }

    pub fn accent() -> ColorU {
        ColorU::from_u32(PHENOMENON_ACCENT)
    }

    pub fn blue() -> ColorU {
        ColorU::from_u32(PHENOMENON_BLUE)
    }

    pub fn body_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_BODY_TEXT)
    }

    pub fn label_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_LABEL_TEXT)
    }

    pub fn disabled_label_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_DISABLED_LABEL_TEXT)
    }

    pub fn subtle_border() -> ColorU {
        ColorU::from_u32(PHENOMENON_SUBTLE_BORDER)
    }

    pub fn modal_background() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_BACKGROUND)
    }

    pub fn modal_badge_background() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_BADGE_BACKGROUND)
    }

    pub fn modal_badge_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_BADGE_TEXT)
    }

    pub fn modal_title_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_TITLE_TEXT)
    }

    pub fn modal_feature_title_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_FEATURE_TITLE_TEXT)
    }

    pub fn modal_feature_description_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_FEATURE_DESCRIPTION_TEXT)
    }

    pub fn modal_button_background() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_BUTTON_BACKGROUND)
    }

    pub fn modal_button_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_BUTTON_TEXT)
    }

    pub fn modal_button_hover_overlay() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_BUTTON_HOVER_OVERLAY)
    }

    pub fn modal_close_button_text() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_CLOSE_BUTTON_TEXT)
    }

    pub fn modal_close_button_hover() -> ColorU {
        ColorU::from_u32(PHENOMENON_MODAL_CLOSE_BUTTON_HOVER)
    }

    pub fn tinted_surface() -> Fill {
        Fill::Solid(Self::background()).blend(&Fill::Solid(Self::blue()).with_opacity(50))
    }

    pub fn surface_border() -> ColorU {
        Self::blue()
    }

    pub fn primary_button_background(hovered: bool) -> Fill {
        Fill::Solid(if hovered {
            Self::blue()
        } else {
            Self::accent()
        })
    }

    pub fn primary_button_text() -> ColorU {
        Self::foreground()
    }

    pub fn modal_button_background_fill(hovered: bool) -> Fill {
        if hovered {
            Fill::Solid(Self::modal_button_background())
                .blend(&Fill::Solid(Self::modal_button_hover_overlay()))
        } else {
            Fill::Solid(Self::modal_button_background())
        }
    }

    pub fn segmented_control_background() -> Fill {
        Fill::Solid(Self::foreground()).with_opacity(8)
    }

    pub fn selected_chip_background() -> Fill {
        Fill::Solid(Self::foreground())
    }

    pub fn selected_chip_text() -> ColorU {
        Self::background()
    }

    pub fn selected_chip_border() -> Fill {
        Fill::Solid(Self::accent())
    }

    pub fn unselected_chip_background() -> Fill {
        Fill::Solid(Self::foreground()).with_opacity(8)
    }

    pub fn unselected_chip_text() -> ColorU {
        Self::body_text()
    }
}
