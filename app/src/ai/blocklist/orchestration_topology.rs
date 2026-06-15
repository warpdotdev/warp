//! Shared helpers for walking the orchestration topology of conversations.
//!
//! The topology is stored as a parent → children index on
//! [`BlocklistAIHistoryModel`]. These helpers are factored out of the
//! orchestration pill bar so other surfaces (e.g. keyboard navigation and
//! the agent-mode usage footer's credit rollup) can walk and order the same
//! tree without duplicating the logic.

use std::collections::{HashMap, HashSet};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId};
use crate::ai::artifacts::{merge_artifacts, Artifact};
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
    let mut visited = HashSet::from([parent_id]);
    collect_descendants_guarded(history, parent_id, &mut visited, &mut descendants);
    descendants
}

/// Recursive worker for [`descendant_conversation_ids_in_spawn_order`].
/// `visited` guards against cycles and diamonds in the parent/child index so a
/// corrupted topology can't cause infinite recursion or duplicate entries.
fn collect_descendants_guarded(
    history: &BlocklistAIHistoryModel,
    parent_id: AIConversationId,
    visited: &mut HashSet<AIConversationId>,
    descendants: &mut Vec<AIConversationId>,
) {
    for child_id in history.child_conversation_ids_of(&parent_id) {
        if !visited.insert(*child_id) {
            continue;
        }
        descendants.push(*child_id);
        collect_descendants_guarded(history, *child_id, visited, descendants);
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

/// Root of an orchestration subtree for [`aggregated_subtree_artifacts`]: either
/// a local conversation or an ambient run record. Scoped to this helper so the
/// low-level topology utilities don't depend on view-model identity types.
pub(crate) enum SubtreeRoot {
    Conversation(AIConversationId),
    Task(AmbientAgentTaskId),
}

/// Returns the subtree's artifacts in pre-order, deduped by identity (first
/// occurrence wins). Reads only in-memory state; never fetches.
///
/// Walks both the run-record child links ([`AmbientAgentTask::children`]) and
/// the conversation parent → children index, since either side may be missing
/// nodes (remote children often lack a local conversation; local children may
/// have no loaded run record). Each node contributes its conversation artifacts
/// (or cached metadata when unloaded) plus its run record's artifacts — the
/// only local source for a remote child's artifacts. A `Task` root absent from
/// `tasks` contributes nothing.
pub(crate) fn aggregated_subtree_artifacts<'a>(
    history: &'a BlocklistAIHistoryModel,
    tasks: &'a HashMap<AmbientAgentTaskId, AmbientAgentTask>,
    root: SubtreeRoot,
) -> Vec<Artifact> {
    let mut walker = SubtreeArtifactWalker {
        history,
        tasks,
        visited_runs: HashSet::new(),
        visited_conversations: HashSet::new(),
        artifact_lists: Vec::new(),
    };
    match root {
        SubtreeRoot::Task(task_id) => walker.visit(tasks.get(&task_id), None),
        SubtreeRoot::Conversation(conversation_id) => walker.visit(None, Some(conversation_id)),
    }
    merge_artifacts(walker.artifact_lists)
}

/// Pre-order DFS state for [`aggregated_subtree_artifacts`]. Each visit
/// handles one agent node — the pairing of a run record and its linked local
/// conversation — and the per-identity visited sets both guard against
/// cycles and stop a node reached through one tree from contributing again
/// when reached through the other.
struct SubtreeArtifactWalker<'a> {
    history: &'a BlocklistAIHistoryModel,
    tasks: &'a HashMap<AmbientAgentTaskId, AmbientAgentTask>,
    visited_runs: HashSet<AmbientAgentTaskId>,
    visited_conversations: HashSet<AIConversationId>,
    artifact_lists: Vec<&'a [Artifact]>,
}

impl<'a> SubtreeArtifactWalker<'a> {
    /// Visits one node given whichever of its identities the caller knows,
    /// resolving the other half through the run ↔ conversation links.
    fn visit(
        &mut self,
        task: Option<&'a AmbientAgentTask>,
        conversation_id: Option<AIConversationId>,
    ) {
        let history = self.history;
        let tasks = self.tasks;
        let conversation_id = conversation_id
            .or_else(|| task.and_then(|task| local_conversation_id_for_task(task, history)));
        let conversation = conversation_id.and_then(|id| history.conversation(&id));
        let task = task.or_else(|| {
            conversation
                .and_then(|conversation| conversation.task_id())
                .and_then(|task_id| tasks.get(&task_id))
        });

        // `filter` doubles as the visited guard: an identity already seen
        // contributes nothing and is not traversed again.
        let new_conversation_id =
            conversation_id.filter(|id| self.visited_conversations.insert(*id));
        let new_task = task.filter(|task| self.visited_runs.insert(task.task_id));

        if let Some(conversation_id) = new_conversation_id {
            let artifacts = conversation
                .map(|conversation| conversation.artifacts())
                .or_else(|| {
                    history
                        .get_conversation_metadata(&conversation_id)
                        .map(|metadata| metadata.artifacts.as_slice())
                })
                .unwrap_or_default();
            self.artifact_lists.push(artifacts);
        }
        if let Some(task) = new_task {
            self.artifact_lists.push(&task.artifacts);
        }

        if let Some(conversation_id) = new_conversation_id {
            for child_id in history.child_conversation_ids_of(&conversation_id) {
                self.visit(None, Some(*child_id));
            }
        }
        if let Some(task) = new_task {
            for run_id in &task.children {
                if let Some(child_task) = run_id
                    .parse::<AmbientAgentTaskId>()
                    .ok()
                    .and_then(|id| tasks.get(&id))
                {
                    self.visit(Some(child_task), None);
                }
            }
        }
    }
}

/// Returns the local conversation ID that backs the given task, if this task and a
/// conversation entry both point at the same underlying local run.
///
/// We first match using the orchestration agent ID (task ID / run ID under v2), and fall back
/// to the server conversation token for cases where the task only carries conversation identity
/// through `conversation_id`.
pub(crate) fn local_conversation_id_for_task(
    task: &AmbientAgentTask,
    history_model: &BlocklistAIHistoryModel,
) -> Option<AIConversationId> {
    history_model
        .conversation_id_for_agent_id(&task.run_id().to_string())
        .or_else(|| {
            task.conversation_id().and_then(|conversation_id| {
                history_model.find_conversation_id_by_server_token(&ServerConversationToken::new(
                    conversation_id.to_string(),
                ))
            })
        })
}

/// Returns `conversation_id` followed by its orchestration ancestors
/// (nearest parent first), walking parent links on loaded conversations.
pub fn conversation_and_ancestors(
    history: &BlocklistAIHistoryModel,
    conversation_id: AIConversationId,
) -> Vec<AIConversationId> {
    let mut chain = vec![conversation_id];
    let mut current = conversation_id;
    while let Some(parent) = history.conversation(&current).and_then(|conversation| {
        history.resolved_parent_conversation_id_for_conversation(conversation)
    }) {
        // Guard against parent-link cycles.
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        current = parent;
    }
    chain
}

/// Returns whether `conversation_id` is `root_id` itself or one of its
/// descendants, by walking parent links upward from `conversation_id`.
pub fn is_in_orchestration_subtree(
    history: &BlocklistAIHistoryModel,
    root_id: AIConversationId,
    conversation_id: AIConversationId,
) -> bool {
    conversation_and_ancestors(history, conversation_id).contains(&root_id)
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
