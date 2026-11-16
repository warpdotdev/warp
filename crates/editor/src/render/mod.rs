//! Rich Text Editor rendering layer - model and UI element for rendering
//! marked-up rich text.

pub mod element;
pub mod layout;
pub mod model;

/// The size for icon buttons within the rich-text editor. This is needed for both layout and
/// painting, so it's defined here.
pub const ICON_BUTTON_SIZE: f32 = 24.;
pub const BLOCK_FOOTER_HEIGHT: f32 = 42.;
pub(crate) const TABLE_LINE_HEIGHT_RATIO: f32 = 1.5;
pub(crate) const TABLE_BASELINE_RATIO: f32 = 0.8;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
