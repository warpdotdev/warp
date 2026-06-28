//! TUI input view.
//!
//! The key types are:
//! - [`view::TuiInputView`] — ratatui-rendered view implementing [`TuiView`], backed
//!   by a [`warp::editor::CodeEditorModel`] in char-cell mode
//! - [`view::TuiInputViewEvent`] — events emitted by the view (e.g. `Submitted`)
//!
//! TUI-specific session state (kill buffer, scroll offset, terminal width) lives on
//! the view, not on a separate model. See `specs/tui-input-view/TECH.md` for details.

pub mod kill_buffer;
pub mod view;

pub use view::{TuiInputView, TuiInputViewEvent};
