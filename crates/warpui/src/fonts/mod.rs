#[cfg(native)]
#[cfg_attr(not(macos), allow(dead_code))]
pub mod font_kit;

#[cfg(test)]
#[path = "text_layout_tests.rs"]
mod text_layout_tests;

#[cfg(all(test, target_os = "macos"))]
pub(crate) use text_layout_tests::collect_line_caret_position_starts;
#[cfg(test)]
pub(crate) use text_layout_tests::{collect_glyph_indices, init_fonts};
pub use warpui_core::fonts::*;
