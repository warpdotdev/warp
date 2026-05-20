//! Shared helpers for walking the orchestration topology of conversations.
//!
//! The topology is stored as a parent → children index on
//! [`BlocklistAIHistoryModel`]. These helpers are factored out of the
//! orchestration pill bar so other surfaces (e.g. the agent-mode usage
//! footer's credit rollup) can walk the same tree without duplicating the
//! traversal.

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;

/// Returns all locally-known descendants (children, grandchildren, …) of
/// `parent_id`, flattened in pre-order with each parent's child registration
/// order preserved.
///
/// This walks `BlocklistAIHistoryModel::child_conversation_ids_of`
/// transitively. The walker only consults the `children_by_parent` index, so
/// it works even before child `AIConversation`s have been loaded into
/// `conversations_by_id`. Unloaded descendants are still returned by id;
/// callers can filter them out via `history.conversation(&id)` as needed.
pub fn descendant_conversation_ids_in_spawn_order(
    history: &BlocklistAIHistoryModel,
    parent_id: AIConversationId,
) -> Vec<AIConversationId> {
    let mut descendants = Vec::new();
    collect_descendant_conversation_ids_in_spawn_order(history, parent_id, &mut descendants);
    descendants
}

/// Recursive worker for [`descendant_conversation_ids_in_spawn_order`]. Kept
/// separate so it can be invoked from existing call sites that already own a
/// buffer.
pub fn collect_descendant_conversation_ids_in_spawn_order(
    history: &BlocklistAIHistoryModel,
    parent_id: AIConversationId,
    descendants: &mut Vec<AIConversationId>,
) {
    for child_id in history.child_conversation_ids_of(&parent_id) {
        descendants.push(*child_id);
        collect_descendant_conversation_ids_in_spawn_order(history, *child_id, descendants);
    }
}

#[cfg(test)]
#[path = "orchestration_topology_tests.rs"]
mod tests;
