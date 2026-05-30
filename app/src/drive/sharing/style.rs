use std::borrow::Cow;

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::color::ColorU;
use warpui::elements::{CornerRadius, Radius};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui::ui_components::text::Span;

/// The padding around ACL items in the sharing dialog.
pub const ACL_ITEM_PADDING: f32 = 16.;

/// The gap between ACL items in the sharing dialog. Because the UI framework doesn't support gaps,
/// items should generally have vertical margins of `ACL_ITEM_GAP / 2`.
pub const ACL_ITEM_GAP: f32 = 10.;

/// The height of ACL items, not including spacing.
pub const ACL_ITEM_HEIGHT: f32 = 32.;

/// The height for guest ACL items, which is slightly larger than [`ACL_ITEM_HEIGHT`] to account
/// for guest details.
pub const ACL_GUEST_HEIGHT: f32 = 36.;

/// The font size for primary text in the dialog, like subject names.
pub const PRIMARY_TEXT_SIZE: f32 = 14.;

/// The font size for header text in the dialog.
pub const HEADER_TEXT_SIZE: f32 = 16.;

/// Background color for the sharing dialog.
pub fn dialog_background(appearance: &Appearance) -> ColorU {
    appearance.theme().surface_1().into_solid()
}

/// Text color for primary ACL information.
pub fn acl_primary_text_color(appearance: &Appearance) -> ColorU {
    internal_colors::text_main(appearance.theme(), dialog_background(appearance))
}

/// Text color for secondary ACL information.
pub fn acl_secondary_text_color(appearance: &Appearance) -> ColorU {
    internal_colors::text_sub(appearance.theme(), dialog_background(appearance))
}

/// Text color for non-interactive labels.
pub fn label_text(appearance: &Appearance) -> ColorU {
    internal_colors::text_disabled(appearance.theme(), dialog_background(appearance))
}

/// Fill to use for borders around form-like text.
pub fn form_border_color(appearance: &Appearance) -> ColorU {
    appearance.theme().surface_3().into_solid()
}

/// Background to use for chip-like form elements.
pub fn form_chip_background(appearance: &Appearance) -> Fill {
    appearance.theme().surface_2()
}

/// Common ACL avatar styles.
pub fn subject_avatar_styles(appearance: &Appearance) -> UiComponentStyles {
    UiComponentStyles {
        width: Some(24.),
        height: Some(24.),
        border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
        font_size: Some(appearance.ui_font_size()),
        font_family_id: Some(appearance.ui_font_family()),
        font_weight: Some(Weight::Bold),
        ..Default::default()
    }
}

/// Create a detail-text span.
pub fn detail_text(text: impl Into<Cow<'static, str>>, appearance: &Appearance) -> Span {
    appearance
        .ui_builder()
        .span(text)
        .with_style(UiComponentStyles {
            font_color: Some(acl_secondary_text_color(appearance)),
            ..Default::default()
        })
}
