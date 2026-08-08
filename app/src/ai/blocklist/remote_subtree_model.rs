//! Pull-based discovery of remote orchestration subtrees.
//!
//! A remote child conversation is a local placeholder for a run executing on
//! a cloud worker. When that run orchestrates children of its own, nothing
//! pushes them to this client (v1 is pull-only): this model reads the
//! `children` list on the run item from the public API, materializes local
//! placeholder conversations for them (mirroring the shared-session viewer's
//! pattern), and refreshes their statuses on a slow poll so rollup badges
//! stay fresh. The placeholders flow through the regular
//! `children_by_parent` topology, so the pill bar, keyboard navigation, and
//! status rollups need no remote-specific handling.
use std::collections::HashMap;
use std::time::Duration;

use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::task::AmbientAgentTask;
use crate::ai::ambient_agents::{AmbientAgentTaskId, AmbientAgentTaskState};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::shared_session::viewer::orchestration_viewer_model::conversation_status_from_state;

/// Refresh cadence for watched remote subtrees. Deliberately slow: v1 has no
/// push channel for grandchild lifecycle, so this only keeps rollup badges
/// from going stale.
const REMOTE_SUBTREE_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// One watched remote node (a local remote-child placeholder whose cloud run
/// may orchestrate children of its own).
struct RemoteSubtreeEntry {
    task_id: AmbientAgentTaskId,
    /// Children materialized as local placeholder conversations, keyed by
    /// child run id.
    children: HashMap<AmbientAgentTaskId, RemoteSubtreeChild>,
    /// Guards against overlapping fetches for the same node.
    fetch_in_flight: bool,
    /// `true` once a fetch issued AFTER the node terminated has completed
    /// successfully. Children cannot be created past that point, so the
    /// server-side children list is final and polling may stop. Without
    /// this "final sweep", children spawned between the last fetch and a
    /// fast termination would never be discovered.
    final_sweep_done: bool,
}

struct RemoteSubtreeChild {
    conversation_id: AIConversationId,
    /// Last server state written through to the placeholder, used to
    /// deduplicate status updates. `None` for placeholders adopted from a
    /// previous session before the first fetch lands.
    last_state: Option<AmbientAgentTaskState>,
}

pub struct RemoteSubtreeModel {
    entries: HashMap<AIConversationId, RemoteSubtreeEntry>,
    poll_timer_armed: bool,
}

impl Entity for RemoteSubtreeModel {
    type Event = ();
}

impl SingletonEntity for RemoteSubtreeModel {}

impl RemoteSubtreeModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |me, _, event, _ctx| {
            me.handle_history_event(event);
        });
        Self {
            entries: HashMap::new(),
            poll_timer_armed: false,
        }
    }

    /// Starts watching a conversation's remote subtree. No-op unless the
    /// conversation is a remote-child placeholder with a known run id.
    /// Idempotent: an already-watched conversation is refreshed by the slow
    /// poll rather than re-fetched here.
    pub fn watch(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        if self.entries.contains_key(&conversation_id) {
            return;
        }
        let task_id = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation) = history.conversation(&conversation_id) else {
                return;
            };
            if !conversation.is_remote_child() {
                return;
            }
            let Some(task_id) = conversation
                .task_id()
                .or_else(|| conversation.run_id().and_then(|id| id.parse().ok()))
            else {
                return;
            };
            task_id
        };
        self.entries.insert(
            conversation_id,
            RemoteSubtreeEntry {
                task_id,
                children: HashMap::new(),
                fetch_in_flight: false,
                final_sweep_done: false,
            },
        );
        self.adopt_existing_local_children(conversation_id, ctx);
        self.spawn_subtree_fetch(conversation_id, ctx);
        self.arm_poll_timer(ctx);
    }

    /// Adopts placeholder children restored from a previous session so the
    /// first fetch updates them in place instead of duplicating them.
    fn adopt_existing_local_children(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &ModelContext<Self>,
    ) {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let adopted: Vec<(AmbientAgentTaskId, AIConversationId)> = history
            .child_conversation_ids_of(&conversation_id)
            .iter()
            .filter_map(|child_id| {
                let child = history.conversation(child_id)?;
                Some((child.task_id()?, *child_id))
            })
            .collect();
        let Some(entry) = self.entries.get_mut(&conversation_id) else {
            return;
        };
        for (task_id, child_conversation_id) in adopted {
            entry.children.entry(task_id).or_insert(RemoteSubtreeChild {
                conversation_id: child_conversation_id,
                last_state: None,
            });
        }
    }

    /// Fetches the watched node's run item; its `children` list drives
    /// per-child metadata fetches.
    fn spawn_subtree_fetch(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Captured at fetch-issue time (not completion) so a run that
        // terminates mid-flight still gets one more fetch before the entry
        // is allowed to go dormant.
        let node_terminal_at_fetch = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.status().is_done());
        let Some(entry) = self.entries.get_mut(&conversation_id) else {
            return;
        };
        if entry.fetch_in_flight {
            return;
        }
        entry.fetch_in_flight = true;
        let task_id = entry.task_id;
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |me, result, ctx| {
                me.finish_subtree_fetch(conversation_id, node_terminal_at_fetch, result, ctx);
            },
        );
    }

    fn finish_subtree_fetch(
        &mut self,
        conversation_id: AIConversationId,
        node_terminal_at_fetch: bool,
        result: anyhow::Result<AmbientAgentTask>,
        ctx: &mut ModelContext<Self>,
    ) {
        let (own_task_id, child_run_ids) = {
            let Some(entry) = self.entries.get_mut(&conversation_id) else {
                return;
            };
            entry.fetch_in_flight = false;
            let task = match result {
                Ok(task) => task,
                Err(err) => {
                    // The flag stays untouched on failure, so the next poll
                    // retries instead of going dormant on a failed sweep.
                    log::warn!(
                        "remote subtree fetch failed for {conversation_id:?} \
                         task_id={}: {err:#}; will retry on the next poll",
                        entry.task_id
                    );
                    return;
                }
            };
            entry.final_sweep_done = node_terminal_at_fetch;
            (entry.task_id, task.children)
        };
        for child_run_id in child_run_ids {
            let Ok(child_task_id) = child_run_id.parse::<AmbientAgentTaskId>() else {
                log::warn!("remote subtree child has malformed run_id {child_run_id:?}; skipping");
                continue;
            };
            // The endpoint may echo the node itself; only children matter.
            if child_task_id == own_task_id {
                continue;
            }
            self.spawn_child_fetch(conversation_id, child_task_id, ctx);
        }
    }

    /// Fetches one child run's metadata so its placeholder gets a name,
    /// harness, and fresh status.
    fn spawn_child_fetch(
        &mut self,
        parent_conversation_id: AIConversationId,
        child_task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&child_task_id).await },
            move |me, result, ctx| {
                let task = match result {
                    Ok(task) => task,
                    Err(err) => {
                        log::warn!(
                            "remote subtree child fetch failed for task_id={child_task_id} \
                             under {parent_conversation_id:?}: {err:#}"
                        );
                        return;
                    }
                };
                me.register_or_update_child(parent_conversation_id, task, ctx);
            },
        );
    }

    /// Creates or refreshes the local placeholder conversation for a remote
    /// grandchild. Mirrors the shared-session viewer's `register_child`, but
    /// marks the placeholder as a remote child (this client owns the view,
    /// not the run).
    fn register_or_update_child(
        &mut self,
        parent_conversation_id: AIConversationId,
        task: AmbientAgentTask,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(terminal_view_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(&parent_conversation_id)
        else {
            // The watched node lost its surface (pane closed mid-fetch); the
            // next watch/poll re-checks.
            return;
        };
        let child_task_id = task.task_id;
        let new_state = task.state.clone();
        let status = conversation_status_from_state(&new_state);

        if let Some(child) = self
            .entries
            .get_mut(&parent_conversation_id)
            .and_then(|entry| entry.children.get_mut(&child_task_id))
        {
            if child.last_state.as_ref() != Some(&new_state) {
                child.last_state = Some(new_state);
                let child_conversation_id = child.conversation_id;
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.update_conversation_status(
                        terminal_view_id,
                        child_conversation_id,
                        status,
                        ctx,
                    );
                });
            }
            return;
        }

        // A placeholder may already exist locally without having been
        // adopted (e.g. created by another surface); reuse it by run id.
        let existing_conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation_id_for_agent_id(&child_task_id.to_string());
        let child_conversation_id = match existing_conversation_id {
            Some(existing_id) => {
                let status = status.clone();
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.update_conversation_status(terminal_view_id, existing_id, status, ctx);
                });
                existing_id
            }
            None => {
                let name = task.display_name().to_string();
                // Trim to stay in sync with `display_name()`, which also
                // trims; the descriptive title flows through
                // `set_fallback_display_title`.
                let fallback_title = task.title.trim().to_string();
                let harness = task
                    .agent_config_snapshot
                    .as_ref()
                    .and_then(|config| config.harness.as_ref())
                    .map(|harness| harness.harness_type);
                let status = status.clone();
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let conversation_id = history.start_new_child_conversation(
                        terminal_view_id,
                        name,
                        parent_conversation_id,
                        harness,
                        ctx,
                    );
                    history.mark_conversation_as_remote_child(conversation_id, ctx);
                    if let Some(conversation) = history.conversation_mut(&conversation_id)
                        && !fallback_title.is_empty()
                    {
                        conversation.set_fallback_display_title(fallback_title);
                    }
                    history.assign_run_id_for_conversation(
                        conversation_id,
                        child_task_id.to_string(),
                        Some(child_task_id),
                        terminal_view_id,
                        ctx,
                    );
                    history.update_conversation_status(
                        terminal_view_id,
                        conversation_id,
                        status,
                        ctx,
                    );
                    conversation_id
                })
            }
        };

        if let Some(entry) = self.entries.get_mut(&parent_conversation_id) {
            entry.children.insert(
                child_task_id,
                RemoteSubtreeChild {
                    conversation_id: child_conversation_id,
                    last_state: Some(new_state),
                },
            );
        }
    }

    // ---- Slow poll -----------------------------------------------------

    fn arm_poll_timer(&mut self, ctx: &mut ModelContext<Self>) {
        if self.poll_timer_armed || self.entries.is_empty() {
            return;
        }
        self.poll_timer_armed = true;
        ctx.spawn(
            async { Timer::after(REMOTE_SUBTREE_POLL_INTERVAL).await },
            |me, _, ctx| {
                me.poll_timer_armed = false;
                me.run_poll_tick(ctx);
            },
        );
    }

    fn run_poll_tick(&mut self, ctx: &mut ModelContext<Self>) {
        let refetch_ids: Vec<AIConversationId> = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            self.entries
                .iter()
                .filter(|(conversation_id, entry)| {
                    subtree_may_still_change(conversation_id, entry, history)
                })
                .map(|(conversation_id, _)| *conversation_id)
                .collect()
        };
        for conversation_id in refetch_ids {
            self.spawn_subtree_fetch(conversation_id, ctx);
        }
        self.arm_poll_timer(ctx);
    }

    fn handle_history_event(&mut self, event: &BlocklistAIHistoryEvent) {
        match event {
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                self.unwatch(conversation_id);
            }
            // A bulk clear emits no per-conversation remove events, and a cleared conversation
            // loses its surface, so its local status freezes and the entry would keep polling
            // forever. Drop the entries; a restore/reopen re-watches via the pill bar's regular
            // sync triggers.
            BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface {
                cleared_conversation_ids,
                ..
            } => {
                for conversation_id in cleared_conversation_ids {
                    self.unwatch(conversation_id);
                }
            }
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::ReassignedExchange { .. }
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
        }
    }

    /// Stops watching a conversation and drops any child references to it.
    fn unwatch(&mut self, conversation_id: &AIConversationId) {
        self.entries.remove(conversation_id);
        for entry in self.entries.values_mut() {
            entry
                .children
                .retain(|_, child| child.conversation_id != *conversation_id);
        }
    }
}

/// A watched subtree goes dormant only once the node and all of its known
/// children are terminal AND a fetch issued after the node terminated has
/// completed successfully (`final_sweep_done`) — without that sweep,
/// children spawned between the last fetch and a fast termination would
/// never be discovered. A revival (e.g. a completion report waking a
/// dormant parent) flips the node's local status back to non-terminal via
/// the regular event stream, which re-activates polling; the sweep flag
/// resets on the next fetch completed while the node is active.
fn subtree_may_still_change(
    conversation_id: &AIConversationId,
    entry: &RemoteSubtreeEntry,
    history: &BlocklistAIHistoryModel,
) -> bool {
    let node_active = history
        .conversation(conversation_id)
        .is_some_and(|conversation| !conversation.status().is_done());
    node_active
        || !entry.final_sweep_done
        || entry.children.values().any(|child| {
            history
                .conversation(&child.conversation_id)
                .is_some_and(|conversation| !conversation.status().is_done())
        })
}

#[cfg(test)]
#[path = "remote_subtree_model_tests.rs"]
mod tests;
