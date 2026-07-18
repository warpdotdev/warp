//! TUI input view.
//!
//! The key types are:
//! - [`view::TuiInputView`] — ratatui-rendered view implementing [`TuiView`], backed
//!   by a [`warp::editor::CodeEditorModel`] in char-cell mode
//! - [`view::TuiInputViewEvent`] — events emitted by the view (e.g. `Submitted`)
//!
//! TUI-specific prompt policy lives on the view. See
//! `specs/tui-input-view/TECH.md` for details.

pub mod view;

pub use view::{init, TuiInputView, TuiInputViewEvent};
