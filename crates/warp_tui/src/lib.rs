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
mod alt_screen_view;
mod autoupdate;
mod clipboard;
pub mod input;
pub mod root_view;
pub mod session;
mod telemetry;
mod tui_ask_question_view;
mod tui_builder;
mod ui;

mod conversation_menu;
mod conversation_selection;
mod editor_element;
mod editor_interaction;
mod editor_view;
mod exit_confirmation;
mod inline_menu;
mod input_mode_policy;
mod input_suggestions_mode;
mod keybindings;
mod mcp_menu;
mod model_menu;
mod option_selector;
mod orchestrated_agent_identity_styling;
mod orchestration_block;
mod orchestration_model;
mod resume;
mod session_registry;
mod skills_menu;
mod slash_commands;
mod terminal_background;
mod terminal_block;
mod terminal_content_element;
mod terminal_session_view;
mod terminal_use;
#[cfg(test)]
mod test_fixtures;
mod tool_call_labels;
mod transcript_view;
mod transient_hint;
mod tui_block_list_viewport_source;
mod tui_cli_subagent_view;
mod tui_code_block_view;
mod tui_column_layout;
mod tui_diff_storage;
mod tui_file_edits_view;
mod tui_markdown;
mod tui_plan_view;
mod tui_shell_command_view;
mod usage;
mod warping_indicator;
mod zero_state;

pub use root_view::RootTuiView;
pub use session::run;
