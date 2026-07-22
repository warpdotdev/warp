//! Shared helpers for walking the orchestration topology of conversations.
//!
//! The topology is stored as a parent → children index on
//! [`BlocklistAIHistoryModel`]. These helpers are factored out of the
//! orchestration pill bar so other surfaces (e.g. keyboard navigation and
//! the agent-mode usage footer's credit rollup) can walk and order the same
//! tree without duplicating the logic.
#[cfg(feature = "tui")]
use std::collections::HashSet;

use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrchestrationNavigationDirection {
    Previous,
    Next,
}

/// Semantic role of a participant in an orchestration transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrchestrationParticipantKind {
    Orchestrator,
    Agent { name: String },
    Unknown,
}

impl OrchestrationParticipantKind {
    pub(super) fn display_name(&self) -> &str {
        match self {
            Self::Orchestrator => "Orchestrator",
            Self::Agent { name } => name,
            Self::Unknown => "Unknown agent",
        }
    }
}

/// Frontend-independent identity for an orchestration participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOrchestrationParticipant {
    pub kind: OrchestrationParticipantKind,
    pub conversation_id: Option<AIConversationId>,
}

impl ResolvedOrchestrationParticipant {
    fn orchestrator(conversation_id: Option<AIConversationId>) -> Self {
        Self {
            kind: OrchestrationParticipantKind::Orchestrator,
            conversation_id,
        }
    }

    fn unknown() -> Self {
        Self {
            kind: OrchestrationParticipantKind::Unknown,
            conversation_id: None,
        }
    }
}

/// Returns the agent ID of the conversation that orchestrates `conversation`.
pub fn orchestrator_agent_id_for_conversation(
    history: &BlocklistAIHistoryModel,
    conversation: &AIConversation,
) -> Option<String> {
    match history.resolved_parent_conversation_id_for_conversation(conversation) {
        Some(parent_id) => history
            .conversation(&parent_id)
            .and_then(AIConversation::orchestration_agent_id)
            .or_else(|| conversation.parent_agent_id().map(str::to_owned)),
        None => conversation
            .parent_agent_id()
            .map(str::to_owned)
            .or_else(|| conversation.orchestration_agent_id()),
    }
}

/// Resolves a server-side agent ID to frontend-independent participant data.
pub fn resolve_orchestration_participant(
    history: &BlocklistAIHistoryModel,
    agent_id: &str,
    orchestrator_agent_id: Option<&str>,
) -> ResolvedOrchestrationParticipant {
    let conversation_id = history.conversation_id_for_agent_id(agent_id);
    if orchestrator_agent_id == Some(agent_id) {
        return ResolvedOrchestrationParticipant::orchestrator(conversation_id);
    }
    let Some(conversation_id) = conversation_id else {
        return ResolvedOrchestrationParticipant::unknown();
    };
    let Some(conversation) = history.conversation(&conversation_id) else {
        return ResolvedOrchestrationParticipant::unknown();
    };
    let name = conversation
        .agent_name()
        .filter(|name| !name.is_empty())
        .unwrap_or("Agent")
        .to_string();
    ResolvedOrchestrationParticipant {
        kind: OrchestrationParticipantKind::Agent { name },
        conversation_id: Some(conversation_id),
    }
}

/// Returns the topmost loaded conversation in an orchestration tree.
///
/// Conversations without descendants are not orchestration roots. Malformed
/// parent cycles and missing ancestors fail closed.
#[cfg(feature = "tui")]
pub fn orchestration_root_conversation_id(
    history: &BlocklistAIHistoryModel,
    conversation_id: AIConversationId,
) -> Option<AIConversationId> {
    history.conversation(&conversation_id)?;
    let mut current = conversation_id;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let conversation = history.conversation(&current)?;
        let Some(parent) = history.resolved_parent_conversation_id_for_conversation(conversation)
        else {
            return (!history.child_conversation_ids_of(&current).is_empty()).then_some(current);
        };
        current = parent;
    }
    None
}

const DONE_STATUS_KEY: u8 = 3;

fn pill_status_sort_key(status: Option<&ConversationStatus>) -> u8 {
    match status {
        Some(ConversationStatus::Blocked { .. }) => 0,
        Some(ConversationStatus::Error) => 1,
        // A recovering conversation sorts with the actively-running ones.
        Some(ConversationStatus::InProgress)
        | Some(ConversationStatus::TransientError)
        | Some(ConversationStatus::WaitingForEvents) => 2,
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

/// Returns `true` if `orchestrator_id` has at least one **active local** child
/// agent in its orchestration subtree.
///
/// "Local" means the child runs in this client (`!is_remote_child()`); a
/// remote/cloud child is owned by its worker and is unaffected by handing the
/// parent off to the cloud. "Active" means the child is not in a terminal state
/// (`!ConversationStatus::is_done()`), so a session whose children have all
/// finished is not treated as still orchestrating.
///
/// This is used to keep automatic cloud handoff from forking only the
/// orchestrator to the cloud and orphaning its still-running local children.
/// Descendants whose `AIConversation` is not loaded are ignored — active local
/// children are always loaded in this client.
pub fn has_local_orchestrated_children(
    history: &BlocklistAIHistoryModel,
    orchestrator_id: AIConversationId,
) -> bool {
    descendant_conversation_ids_in_spawn_order(history, orchestrator_id)
        .iter()
        .any(|id| {
            history.conversation(id).is_some_and(|conversation| {
                !conversation.is_remote_child() && !conversation.status().is_done()
            })
        })
}

/// One descendant in canonical orchestration pill order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderedOrchestrationDescendant {
    pub conversation_id: AIConversationId,
    pub spawn_index: usize,
}

/// Returns descendants in the canonical orchestration pill order:
///   1) pinned children
///   2) unpinned children
/// each bucket ordered by status priority, then done-recency, then spawn order.
///
/// This is the single ordering source used by both the pill bar and keyboard
/// navigation. Callers should preserve the returned order rather than sorting
/// the conversations again.
pub fn descendant_conversations_in_pill_order(
    history: &BlocklistAIHistoryModel,
    parent_id: AIConversationId,
) -> Vec<OrderedOrchestrationDescendant> {
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
        .map(
            |(_, _, _, spawn_index, conversation_id)| OrderedOrchestrationDescendant {
                conversation_id,
                spawn_index,
            },
        )
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
    let descendants = descendant_conversations_in_pill_order(history, orchestration_root_id);
    if descendants.is_empty() {
        return None;
    }
    let conversation_ids = std::iter::once(orchestration_root_id)
        .chain(
            descendants
                .into_iter()
                .map(|descendant| descendant.conversation_id),
        )
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
            // A recovering node counts as still running for aggregation purposes.
            ConversationStatus::InProgress | ConversationStatus::TransientError => {
                any_in_progress = true
            }
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
