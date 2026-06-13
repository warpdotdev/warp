//! The TUI element library, additive behind the `tui` feature.
//!
//! Spike stub: only the [`TuiElement`] trait object is defined here so the
//! TUI view layer has a render output type. The full element library (layout,
//! buffers, concrete elements) is ported from the legacy `warpui_tui` crate in
//! the M8 implementation task.

/// A renderable TUI element: what a [`TuiView`](crate::TuiView) renders to.
pub trait TuiElement {
    /// A short name for debugging.
    fn name(&self) -> &'static str {
        "TuiElement"
    }
}

/// An empty element, useful as a placeholder render output.
#[derive(Default)]
pub struct TuiEmpty;

impl TuiElement for TuiEmpty {
    fn name(&self) -> &'static str {
        "TuiEmpty"
    }
}
