//! `warp_tui` — the headless TUI front-end for Warp.
//!
//! This crate contains:
//! - [`input`] — the editor-backed TUI input view (`TuiEditorModel` + `TuiInputView`).
//! - [`root_view`] — [`RootTuiView`], the login-gated transcript root view.
//! - [`session`] — [`run`], the binary entry point that boots the headless app
//!   and starts the transcript-capable TUI draw + input driver.
//! - Binary entry points under `src/bin/`.

mod agent_block;
pub mod input;
pub mod root_view;
pub mod session;
mod ui;

mod conversation_selection;
mod terminal_block;
mod terminal_session_view;
mod transcript_view;
mod tui_block_list_viewport_source;

pub use root_view::RootTuiView;
pub use session::run;
