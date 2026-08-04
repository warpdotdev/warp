//! Guides child runs from first discovery through to pane materialization.
//!
//! When the parent's SSE stream fires a `child_agent_started` event, the
//! tracker creates a local placeholder, fetches the child's task metadata,
//! waits for the sandbox session to be linked, and then requests a live or
//! transcript pane. Every signal a child can produce — discovery, lifecycle,
//! session links, REST seed rows, and in-band registrations — enters through
//! the single [`OrchestrationChildTracker::observe_child`] entry point.
//!
//! `FamilyDrainMode` captures the one behavioral axis between orchestrator
//! and shared-session observer: who pushes the server cursor and who receives
//! the parent's own inbox events. It says nothing about authenticated
//! ownership, permissions, or pane capability.
//!
//! Pill-bar broadcasts (`ChildSpawned` / `ChildStatusChanged`) are emitted
//! via the `ctx` so downstream views can react without polling.
//!
//! # TODO: unify `is_remote_child` and `is_viewing_shared_session`
//! Both flags mark conversations that are local placeholders for a remote run
//! accessed via the shared-session protocol; they differ only in which code
//! path created the placeholder. Merging them into a single
//! `is_remote_placeholder` flag would let every placeholder conversation be
//! persisted uniformly.

use std::collections::{HashMap, HashSet};

use session_sharing_protocol::common::SessionId;
use warp_multi_agent_api as api;
use warpui::ModelContext;
#[cfg(not(test))]
use warpui::SingletonEntity;

#[cfg(not(test))]
use super::history_model::BlocklistAIHistoryModel;
use super::orchestration_event_streamer::{
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
    conversation_status_from_lifecycle_event_type,
};
// Compiled out of unit-test builds so the state machine can be exercised
// without installing the full model singleton graph.
#[cfg(not(test))]
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};

/// Every way a child run can become known funnels into
/// [`OrchestrationChildTracker::observe_child`].
pub enum ChildSignal {
    /// `child_agent_started` on the parent run (child run id in `ref_id`).
    Started,
    /// `run_session_linked` on the child run: carries the sandbox session
    /// UUID directly, so `session_id` can be filled in without a metadata
    /// fetch.
    SessionLinked { session_uuid: String },
    /// Any recognised lifecycle event on the child run.
    Lifecycle(api::LifecycleEventType),
    /// A REST seed row (cold-start seed / restore fetch). Boxed because the
    /// task row dwarfs the other variants.
    #[allow(dead_code)]
    Seeded(Box<AmbientAgentTask>),
    /// A child created in this process, already backed by a local
    /// conversation that its executor hydrates.
    #[allow(dead_code)]
    Registered,
}

/// Per-child orchestration state, keyed by [`AmbientAgentTaskId`].
pub struct TrackedChild {
    /// `None` until execution is claimed and a session is linked.
    pub session_id: Option<SessionId>,
    /// Last observed task state, when known (seeded/refetched rows).
    pub last_state: Option<AmbientAgentTaskState>,
    /// True once pane materialization has been requested for this child.
    pub pane_materialized: bool,
    /// `true` for every placeholder the tracker materializes on behalf of a
    /// run hosted elsewhere. `false` only for in-band children, which already
    /// own a real local conversation and are tracked for status only.
    #[allow(dead_code)]
    pub is_remote_child: bool,
}

/// Owns discovery, placeholder bookkeeping, claim-time metadata refetch, and
/// pane-materialization requests for one parent family.
pub struct OrchestrationChildTracker {
    parent_task_id: AmbientAgentTaskId,
    /// Materialized children keyed by task id.
    children: HashMap<AmbientAgentTaskId, TrackedChild>,
    /// Secondary index from stringified `run_id` to task id, kept in sync
    /// with `children`.
    children_by_run_id: HashMap<String, AmbientAgentTaskId>,
    /// Children created in this process (`ChildSignal::Registered`). They are
    /// observed for status only: their conversation and session are assigned
    /// elsewhere, so no discovery fetch is ever issued on their behalf.
    in_band_children: HashSet<AmbientAgentTaskId>,
    /// In-flight metadata fetches keyed by `run_id`. A second discovery signal
    /// for a run already being fetched is a no-op.
    metadata_fetches: HashSet<String>,
    /// Session ids delivered by `run_session_linked` before the child's
    /// placeholder exists; applied when the child is created.
    pending_session_ids: HashMap<AmbientAgentTaskId, SessionId>,
    /// Test-only: counts stubbed metadata-fetch dispatches so fetch dedup can
    /// be asserted without any network plumbing.
    #[cfg(test)]
    metadata_fetch_dispatch_count: usize,
}

impl OrchestrationChildTracker {
    /// Builds an empty tracker for the given parent family.
    pub fn new(parent_task_id: AmbientAgentTaskId) -> Self {
        Self {
            parent_task_id,
            children: HashMap::new(),
            children_by_run_id: HashMap::new(),
            in_band_children: HashSet::new(),
            metadata_fetches: HashSet::new(),
            pending_session_ids: HashMap::new(),
            #[cfg(test)]
            metadata_fetch_dispatch_count: 0,
        }
    }

    /// The single entry point for all child state changes: drops signals for
    /// killed runs, creates or updates the child's placeholder, writes status
    /// through on `Lifecycle` signals, refetches metadata while the child is
    /// incompletely hydrated, and requests pane materialization once the
    /// session id is known.
    pub fn observe_child(
        &mut self,
        child_run_id: &str,
        signal: ChildSignal,
        killed_run_ids: &HashSet<String>,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // Gate every signal on the tombstone set before any placeholder
        // creation or pane request, so a locally killed run cannot be
        // resurrected by a late event or an in-flight metadata fetch.
        if killed_run_ids.contains(child_run_id) {
            self.forget_run(child_run_id);
            return;
        }

        let Ok(task_id) = child_run_id.parse::<AmbientAgentTaskId>() else {
            log::warn!(
                "[orch-tracker] signal for malformed run_id={child_run_id:?} \
                 (parent_task_id={}); dropping",
                self.parent_task_id,
            );
            return;
        };

        match signal {
            ChildSignal::Registered => {
                self.register_in_band_child(task_id, child_run_id, ctx);
            }
            ChildSignal::SessionLinked { session_uuid } => {
                self.apply_session_linked(task_id, &session_uuid);
            }
            ChildSignal::Lifecycle(kind) => {
                self.apply_lifecycle(task_id, child_run_id, kind, ctx);
            }
            ChildSignal::Seeded(task) => {
                self.apply_seeded(*task, ctx);
            }
            ChildSignal::Started => {
                self.apply_started(task_id, child_run_id, ctx);
            }
        }
    }

    /// Discovery via `child_agent_started`. Idempotent: an already-known child
    /// only continues hydrating, while the first sighting of a genuinely new
    /// out-of-band run inserts a pending `TrackedChild` immediately — before
    /// the async metadata fetch completes — so later `Lifecycle` and
    /// `SessionLinked` signals see a known child and are processed.
    fn apply_started(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        if self.children.contains_key(&task_id) {
            // Already represented; keep hydrating if not yet complete.
            self.refetch_metadata_if_incomplete(task_id, run_id, ctx);
            self.maybe_request_pane_materialization(task_id, ctx);
            return;
        }
        // Insert a placeholder TrackedChild immediately so lifecycle and
        // session-linked signals that arrive before the async metadata fetch
        // completes see tracker_known=true. Any session_id that arrived
        // before this signal is also applied now.
        let session_id = self.pending_session_ids.remove(&task_id);
        self.insert_child(
            task_id,
            run_id,
            TrackedChild {
                session_id,
                last_state: None,
                pane_materialized: false,
                is_remote_child: true,
            },
            ctx,
        );
        // Also kick the metadata fetch to get real task state, session_id,
        // and conversation token for transcript / live-attach decisions.
        self.spawn_metadata_fetch(task_id, run_id, ctx);
    }

    /// Sole status writer for placeholder children: writes the status through
    /// to the history model so the pill badge updates immediately, and
    /// broadcasts `ChildStatusChanged`. A lifecycle event for an unknown child
    /// falls back to discovery, making it a self-healing backstop for a missed
    /// `child_agent_started`.
    fn apply_lifecycle(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        kind: api::LifecycleEventType,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        let tracker_known = self.children.contains_key(&task_id);
        if tracker_known {
            let status = conversation_status_from_lifecycle_event_type(kind);
            // Write status through immediately so the pill bar badge reflects
            // the lifecycle transition without waiting for a redraw cycle.
            #[cfg(not(test))]
            {
                let child_info = {
                    let history = BlocklistAIHistoryModel::as_ref(ctx);
                    history
                        .conversation_id_for_agent_id(run_id)
                        .and_then(|child_conv_id| {
                            history
                                .terminal_surface_id_for_conversation(&child_conv_id)
                                .map(|surface_id| (child_conv_id, surface_id))
                        })
                };
                if let Some((child_conv_id, surface_id)) = child_info {
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.update_conversation_status(
                            surface_id,
                            child_conv_id,
                            status.clone(),
                            ctx,
                        );
                    });
                }
            }
            ctx.emit(OrchestrationEventStreamerEvent::ChildStatusChanged {
                parent_task_id: self.parent_task_id,
                run_id: run_id.to_string(),
                status,
            });
            self.refetch_metadata_if_incomplete(task_id, run_id, ctx);
            self.maybe_request_pane_materialization(task_id, ctx);
            return;
        }
        // Lifecycle for an unknown run is a complete discovery backstop:
        // insert once (emitting ChildSpawned), start/dedupe metadata hydration,
        // and publish the status immediately. This handles a missed or
        // reordered child_agent_started event without a tracker-only ghost.
        self.apply_started(task_id, run_id, ctx);
        let status = conversation_status_from_lifecycle_event_type(kind);
        ctx.emit(OrchestrationEventStreamerEvent::ChildStatusChanged {
            parent_task_id: self.parent_task_id,
            run_id: run_id.to_string(),
            status,
        });
    }

    /// Registers a child created in this process against its existing local
    /// conversation, marking the run already-represented so later
    /// `Started`/`Lifecycle` signals become idempotent status updates rather
    /// than placeholder creation.
    fn register_in_band_child(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // A real conversation already exists for this run, so any speculative
        // discovery fetch is moot.
        self.metadata_fetches.remove(run_id);
        self.in_band_children.insert(task_id);
        if self.children.contains_key(&task_id) {
            return;
        }
        let session_id = self.pending_session_ids.remove(&task_id);
        self.insert_child(
            task_id,
            run_id,
            TrackedChild {
                session_id,
                last_state: None,
                pane_materialized: false,
                // In-band children own a real local conversation; they are
                // never persisted as `is_remote_child` placeholders.
                is_remote_child: false,
            },
            ctx,
        );
        // If the session link already arrived, hydrate the pane immediately.
        self.maybe_request_pane_materialization(task_id, ctx);
    }

    /// Applies a REST seed / restore row. Creates the placeholder if new and
    /// records the latest known state and session id.
    fn apply_seeded(
        &mut self,
        task: AmbientAgentTask,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // The ancestor endpoint includes the parent itself in the response;
        // skip it.
        if task.task_id == self.parent_task_id {
            return;
        }
        let task_id = task.task_id;
        let run_id = task_id.to_string();
        let seed_session_id = task
            .session_id
            .as_deref()
            .and_then(|s| s.parse::<SessionId>().ok());
        let state = task.state.clone();

        self.metadata_fetches.remove(&run_id);

        if let Some(existing) = self.children.get_mut(&task_id) {
            existing.last_state = Some(state);
            if existing.session_id.is_none() {
                existing.session_id = seed_session_id;
            }
            self.maybe_request_pane_materialization(task_id, ctx);
            return;
        }

        // Materialize the placeholder for a run hosted elsewhere, falling back
        // to a session link that arrived before the child was known.
        let session_id = seed_session_id.or_else(|| self.pending_session_ids.remove(&task_id));
        self.insert_child(
            task_id,
            &run_id,
            TrackedChild {
                session_id,
                last_state: Some(state),
                pane_materialized: false,
                is_remote_child: true,
            },
            ctx,
        );
        self.maybe_request_pane_materialization(task_id, ctx);
    }

    /// Handles `run_session_linked`: fills in `session_id` directly (no
    /// metadata fetch) and requests pane materialization immediately. If the
    /// placeholder does not exist yet, the session id is stashed and applied
    /// when the child is created.
    fn apply_session_linked(&mut self, task_id: AmbientAgentTaskId, session_uuid: &str) {
        let Ok(session_id) = session_uuid.parse::<SessionId>() else {
            log::warn!(
                "[orch-tracker] run_session_linked with malformed session_uuid={session_uuid:?} \
                 for task_id={task_id} (parent_task_id={}); dropping",
                self.parent_task_id,
            );
            return;
        };
        match self.children.get_mut(&task_id) {
            Some(child) => {
                if child.session_id.is_none() {
                    child.session_id = Some(session_id);
                }
                // Request the live pane now that the session is known,
                // bypassing the metadata-fetch round-trip.
                self.request_pane_materialization(task_id);
            }
            None => {
                self.pending_session_ids.insert(task_id, session_id);
            }
        }
    }

    /// Refetches metadata while `session_id` is missing or the pane has not
    /// been materialized. No-op once the child is fully hydrated.
    fn refetch_metadata_if_incomplete(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        // In-band children are hydrated by the executor, not the tracker.
        if self.in_band_children.contains(&task_id) {
            return;
        }
        let incomplete = self
            .children
            .get(&task_id)
            .is_some_and(|child| child.session_id.is_none() || !child.pane_materialized);
        if incomplete {
            self.spawn_metadata_fetch(task_id, run_id, ctx);
        }
    }

    /// Requests pane materialization once a `session_id` is known and no pane
    /// has been requested yet.
    fn maybe_request_pane_materialization(
        &mut self,
        task_id: AmbientAgentTaskId,
        _ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        let should_request = self
            .children
            .get(&task_id)
            .is_some_and(|child| child.session_id.is_some() && !child.pane_materialized);
        if should_request {
            self.request_pane_materialization(task_id);
        }
    }

    /// Marks the child's pane as materialized.
    fn request_pane_materialization(&mut self, task_id: AmbientAgentTaskId) {
        if let Some(child) = self.children.get_mut(&task_id) {
            child.pane_materialized = true;
        }
    }

    /// Records a new tracked child, keeping both indices in sync, and emits
    /// the `ChildSpawned` pill-bar broadcast exactly once.
    fn insert_child(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        child: TrackedChild,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        self.children.insert(task_id, child);
        self.children_by_run_id.insert(run_id.to_string(), task_id);
        ctx.emit(OrchestrationEventStreamerEvent::ChildSpawned {
            parent_task_id: self.parent_task_id,
            run_id: run_id.to_string(),
        });
    }

    /// Starts (or dedupes) a metadata fetch for a run. A synchronous cache hit
    /// resolves the placeholder inline via [`Self::apply_seeded`]; a miss
    /// leaves the `metadata_fetches` guard set so redundant dispatches are
    /// suppressed until a later signal re-drives discovery against a warm
    /// cache.
    ///
    /// The guard is inserted before dispatching so both the inline cache-hit
    /// path (which clears it) and the in-flight path behave correctly.
    fn spawn_metadata_fetch(
        &mut self,
        task_id: AmbientAgentTaskId,
        run_id: &str,
        ctx: &mut ModelContext<OrchestrationEventStreamer>,
    ) {
        if !self.metadata_fetches.insert(run_id.to_string()) {
            // A prior dispatch set the guard. If that fetch has since
            // completed the cache is warm and returns the task synchronously;
            // otherwise the shared fetch dedupes the network request.
            #[cfg(not(test))]
            {
                let cached = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                    model.get_or_async_fetch_task_data(&task_id, ctx)
                });
                if let Some(task) = cached {
                    self.metadata_fetches.remove(run_id);
                    self.apply_seeded(task, ctx);
                }
            }
            return;
        }
        log::debug!(
            "[orch-tracker] metadata fetch queued for run_id={run_id} \
             (parent_task_id={})",
            self.parent_task_id,
        );
        #[cfg(test)]
        {
            // Count the dispatch rather than issuing it: unit tests run
            // without the model singleton graph the real fetch needs.
            let _ = (task_id, &ctx);
            self.metadata_fetch_dispatch_count += 1;
        }
        #[cfg(not(test))]
        {
            let cached = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(&task_id, ctx)
            });
            // Cache hit: create or refresh the placeholder immediately. A miss
            // leaves the guard set; the shared fetch populates the cache and a
            // later re-drive completes discovery.
            if let Some(task) = cached {
                self.apply_seeded(task, ctx);
            }
        }
    }

    /// Drops all tracked state for a run (tombstone / kill path).
    fn forget_run(&mut self, run_id: &str) {
        self.metadata_fetches.remove(run_id);
        if let Some(task_id) = self.children_by_run_id.remove(run_id) {
            self.children.remove(&task_id);
            self.in_band_children.remove(&task_id);
            self.pending_session_ids.remove(&task_id);
        }
    }

    /// Test-only: number of metadata-fetch dispatches issued so far.
    #[cfg(test)]
    pub(crate) fn metadata_fetch_dispatch_count(&self) -> usize {
        self.metadata_fetch_dispatch_count
    }

    /// Test-only: whether a metadata fetch is currently in flight for `run_id`.
    #[cfg(test)]
    pub(crate) fn has_in_flight_fetch(&self, run_id: &str) -> bool {
        self.metadata_fetches.contains(run_id)
    }
}

#[cfg(test)]
#[path = "orchestration_child_tracker_tests.rs"]
mod tests;
