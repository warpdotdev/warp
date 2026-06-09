//! `warpui_tui` is the concrete TUI rendering stack for Warp's `warpui`
//! framework: the terminal-side analogue of the GUI `warpui` crate. It turns a
//! [`TuiView`]'s render output into terminal output via crossterm, driven by the
//! same `App`/`AppContext`/handle/context/async/action machinery the GUI uses
//! (re-exported from [`warpui_core`] under its `tui` feature).
//!
//! # Foundation (this module set) — FROZEN INTERFACES
//!
//! This crate is built in layers. The foundation defined here — [`geometry`],
//! [`buffer`], the [`TuiElement`](elements::TuiElement) trait, and the
//! [`event`] types — is **frozen**: the concrete elements, presenter, renderer,
//! and runtime are built against these surfaces and must not need to change
//! them. The frozen surfaces are:
//!
//! ## Geometry ([`TuiSize`], [`TuiRect`], [`TuiConstraint`])
//! Integer (`u16`) cell-grid geometry. A [`TuiConstraint`] carries a `min` and
//! `max` [`TuiSize`]; an element's [`layout`](elements::TuiElement::layout)
//! returns a size within that box (use [`TuiConstraint::clamp`]). [`TuiRect`]
//! provides [`inset`](TuiRect::inset) and the
//! [`split_top`](TuiRect::split_top)/[`split_left`](TuiRect::split_left)
//! helpers used for stacking layout.
//!
//! ## Cell buffer ([`TuiBuffer`], [`Cell`], [`TuiStyle`])
//! An in-memory grid of styled [`Cell`]s. Construct with [`TuiBuffer::new`],
//! write with [`set_cell`](TuiBuffer::set_cell)/[`set_str`](TuiBuffer::set_str)
//! (wide- and combining-grapheme aware; out-of-bounds writes are clipped, never
//! panic), read with [`get`](TuiBuffer::get), and assert against it headlessly
//! with [`to_lines`](TuiBuffer::to_lines). Two buffers are equal iff their
//! contents *and* styles match.
//!
//! ## Element trait ([`TuiElement`](elements::TuiElement)) and the render-output bridge
//! [`TuiElement`](elements::TuiElement) is the unit of layout + paint. A
//! [`TuiView`] renders to [`TuiRenderOutput`] (`Box<dyn TuiElement>`); see
//! [`elements`] for how that satisfies the core's abstract, type-erased
//! `RenderOutput`.
//!
//! ## Events ([`event`])
//! TUI event plumbing ([`TuiEventContext`], [`TuiEventDispatchResult`],
//! [`TuiDispatchEventResult`]) plus the frozen signature of
//! [`crossterm_event_to_warp_event`] (implemented by the runtime layer).

mod buffer;
pub mod elements;
mod event;
mod geometry;

pub use buffer::{Cell, TuiBuffer, TuiStyle};
pub use elements::{TuiElement, TuiPresentationContext, TuiRenderOutput};
pub use event::{
    crossterm_event_to_warp_event, TuiDispatchEventResult, TuiEventContext, TuiEventDispatchResult,
};
pub use geometry::{TuiConstraint, TuiRect, TuiSize};
pub use warpui_core::TuiView;
