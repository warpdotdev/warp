//! TUI input view and model.
//!
//! The key types are:
//! - [`model::TuiInputModel`] — editor-backed model with Emacs/readline keybindings
//! - [`view::TuiInputView`] — ratatui-rendered view implementing [`TuiView`]
//! - [`model::TuiInputModelEvent`] — events emitted by the model
//!
//! See `specs/tui-input-view/TECH.md` for architecture and keybinding details.

pub mod kill_buffer;
pub mod model;
pub mod view;

pub use model::{TuiInputModel, TuiInputModelEvent};
pub use view::TuiInputView;
