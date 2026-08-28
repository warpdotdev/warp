//! Drives the orchestration pill bar in shared session viewers.
//!
//! After the viewer joins a parent ambient-agent session, this model
//! discovers and tracks the parent's direct children via the
//! [`OrchestrationEventStreamer`], which opens an ancestor SSE (seeded
//! by a one-shot REST snapshot) and broadcasts `ChildSpawned` /
//! `ChildStatusChanged` events.
//!
//! Each viewer pane has its own materialization model; durable child identity
//! is shared through `BlocklistAIHistoryModel`, and the streamer is a shared
//! singleton.
//! Pill clicks navigate via `SwapPaneToConversation`.
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use session_sharing_protocol::common::SessionId;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity, WeakViewHandle};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent_conversations_model::{AgentConversationsModel, AgentConversationsModelEvent};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::blocklist::orchestration_event_streamer::{
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
};
use crate::features::FeatureFlag;
use crate::pane_group::{ChildPaneMaterialization, decide_child_pane_materialization};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::{Event as TerminalViewEvent, TerminalView};

/// Refetch cadence for children whose claim-time `session_id` is not yet known.
const PENDING_SESSION_ID_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Per-child orchestration metadata, keyed by `AmbientAgentTaskId`.
struct ChildAgentEntry {
    conversation_id: AIConversationId,
    /// `None` until execution has been claimed.
    session_id: Option<SessionId>,
    /// Cached to deduplicate status writes: we only push an update when the
    /// state actually changes.
    last_state: AmbientAgentTaskState,
    /// True once `EnsureSharedSessionViewerChildPane` has been emitted.
    pane_materialization_requested: bool,
}

/// Owns child discovery + status tracking for a shared session viewer of
/// an orchestrated session.
pub struct OrchestrationViewerModel {
    parent_task_id: AmbientAgentTaskId,
    terminal_view_id: EntityId,
    terminal_view: WeakViewHandle<TerminalView>,
    /// Placeholder conversations materialized for direct children.
    children: HashMap<AmbientAgentTaskId, ChildAgentEntry>,
    /// Secondary index keyed by stringified `run_id`, used by the streamer
    /// broadcast event handler. Kept in sync with `children`.
    children_by_run_id: HashMap<String, AmbientAgentTaskId>,
    /// Task metadata requests in flight, tracked only while
    /// `OrchestrationUnifiedStack` is disabled. Discovery and lifecycle can
    /// race; only one request may create/adopt the durable run-id mapping.
    metadata_fetches: HashSet<AmbientAgentTaskId>,
    /// Children discovered from the streamer whose task data is not cached
    /// yet. Drained on `TasksUpdated`. Only populated while
    /// `OrchestrationUnifiedStack` is enabled.
    pending_task_ids_for_discovery: HashSet<AmbientAgentTaskId>,
    /// Periodic timer fetching the claim-time `session_id` for
    /// not-yet-claimed children.
    pending_session_id_poll_handle: Option<SpawnedFutureHandle>,
    /// Test-only: counts `spawn_task_metadata_fetch` invocations.
    #[cfg(test)]
    metadata_fetch_dispatch_count: usize,
}

impl Entity for OrchestrationViewerModel {
    type Event = ();
}

impl OrchestrationViewerModel {
    /// Returns the orchestrator's `AmbientAgentTaskId`.
    pub fn parent_task_id(&self) -> AmbientAgentTaskId {
        self.parent_task_id
    }
    /// Builds a viewer model attached to the given parent shared session.
    pub fn new(
        parent_task_id: AmbientAgentTaskId,
        terminal_view_id: EntityId,
        terminal_view: WeakViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Subscribe to broadcast events filtered on `parent_task_id`; the
        // streamer handles SSE open/teardown, cold-start seed, and cursor
        // persistence on our behalf.
        let streamer = OrchestrationEventStreamer::handle(ctx);
        ctx.subscribe_to_model(&streamer, move |me, _, event, ctx| {
            me.handle_streamer_event(event, ctx);
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                me.handle_history_event(event, ctx);
            },
        );
        ctx.subscribe_to_model(
            &AgentConversationsModel::handle(ctx),
            |me, _, event, ctx| match event {
                AgentConversationsModelEvent::ConversationsLoaded
                | AgentConversationsModelEvent::NewTasksReceived
                | AgentConversationsModelEvent::TasksUpdated => {
                    me.register_viewer_mode_consumer_if_possible(ctx);
                    if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
                        me.drain_pending_task_discoveries(ctx);
                    }
                }
                AgentConversationsModelEvent::ConversationUpdated { .. }
                | AgentConversationsModelEvent::ConversationArtifactsUpdated { .. } => {}
            },
        );

        let model = Self {
            parent_task_id,
            terminal_view_id,
            terminal_view,
            children: HashMap::new(),
            children_by_run_id: HashMap::new(),
            metadata_fetches: HashSet::new(),
            pending_task_ids_for_discovery: HashSet::new(),
            pending_session_id_poll_handle: None,
            #[cfg(test)]
            metadata_fetch_dispatch_count: 0,
        };
        model.register_viewer_mode_consumer_if_possible(ctx);
        model
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // Stamp `parent_agent_id` on any tracked children once the parent
        // placeholder receives its server token. Children registered before
        // the parent run_id was known would otherwise stay with
        // `parent_agent_id = None` and break parent-conversation lookups.
        self.maybe_backfill_parent_agent_ids(event, ctx);

        match event {
            BlocklistAIHistoryEvent::SetActiveConversation {
                terminal_surface_id,
                ..
            }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                terminal_surface_id,
                ..
            } if *terminal_surface_id == self.terminal_view_id => {
                self.register_viewer_mode_consumer_if_possible(ctx);
            }
            _ => {}
        }
    }

    /// Registers this model as a viewer-mode consumer once the active
    /// conversation is the orchestrator placeholder (identified by
    /// `is_viewing_shared_session() && parent_conversation_id().is_none()`).
    /// Defers if the placeholder hasn't been stamped yet; re-runs from
    /// history events that may flip the placeholder state.
    fn register_viewer_mode_consumer_if_possible(&self, ctx: &mut ModelContext<Self>) {
        let Some(parent_conversation_id) =
            BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.terminal_view_id)
        else {
            log::debug!(
                "[orch-viewer] no active conversation yet for terminal_view_id={:?} \
                 parent_task_id={}; registration deferred",
                self.terminal_view_id,
                self.parent_task_id,
            );
            return;
        };
        let (is_viewing_shared_session, has_parent_conv) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&parent_conversation_id)
            .map(|conversation| {
                (
                    conversation.is_viewing_shared_session(),
                    conversation.parent_conversation_id().is_some(),
                )
            })
            .unwrap_or((false, false));
        let is_parent_placeholder = is_viewing_shared_session && !has_parent_conv;
        if !is_parent_placeholder {
            log::debug!(
                "[orch-viewer] active conversation {parent_conversation_id:?} for \
                 terminal_view_id={:?} is not the parent placeholder yet \
                 (is_viewing_shared_session={is_viewing_shared_session}, \
                 has_parent_conv={has_parent_conv}); registration deferred",
                self.terminal_view_id,
            );
            return;
        }

        let parent_task_id = self.parent_task_id;
        let consumer_id = ctx.model_id();
        OrchestrationEventStreamer::handle(ctx).update(ctx, move |streamer, ctx| {
            streamer.register_viewer_mode_consumer(
                parent_task_id,
                parent_conversation_id,
                consumer_id,
                ctx,
            );
        });
    }

    /// Routes broadcast events from the streamer, filtered on this model's
    /// `parent_task_id`.
    fn handle_streamer_event(
        &mut self,
        event: &OrchestrationEventStreamerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            OrchestrationEventStreamerEvent::ChildSpawned {
                parent_task_id,
                run_id,
            } if *parent_task_id == self.parent_task_id => {
                self.handle_child_spawned(run_id.clone(), ctx);
            }
            OrchestrationEventStreamerEvent::ChildStatusChanged {
                parent_task_id,
                run_id,
                status,
            } if *parent_task_id == self.parent_task_id => {
                self.handle_child_status_changed(run_id, status.clone(), ctx);
            }
            // Other orchestrators (or non-viewer-mode variants) are ignored.
            _ => {}
        }
    }

    /// First observation of a child `run_id`. Fetches pill metadata and
    /// dispatches to `register_child`. Dropped events are retried on the
    /// next status change for the same `run_id`.
    fn handle_child_spawned(&mut self, run_id: String, ctx: &mut ModelContext<Self>) {
        let Ok(task_id) = run_id.parse::<AmbientAgentTaskId>() else {
            log::warn!("[orch-viewer] ChildSpawned with malformed run_id={run_id:?}; dropping");
            return;
        };
        if self.children.contains_key(&task_id) {
            // Already materialized (e.g. re-registered after reconnect).
            return;
        }
        self.spawn_task_metadata_fetch(task_id, "ChildSpawned", ctx);
    }

    /// Writes the new status through `BlocklistAIHistoryModel`. If the entry
    /// has not reached a live, transcript, or legacy-session materialization
    /// yet, also refreshes its task metadata.
    fn handle_child_status_changed(
        &mut self,
        run_id: &str,
        status: ConversationStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(task_id) = self.children_by_run_id.get(run_id).copied() else {
            // Lifecycle may arrive before (or instead of) ChildStarted.
            if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
                self.handle_child_spawned(run_id.to_string(), ctx);
            }
            return;
        };
        let Some(entry) = self.children.get(&task_id) else {
            return;
        };
        let conversation_id = entry.conversation_id;
        let needs_metadata_refetch = Self::entry_needs_materialization_metadata(entry);
        let terminal_view_id = self.terminal_view_id;
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.update_conversation_status(terminal_view_id, conversation_id, status, ctx);
        });

        if needs_metadata_refetch {
            self.spawn_task_metadata_fetch(task_id, "ChildStatusChanged", ctx);
        }
    }

    /// Fetches a single task's metadata and routes the response through
    /// `register_child`. The `trigger` label is logged on failure to
    /// distinguish the caller.
    fn spawn_task_metadata_fetch(
        &mut self,
        task_id: AmbientAgentTaskId,
        trigger: &'static str,
        ctx: &mut ModelContext<Self>,
    ) {
        if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
            // Route through the shared task cache so discovery reuses whatever
            // fetch is already in flight. A hit registers the child now; a miss
            // parks it until the cache reports fresh data.
            let cached = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(&task_id, ctx)
            });
            if let Some(task) = cached {
                self.register_child(task, ctx);
            } else {
                self.pending_task_ids_for_discovery.insert(task_id);
            }
            #[cfg(test)]
            {
                self.metadata_fetch_dispatch_count += 1;
            }
            return;
        }
        self.metadata_fetches.insert(task_id);
        #[cfg(test)]
        {
            self.metadata_fetch_dispatch_count += 1;
        }
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let parent_task_id = self.parent_task_id;
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |me, result, ctx| {
                me.metadata_fetches.remove(&task_id);
                let task = match result {
                    Ok(task) => task,
                    Err(err) => {
                        log::warn!(
                            "[orch-viewer] failed to fetch pill metadata for \
                             child task_id={task_id} parent_task_id={parent_task_id} \
                             trigger={trigger}: {err:#}"
                        );
                        return;
                    }
                };
                me.register_child(task, ctx);
            },
        );
    }

    /// Registers every parked child whose task data has since been cached.
    fn drain_pending_task_discoveries(&mut self, ctx: &mut ModelContext<Self>) {
        let ready: Vec<_> = self
            .pending_task_ids_for_discovery
            .iter()
            .filter_map(|&task_id| {
                AgentConversationsModel::as_ref(ctx)
                    .get_task_data(&task_id)
                    .map(|task| (task_id, task))
            })
            .collect();
        for (task_id, task) in ready {
            self.pending_task_ids_for_discovery.remove(&task_id);
            self.register_child(task, ctx);
        }
    }

    // ---- Shared child registration (used by both paths) -----------------

    /// Creates the local placeholder conversation for a child task, records
    /// it in the per-pane map, and requests materialization when current task
    /// state is attachable or transcript-loadable. Idempotent: a second call
    /// for the same `task_id` updates status and materialization state only.
    fn register_child(&mut self, task: AmbientAgentTask, ctx: &mut ModelContext<Self>) {
        // The server-side ancestor endpoint includes the parent itself in
        // the response; skip it.
        if task.task_id == self.parent_task_id {
            return;
        }

        let task_id = task.task_id;
        let session_id = task
            .session_id
            .as_deref()
            .and_then(|s| s.parse::<SessionId>().ok());
        let materialization_ready = Self::materialization_is_ready(&task);
        let new_state = task.state.clone();
        let conversation_status = conversation_status_from_state(&new_state);

        if let Some(entry) = self.children.get_mut(&task_id) {
            // Existing child: update status if it changed and fill in
            // session id once it becomes available. Can be called again
            // on streamer reconnect.
            if entry.last_state != new_state {
                let conversation_id = entry.conversation_id;
                let terminal_view_id = self.terminal_view_id;
                let status_for_update = conversation_status.clone();
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.update_conversation_status(
                        terminal_view_id,
                        conversation_id,
                        status_for_update,
                        ctx,
                    );
                });
                entry.last_state = new_state;
            }
            entry.session_id = session_id;
            let should_request_materialization =
                materialization_ready && !entry.pane_materialization_requested;
            if should_request_materialization {
                let conversation_id = entry.conversation_id;
                entry.pane_materialization_requested = true;
                self.request_child_pane_materialization(conversation_id, task, ctx);
            }
            // Re-arm the session_id timer; no-op once all children are materialized.
            self.maybe_schedule_pending_session_id_poll(ctx);
            return;
        }

        // New child: register under the orchestrator's local conversation.
        // Without an active parent conversation, `start_new_child_conversation`
        // would lose the parent linkage. Drop and try again next cycle/event.
        let Some(parent_conversation_id) = self.find_parent_conversation_id(ctx) else {
            log::warn!(
                "[orch-viewer] no active parent conversation for terminal_view_id={:?} \
                 parent_task_id={}; deferring child registration for task_id={task_id}",
                self.terminal_view_id,
                self.parent_task_id,
            );
            return;
        };
        let name = task.display_name().to_string();
        // Trim to stay in sync with `display_name()`, which also trims;
        // the descriptive title flows through `set_fallback_display_title`
        // so `AIConversation::title()` keeps surfacing it.
        let fallback_title = task.title.trim().to_string();
        let harness = task
            .agent_config_snapshot
            .as_ref()
            .and_then(|c| c.harness.as_ref())
            .map(|h| h.harness_type);
        let terminal_view_id = self.terminal_view_id;
        let status_for_initial = conversation_status.clone();

        let unified_stack = FeatureFlag::OrchestrationUnifiedStack.is_enabled();
        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = if unified_stack {
                history.ensure_remote_child_conversation(
                    terminal_view_id,
                    parent_conversation_id,
                    task_id.to_string(),
                    task_id,
                    name,
                    fallback_title,
                    harness,
                    ctx,
                )
            } else {
                let conversation_id = history.start_new_child_conversation(
                    terminal_view_id,
                    name,
                    parent_conversation_id,
                    harness,
                    false,
                    ctx,
                );
                history.set_viewing_shared_session_for_conversation(conversation_id, true);
                if !fallback_title.is_empty()
                    && let Some(conversation) = history.conversation_mut(&conversation_id)
                {
                    conversation.set_fallback_display_title(fallback_title);
                }
                history.assign_run_id_for_conversation(
                    conversation_id,
                    task_id.to_string(),
                    Some(task_id),
                    terminal_view_id,
                    ctx,
                );
                conversation_id
            };
            history.update_conversation_status(
                terminal_view_id,
                conversation_id,
                status_for_initial,
                ctx,
            );
            conversation_id
        });

        let pane_materialization_requested = materialization_ready;
        self.children.insert(
            task_id,
            ChildAgentEntry {
                conversation_id,
                session_id,
                last_state: new_state.clone(),
                pane_materialization_requested,
            },
        );
        self.children_by_run_id.insert(task_id.to_string(), task_id);
        log::info!(
            "[orch-viewer] registered child placeholder task_id={task_id} \
             parent_task_id={} conversation_id={conversation_id:?} \
             session_id={session_id:?} initial_state={new_state:?}",
            self.parent_task_id,
        );

        if pane_materialization_requested {
            self.request_child_pane_materialization(conversation_id, task, ctx);
        }

        // Arm the session_id refetch timer if the child arrived pre-claim.
        self.maybe_schedule_pending_session_id_poll(ctx);
    }

    // ---- Pending-session_id polling ----------------------------------

    /// True iff at least one tracked child is still pending materialization.
    fn has_pending_session_id_children(&self) -> bool {
        self.children
            .values()
            .any(Self::entry_needs_materialization_metadata)
    }

    /// Schedules the next session_id refetch tick.
    /// Safe to call unconditionally — bails when not needed.
    fn maybe_schedule_pending_session_id_poll(&mut self, ctx: &mut ModelContext<Self>) {
        if self.pending_session_id_poll_handle.is_some() {
            return;
        }
        if !self.has_pending_session_id_children() {
            return;
        }
        let handle = ctx.spawn(
            async {
                Timer::after(PENDING_SESSION_ID_POLL_INTERVAL).await;
            },
            |me, _, ctx| {
                me.pending_session_id_poll_handle = None;
                me.run_pending_session_id_poll(ctx);
            },
        );
        self.pending_session_id_poll_handle = Some(handle);
    }

    /// Body of the metadata timer tick. Refetches every child that has not
    /// reached a materializable state, then reschedules until none remain.
    fn run_pending_session_id_poll(&mut self, ctx: &mut ModelContext<Self>) {
        let pending: Vec<AmbientAgentTaskId> = self
            .children
            .iter()
            .filter(|(_, entry)| Self::entry_needs_materialization_metadata(entry))
            .map(|(task_id, _)| *task_id)
            .collect();

        if pending.is_empty() {
            return;
        }

        for task_id in pending {
            self.spawn_task_metadata_fetch(task_id, "PendingSessionIdPoll", ctx);
        }

        self.maybe_schedule_pending_session_id_poll(ctx);
    }

    // ---- Helpers -------------------------------------------------------

    /// Backfills `parent_agent_id` on viewer-created children once the
    /// orchestrator receives its server token. Children registered before
    /// the parent's run_id was known would otherwise stay with
    /// `parent_agent_id = None` and break parent-conversation lookups.
    fn maybe_backfill_parent_agent_ids(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id, ..
        } = event
        else {
            return;
        };
        let conversation_id = *conversation_id;
        if self.find_parent_conversation_id(ctx) != Some(conversation_id) {
            return;
        }
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let parent_agent_id = history_handle
            .as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.orchestration_agent_id());
        let Some(parent_agent_id) = parent_agent_id else {
            return;
        };
        let child_conversation_ids: Vec<AIConversationId> = self
            .children
            .values()
            .map(|child| child.conversation_id)
            .collect();
        history_handle.update(ctx, |history, _ctx| {
            for child_id in child_conversation_ids {
                let Some(child) = history.conversation_mut(&child_id) else {
                    continue;
                };
                if child.parent_agent_id().is_some() {
                    continue;
                }
                child.set_parent_agent_id(parent_agent_id.clone());
            }
        });
    }

    /// Resolves the orchestrator's local conversation id via the view's
    /// active conversation, which `on_shared_init` sets on first join.
    fn find_parent_conversation_id(&self, ctx: &ModelContext<Self>) -> Option<AIConversationId> {
        BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.terminal_view_id)
    }

    fn materialization_is_ready(task: &AmbientAgentTask) -> bool {
        if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
            return !matches!(
                decide_child_pane_materialization(task),
                ChildPaneMaterialization::Pending
            );
        }

        task.session_id
            .as_deref()
            .and_then(|session_id| session_id.parse::<SessionId>().ok())
            .is_some()
    }

    fn entry_needs_materialization_metadata(entry: &ChildAgentEntry) -> bool {
        !entry.pane_materialization_requested
            || (!FeatureFlag::OrchestrationUnifiedStack.is_enabled() && entry.session_id.is_none())
    }

    /// Tells the parent's `TerminalView` to materialize a hidden viewer pane
    /// for this child. Unified-stack routing carries the task snapshot so the
    /// pane group can distinguish live, transcript, and pending state; the
    /// legacy flag-off route preserves its raw-session-id behavior.
    fn request_child_pane_materialization(
        &self,
        conversation_id: AIConversationId,
        task: AmbientAgentTask,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(view) = self.terminal_view.upgrade(ctx) else {
            log::warn!(
                "[orch-viewer] cannot request child pane materialization for conv={conversation_id:?}: \
                 parent terminal view is gone"
            );
            return;
        };
        view.update(ctx, |_view, ctx| {
            if FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
                ctx.emit(TerminalViewEvent::EnsureUnifiedViewerChildPane {
                    conversation_id,
                    task: Box::new(task),
                });
            } else if let Some(session_id) = task
                .session_id
                .as_deref()
                .and_then(|session_id| session_id.parse::<SessionId>().ok())
            {
                ctx.emit(TerminalViewEvent::EnsureSharedSessionViewerChildPane {
                    conversation_id,
                    session_id,
                });
            }
        });
    }
}

/// Maps a server-side run state to the [`ConversationStatus`] used by the
/// pill bar and the conversation list. Working states (queued/pending/claimed/
/// in-progress) all collapse to [`ConversationStatus::InProgress`] so the
/// pill badge stays in the loading spinner until the run terminates.
fn conversation_status_from_state(state: &AmbientAgentTaskState) -> ConversationStatus {
    match state {
        AmbientAgentTaskState::Queued
        | AmbientAgentTaskState::Pending
        | AmbientAgentTaskState::Claimed
        | AmbientAgentTaskState::InProgress => ConversationStatus::InProgress,
        AmbientAgentTaskState::Succeeded => ConversationStatus::Success,
        AmbientAgentTaskState::Failed | AmbientAgentTaskState::Error => ConversationStatus::Error,
        AmbientAgentTaskState::Blocked => ConversationStatus::Blocked {
            blocked_action: String::new(),
        },
        AmbientAgentTaskState::Cancelled => ConversationStatus::Cancelled,
        // The `Unknown` variant is a forward-compat catch-all for server
        // states the client doesn't recognize yet. The rest of the codebase
        // (`is_terminal`, `is_failure_like`, `Display`, `status_icon_and_color`)
        // consistently treats it as a terminal error, so we follow suit.
        AmbientAgentTaskState::Unknown => ConversationStatus::Error,
    }
}

#[cfg(test)]
#[path = "orchestration_viewer_model_tests.rs"]
mod tests;
