//! The element library. GUI elements are always compiled; the `tui` feature
//! additively adds the TUI element module alongside them.
mod gui;
pub use gui::*;

#[cfg(feature = "tui")]
pub mod tui;
