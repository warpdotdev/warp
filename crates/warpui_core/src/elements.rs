//! The element library. GUI elements are always compiled; the `tui` feature
//! additively adds the TUI element module alongside them.
mod gui;
pub use gui::*;

pub mod animation;
pub mod shimmer_math;

#[cfg(feature = "tui")]
pub mod tui;
