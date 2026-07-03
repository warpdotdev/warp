//! This module should houses all horizontal/cross-cutting AI functionality throughout
//! Warp (including Agent Mode).
//!
pub(crate) mod active_agent_views_model;
pub(crate) mod agent;
pub(crate) mod agent_tips;
pub(crate) mod ai_document_view;
pub(crate) mod ambient_agents;
pub(crate) mod api_errors;
pub mod artifacts;
pub(crate) mod ask;
pub mod aws_credentials;
pub(crate) mod block_context;
pub(crate) mod blocklist;
pub mod control_code_parser;
pub(crate) mod conversation_navigation;
pub(crate) mod conversation_status_ui;
pub(crate) mod conversation_utils;
pub(crate) mod document;
pub(crate) mod get_relevant_files;
pub(crate) mod llms;
pub(crate) mod persisted_workspace;
pub(crate) mod predict;
pub(crate) mod restored_conversations;
pub(crate) mod skills;
pub use agent_tips::*;
use warpui::AppContext;
pub mod agent_sdk;
pub mod execution_context;
pub mod execution_profiles;
pub mod facts;
pub(crate) mod loading;
pub mod mcp;
pub mod outline;

pub(crate) use ai::paths;
pub(crate) use ask::AskAIType;

pub fn init(app: &mut AppContext) {
    blocklist::keyboard_navigable_buttons::init(app);
    blocklist::block::number_shortcut_buttons::init(app);
    blocklist::toggleable_items::init(app);
    ai_document_view::init(app);
}
