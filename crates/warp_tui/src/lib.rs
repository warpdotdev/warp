//! `warp_tui` — the headless TUI front-end for Warp.
//!
//! This crate contains:
//! - [`input`] — the editor-backed TUI input view (`TuiEditorModel` + `TuiInputView`).
//! - [`root_view`] — [`RootTuiView`], the root view mounted by the `warp-tui`
//!   binary (a single bordered input box docked at the bottom).
//! - [`session`] — [`run`], the binary entry point that boots the headless app
//!   and starts the TUI draw + input driver.
//! - Conversation-streaming modules (`conversation_model`, `conversation_selection`,
//!   `prompt_stream`) — retained but not yet wired into [`run`]; they back the
//!   upcoming transcript view.
//! - Binary entry points under `src/bin/`.

pub mod input;
pub mod root_view;
pub mod session;
mod ui;

// Retained for the upcoming transcript-view integration. They are not wired into
// `run()` yet (the entry point renders the input box), so they are intentionally
// dead code for now.
#[allow(dead_code)]
mod conversation_model;
#[allow(dead_code)]
mod conversation_selection;
#[allow(dead_code)]
mod prompt_stream;

pub use root_view::RootTuiView;
pub use session::run;
