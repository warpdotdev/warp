#![allow(dead_code)]

pub(crate) mod connection;
pub(crate) mod harness_picker;
pub(crate) mod models;
pub(crate) mod openusage;
pub(crate) mod path_search;
pub(crate) mod registry;
pub(crate) mod selectors;
pub(crate) mod session_store;
pub(crate) mod slash_commands;
pub(crate) mod submit;
pub(crate) mod submit_model;
pub(crate) mod tool_calls;
pub(crate) mod telemetry;

pub(crate) fn local_acp_enabled() -> bool {
    true
}
