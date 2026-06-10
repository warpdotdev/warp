//! Shared helpers for walking the orchestration topology of conversations.
//!
//! The topology is stored as a parent → children index on
//! [`BlocklistAIHistoryModel`]. These helpers are factored out of the
//! orchestration pill bar so other surfaces (e.g. keyboard navigation and
//! the agent-mode usage footer's credit rollup) can walk and order the same
//! tree without duplicating the logic.

use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrchestrationNavigationDirection {
    Previous,
    Next,
}

const DONE_STATUS_KEY: u8 = 3;

fn pill_status_sort_key(status: Option<&ConversationStatus>) -> u8 {
    match status {
        Some(ConversationStatus::Blocked { .. }) => 0,
        Some(ConversationStatus::Error) => 1,
        Some(ConversationStatus::InProgress) | Some(ConversationStatus::WaitingForEvents) => 2,
        Some(ConversationStatus::Cancelled) | Some(ConversationStatus::Success) => DONE_STATUS_KEY,
        None => 2,
    }
}

fn pill_secondary_sort_key(status_key: u8, last_modified_ms: Option<i64>) -> i64 {
    if status_key == DONE_STATUS_KEY {
        last_modified_ms.unwrap_or(0).saturating_neg()
    } else {
        0
    }
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

/// Returns descendants in the canonical orchestration pill order:
///   1) pinned children
///   2) unpinned children
/// each bucket ordered by status priority, then done-recency, then spawn order.
///
/// This is the single ordering source used by both the pill bar and keyboard
/// navigation. Callers should preserve the returned order rather than sorting
/// the conversations again.
pub fn descendant_conversation_ids_in_pill_order(
    history: &BlocklistAIHistoryModel,
    parent_id: AIConversationId,
) -> Vec<AIConversationId> {
    let mut descendants = descendant_conversation_ids_in_spawn_order(history, parent_id)
        .into_iter()
        .enumerate()
        .filter_map(|(spawn_index, conversation_id)| {
            history.conversation(&conversation_id).map(|conversation| {
                let status_key = pill_status_sort_key(Some(conversation.status()));
                let secondary_key = pill_secondary_sort_key(
                    status_key,
                    conversation
                        .last_modified_at()
                        .map(|time| time.timestamp_millis()),
                );
                (
                    !conversation.is_pinned(),
                    status_key,
                    secondary_key,
                    spawn_index,
                    conversation_id,
                )
            })
        })
        .collect::<Vec<_>>();

    descendants.sort_by_key(
        |(is_unpinned, status_key, secondary_key, spawn_index, _conversation_id)| {
            (*is_unpinned, *status_key, *secondary_key, *spawn_index)
        },
    );
    descendants
        .into_iter()
        .map(|(_, _, _, _, conversation_id)| conversation_id)
        .collect()
}

/// Returns the adjacent conversation in the active orchestration tree,
/// cycling across the orchestrator and all descendants.
///
/// Traversal order is:
///   [orchestrator, descendants in pill-bar order]
/// where descendants use the same pinned/status/recency ordering rendered by
/// the orchestration pill bar. Navigation wraps within this full list.
pub fn adjacent_orchestration_child_conversation_id(
    history: &BlocklistAIHistoryModel,
    active_conversation_id: AIConversationId,
    direction: OrchestrationNavigationDirection,
) -> Option<AIConversationId> {
    let active_conversation = history.conversation(&active_conversation_id)?;
    let orchestration_root_id = history
        .resolved_parent_conversation_id_for_conversation(active_conversation)
        .unwrap_or(active_conversation_id);
    let descendant_ids = descendant_conversation_ids_in_pill_order(history, orchestration_root_id);
    if descendant_ids.is_empty() {
        return None;
    }
    let conversation_ids = std::iter::once(orchestration_root_id)
        .chain(descendant_ids)
        .collect::<Vec<_>>();

    let active_index = conversation_ids
        .iter()
        .position(|child_id| *child_id == active_conversation_id)?;

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
///   1. `InProgress` — any node in the tree is actively running, **unless**
///      the orchestrator itself yielded into `WaitingForEvents`. The parent's
///      waiting state is a more specific and useful signal to the user than
///      "something somewhere is running".
///   2. `Blocked` — at least one node is waiting on user input. The
///      `blocked_action` from the first blocked node encountered is preserved
///      so callers can display it.
///   3. `WaitingForEvents` — at least one node yielded via `wait_for_events`
///      and is listening for inbound input. The run is quiescent but not
///      terminal — the driver stays alive until something resumes it.
///      Carve-out: when the orchestrator itself is `Cancelled` or `Error`,
///      the parent's terminal status wins over a descendant `WaitingForEvents`
///      so the pill does not falsely advertise a resumable run.
///   4. `Error` — at least one node finished with an error.
///   5. `Cancelled` — at least one node was cancelled.
///   6. `Success` — everything finished successfully.
///
/// Returns `Success` if the orchestrator is not loaded and has no descendants.
pub fn aggregated_orchestrator_status(
    history: &BlocklistAIHistoryModel,
    orchestrator_id: AIConversationId,
) -> ConversationStatus {
    let mut orchestrator_status: Option<ConversationStatus> = None;
    let mut first_blocked: Option<ConversationStatus> = None;
    let mut any_in_progress = false;
    let mut any_waiting = false;
    let mut any_error = false;
    let mut any_cancelled = false;

    for id in std::iter::once(orchestrator_id).chain(descendant_conversation_ids_in_spawn_order(
        history,
        orchestrator_id,
    )) {
        let Some(status) = history.conversation(&id).map(|c| c.status().clone()) else {
            continue;
        };
        if id == orchestrator_id {
            orchestrator_status = Some(status.clone());
        }
        match status {
            ConversationStatus::InProgress => any_in_progress = true,
            ConversationStatus::WaitingForEvents => any_waiting = true,
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
        // Parent's own waiting state outranks descendant in-progress so
        // the pill reflects that THIS conversation is paused.
        if matches!(
            orchestrator_status,
            Some(ConversationStatus::WaitingForEvents)
        ) {
            return ConversationStatus::WaitingForEvents;
        }
        return ConversationStatus::InProgress;
    }
    if let Some(blocked) = first_blocked {
        return blocked;
    }
    if any_waiting {
        // Parent's terminal status beats descendant waiting — a
        // finalized run can't resume, so surface the parent's outcome.
        match orchestrator_status {
            Some(ConversationStatus::Cancelled) => return ConversationStatus::Cancelled,
            Some(ConversationStatus::Error) => return ConversationStatus::Error,
            _ => return ConversationStatus::WaitingForEvents,
        }
    }
    if any_error {
        return ConversationStatus::Error;
    }
    if any_cancelled {
        return ConversationStatus::Cancelled;
    }
    ConversationStatus::Success
}

/// Returns a conversation's direct status, or the aggregated subtree status
/// ([`aggregated_orchestrator_status`]) when it's a known orchestration parent.
///
/// Used by top-level chrome (tab/header icons, status rows) so the badge keeps
/// reflecting active children after the orchestrator's own turn finishes.
pub fn orchestration_aware_conversation_status(
    history: &BlocklistAIHistoryModel,
    conversation: &AIConversation,
) -> ConversationStatus {
    if history
        .child_conversation_ids_of(&conversation.id())
        .is_empty()
    {
        conversation.status().clone()
    } else {
        aggregated_orchestrator_status(history, conversation.id())
    }
}

#[cfg(test)]
#[path = "orchestration_topology_tests.rs"]
mod tests;
