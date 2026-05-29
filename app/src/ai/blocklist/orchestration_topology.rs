//! Shared helpers for walking the orchestration topology of conversations.
//!
//! The topology is stored as a parent → children index on
//! [`BlocklistAIHistoryModel`]. These helpers are factored out of the
//! orchestration pill bar so other surfaces (e.g. the agent-mode usage
//! footer's credit rollup) can walk the same tree without duplicating the
//! traversal.

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrchestrationNavigationDirection {
    Previous,
    Next,
}

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

/// Returns the adjacent conversation in the active orchestration tree,
/// cycling across the orchestrator and all descendants.
///
/// Traversal order is:
///   [orchestrator, descendants in pre-order]
/// where descendants are in the same pre-order used by the orchestration pill
/// bar. Navigation wraps within this full list.
pub fn adjacent_orchestration_child_conversation_id(
    history: &BlocklistAIHistoryModel,
    active_conversation_id: AIConversationId,
    direction: OrchestrationNavigationDirection,
) -> Option<AIConversationId> {
    let active_conversation = history.conversation(&active_conversation_id)?;
    let orchestration_root_id = history
        .resolved_parent_conversation_id_for_conversation(active_conversation)
        .unwrap_or(active_conversation_id);
    let descendant_ids = descendant_conversation_ids_in_spawn_order(history, orchestration_root_id);
    if descendant_ids.is_empty() {
        return None;
    }
    let conversation_ids = std::iter::once(orchestration_root_id)
        .chain(descendant_ids)
        .collect::<Vec<_>>();

    let Some(active_index) = conversation_ids
        .iter()
        .position(|child_id| *child_id == active_conversation_id)
    else {
        return None;
    };


    let target_index = match direction {
        OrchestrationNavigationDirection::Previous => active_index
            .checked_sub(1)
            .unwrap_or(conversation_ids.len() - 1),
        OrchestrationNavigationDirection::Next => (active_index + 1) % conversation_ids.len(),
    };
    conversation_ids.get(target_index).copied()
}

/// Returns a `ConversationStatus` that summarises the orchestrator's state
/// across the whole orchestration tree (orchestrator + all known descendants).
///
/// The orchestrator's own [`ConversationStatus`] only reflects its last
/// exchange's outcome — it flips to `Success` as soon as its own streaming
/// turn finishes, even though child agents may still be running. This helper
/// fixes that mismatch so surfaces like the orchestration pill bar can show a
/// status that matches what the user expects to see while children are still
/// in flight.
///
/// Aggregation precedence (highest wins):
///   1. `InProgress` — any node in the tree is actively running.
///   2. `Blocked` — at least one node is waiting on user input. The
///      `blocked_action` from the first blocked node encountered is preserved
///      so callers can display it.
///   3. `Error` — at least one node finished with an error.
///   4. `Cancelled` — at least one node was cancelled.
///   5. `Success` — everything finished successfully.
///
/// Returns `Success` if the orchestrator is not loaded and has no descendants.
pub fn aggregated_orchestrator_status(
    history: &BlocklistAIHistoryModel,
    orchestrator_id: AIConversationId,
) -> ConversationStatus {
    let statuses = std::iter::once(orchestrator_id)
        .chain(descendant_conversation_ids_in_spawn_order(
            history,
            orchestrator_id,
        ))
        .filter_map(|id| history.conversation(&id).map(|c| c.status().clone()));

    let mut first_blocked: Option<ConversationStatus> = None;
    let mut any_in_progress = false;
    let mut any_error = false;
    let mut any_cancelled = false;
    for status in statuses {
        match status {
            ConversationStatus::InProgress => any_in_progress = true,
            ConversationStatus::Blocked { .. } => {
                if first_blocked.is_none() {
                    first_blocked = Some(status);
                }
            }
            ConversationStatus::Error => any_error = true,
            ConversationStatus::Cancelled => any_cancelled = true,
            ConversationStatus::Success => {}
        }
    }

    if any_in_progress {
        return ConversationStatus::InProgress;
    }
    if let Some(blocked) = first_blocked {
        return blocked;
    }
    if any_error {
        return ConversationStatus::Error;
    }
    if any_cancelled {
        return ConversationStatus::Cancelled;
    }
    ConversationStatus::Success
}

#[cfg(test)]
#[path = "orchestration_topology_tests.rs"]
mod tests;
