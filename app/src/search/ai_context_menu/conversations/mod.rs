pub mod data_source;
mod search_item;

use chrono::{DateTime, Utc};

/// Lightweight representation of a conversation for the @conversations context menu.
/// Only carries the fields needed for display and insertion — avoids constructing
/// a full `ConversationNavigationData` for cloud conversations that have no local state.
#[derive(Debug)]
pub struct ConversationContextItem {
    pub title: String,
    pub server_conversation_token: String,
    pub last_updated: DateTime<Utc>,
}
