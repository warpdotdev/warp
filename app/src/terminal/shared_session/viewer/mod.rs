//! The viewer is a client that joins a shared session.
#[cfg(any(target_family = "wasm", test))]
pub(crate) mod browser_initial_child_anchor_router;
mod event_loop;
pub(crate) mod history_model;
mod network;
pub(crate) mod orchestration_viewer_model;
pub(crate) mod terminal_manager;
pub(crate) use terminal_manager::TerminalManager;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
