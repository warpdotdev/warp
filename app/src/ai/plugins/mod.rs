//! Client-side ownership of Agent Plugin packages.
//!
//! The portable parsing, validation, and precedence rules live in `ai::plugins`. This module is
//! the part that has to know about the running client: which repositories are in scope, what the
//! `Agent Plugin discovery` preference says, and what the rest of the app must do when that
//! preference changes.
pub(crate) mod plugin_manager;
pub(crate) mod registry;

pub use plugin_manager::PluginManager;
