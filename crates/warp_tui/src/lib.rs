//! `warp_tui` — the headless TUI front-end for Warp.
//!
//! This crate contains:
//! - [`input`] — the editor-backed TUI input view (`TuiEditorModel` + `TuiInputView`).
//! - [`root_view`] — [`RootTuiView`], the login-gated transcript root view.
//! - [`session`] — [`run`], the binary entry point that boots the headless app
//!   and starts the transcript-capable TUI draw + input driver.
//! - Binary entry points under `src/bin/`.

mod agent_block;
mod agent_block_sections;
mod autoupdate;
pub mod input;
pub mod root_view;
pub mod session;
mod telemetry;
mod tui_builder;
mod ui;

mod conversation_selection;
mod exit_confirmation;
mod input_mode_policy;
mod keybindings;
mod terminal_background;
mod terminal_block;
mod terminal_session_view;
#[cfg(test)]
mod test_fixtures;
mod tool_call_labels;
mod transcript_view;
mod transient_hint;
mod tui_block_list_viewport_source;
mod tui_diff_storage;
mod tui_file_edits_view;
mod usage;
mod warping_indicator;

pub use root_view::RootTuiView;
pub use session::run;
