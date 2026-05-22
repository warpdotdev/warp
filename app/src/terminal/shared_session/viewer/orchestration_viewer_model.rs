//! Drives the orchestration pill bar in shared session viewers.
//!
//! After the viewer joins a parent ambient-agent session,
//! [`OrchestrationViewerModel`] discovers and tracks the parent's direct
//! children using one of two delivery paths, gated on
//! [`FeatureFlag::OrchestrationViewerStreamer`]:
//!
//! 1. **Streamer-driven (flag ON, default).** The model registers itself as a
//!    viewer-mode consumer with [`OrchestrationEventStreamer`] which:
//!     - Runs a one-shot REST `?ancestor_run_id=` snapshot to seed the
//!       known-child set and the SSE cursor (see spec § `Cold-start seeding`).
//!     - Opens a long-lived ancestor SSE that streams lifecycle events for
//!       every direct child.
//!     - Broadcasts [`OrchestrationEventStreamerEvent::ChildSpawned`] /
//!       [`OrchestrationEventStreamerEvent::ChildStatusChanged`] keyed on
//!       `parent_task_id`. The viewer model translates those events into
//!       local placeholder conversations.
//! 2. **Legacy REST polling (flag OFF).** The model periodically polls
//!    `GET /agent/runs?ancestor_run_id=` (5s / 30s idle) and reconciles the
//!    full child list each cycle. Retained until rollout finishes (see spec
//!    § `PR 3 — Cleanup`).
//!
//! In both paths the model maintains a `run_id → AIConversationId` map so
//! per-child pill metadata, status writes, and `EnsureSharedSessionViewerChildPane`
//! emissions resolve to the right local placeholder. Each viewer pane has
//! its own model and its own placeholder conversations — the streamer is
//! shared, the placeholders are not.
//!
//! Pill clicks navigate via `SwapPaneToConversation` (the existing
//! local-orchestration mechanism), swapping the parent pane for the
//! hidden child pane materialized by `EnsureSharedSessionViewerChildPane`.
use std::collections::HashMap;
use std::time::Duration;

use session_sharing_protocol::common::SessionId;
use warp_core::features::FeatureFlag;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity, WeakViewHandle};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::blocklist::orchestration_event_streamer::{
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::ai::TaskListFilter;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::{Event as TerminalViewEvent, TerminalView};

/// Max child runs per legacy `GET /agent/runs?ancestor_run_id=` page. Used by
/// the polling path only; the streamer-driven path's cold-start seed uses
/// its own constant inside [`OrchestrationEventStreamer`].
const CHILD_DISCOVERY_FETCH_LIMIT: i32 = 100;
/// Polling cadence (legacy path) while at least one child is non-terminal.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Slower polling cadence (legacy path) once every known child is terminal.
/// We don't stop polling entirely because follow-up input can spawn new
/// children.
const STATUS_POLL_INTERVAL_IDLE: Duration = Duration::from_secs(30);

/// Cadence for the per-child `session_id` discovery refetch used by the
/// streamer-driven path. While at least one child is registered with
/// `session_id = None` (or has never had its pane materialized), we
/// periodically refetch its pill metadata via `get_ambient_agent_task`.
///
/// This is needed because lifecycle events (`ChildStatusChanged`) carry
/// only the new status, not the claim-time `session_id`. If the server
/// takes a long time to claim execution for a child (e.g. queued waiting
/// for capacity) and no lifecycle transition fires in between, the
/// `session_id` would never get back-filled by the existing
/// `handle_child_status_changed` refetch hook and the user would stare at
/// "Loading session..." indefinitely. A repro logged a 24s window of
/// session_id pending; a 5s cadence comfortably catches that.
///
/// The timer self-cancels when every tracked child has both
/// `session_id = Some(_)` and `pane_materialization_requested = true`,
/// so cost is bounded by how long children stay pre-claim.
const PENDING_SESSION_ID_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Per-child orchestration metadata, keyed by `AmbientAgentTaskId`.
struct ChildAgentEntry {
    conversation_id: AIConversationId,
    /// Server-side session id; `None` until execution has been claimed.
    session_id: Option<SessionId>,
    /// Most recent state observed (polling path) or the last status applied
    /// (streamer path). Used by the polling path to dedupe status writes;
    /// the streamer path treats every event as authoritative.
    last_state: AmbientAgentTaskState,
    /// True once we've emitted `EnsureSharedSessionViewerChildPane` for
    /// this child, so re-polls / re-events don't spam the event bus.
    pane_materialization_requested: bool,
}

/// Owns child discovery + status tracking for a shared session viewer of an
/// orchestrated session.
pub struct OrchestrationViewerModel {
    /// Orchestrator run id; used as the `ancestor_run_id` filter on both
    /// paths (REST poll under the flag off, broadcast event subscription
    /// filter under the flag on).
    parent_task_id: AmbientAgentTaskId,
    /// Owns the child conversations and anchors the orchestrator lookup.
    terminal_view_id: EntityId,
    /// Used to emit `EnsureSharedSessionViewerChildPane` on the parent's
    /// view when a child becomes joinable.
    terminal_view: WeakViewHandle<TerminalView>,
    /// Local map of children we've materialized as placeholder conversations.
    /// Indexed by `AmbientAgentTaskId` so both paths (REST list response,
    /// streamer broadcast event) can resolve to the same record.
    children: HashMap<AmbientAgentTaskId, ChildAgentEntry>,
    /// Secondary index keyed on the stringified run_id. The streamer's
    /// broadcast events carry `String` run_ids, so this lets the event
    /// handler skip the parse + map lookup that the polling path uses.
    /// Always kept in sync with [`Self::children`].
    children_by_run_id: HashMap<String, AmbientAgentTaskId>,
    /// (Polling path only) Aborted and replaced by every
    /// `schedule_next_poll` so we never have more than one timer chain in
    /// flight. Always `None` when the streamer flag is on.
    polling_handle: Option<SpawnedFutureHandle>,
    /// (Polling path only) Bumped before each fetch; stale responses are
    /// dropped so a slow timer-fired fetch can't clobber a fresher kick.
    fetch_generation: u64,
    /// Set when the most recent fetch returned no children; we wait for
    /// an `AppendedExchange` on the orchestrator before polling again.
    /// Distinct from the in-flight state (`polling_handle = None`).
    idle_due_to_no_children: bool,
    /// Timer chain for the per-child `session_id` discovery refetch.
    /// Scheduled by `maybe_schedule_pending_session_id_poll` whenever any
    /// tracked child is still pending materialization
    /// (`session_id.is_none() || !pane_materialization_requested`).
    /// Reschedules itself on each tick until every child has materialized;
    /// then stays `None` until the next pending child is registered.
    /// Always `None` on the legacy polling path, which gets the same
    /// late-bind behaviour for free via the full-list refetch.
    pending_session_id_poll_handle: Option<SpawnedFutureHandle>,
    /// Test-only: increments each time `spawn_task_metadata_fetch` is
    /// invoked. Lets unit tests assert on the refetch dispatch decisions
    /// in `handle_child_spawned` and `handle_child_status_changed`
    /// without standing up a `MockAIClient`. Behaviorally inert in
    /// non-test builds.
    #[cfg(test)]
    metadata_fetch_dispatch_count: usize,
}

impl Entity for OrchestrationViewerModel {
    type Event = ();
}

impl OrchestrationViewerModel {
    /// Returns the orchestrator's `AmbientAgentTaskId`. Used by
    /// `stop_orchestration_polling` to unregister from the streamer's
    /// viewer-mode entry before the model is dropped.
    pub fn parent_task_id(&self) -> AmbientAgentTaskId {
        self.parent_task_id
    }
    /// Builds a viewer model attached to the given parent shared session.
    ///
    /// When [`FeatureFlag::OrchestrationViewerStreamer`] is ON:
    /// - Registers as a viewer-mode consumer on the shared
    ///   [`OrchestrationEventStreamer`] singleton, which kicks off the
    ///   cold-start REST seed and opens the ancestor SSE.
    /// - Subscribes to `ChildSpawned` / `ChildStatusChanged` broadcasts.
    /// - No polling loop or `AppendedExchange` subscription is created.
    ///
    /// When the flag is OFF (legacy path):
    /// - Subscribes to `BlocklistAIHistoryEvent::AppendedExchange` to kick
    ///   polling out of idle.
    /// - Kicks off the initial children fetch and schedules the first poll.
    pub fn new(
        parent_task_id: AmbientAgentTaskId,
        terminal_view_id: EntityId,
        terminal_view: WeakViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        if FeatureFlag::OrchestrationViewerStreamer.is_enabled() {
            log::info!(
                "[orch-viewer] constructing OrchestrationViewerModel (streamer path) \
                 parent_task_id={parent_task_id} terminal_view_id={terminal_view_id:?}"
            );
            // Streamer-driven path. Subscribe to broadcast events filtered
            // on `parent_task_id`; the streamer handles SSE open/teardown,
            // cold-start seed, and cursor persistence on our behalf.
            let streamer = OrchestrationEventStreamer::handle(ctx);
            ctx.subscribe_to_model(&streamer, move |me, event, ctx| {
                me.handle_streamer_event(event, ctx);
            });
            ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, event, ctx| {
                me.handle_history_event(event, ctx);
            });

            let model = Self {
                parent_task_id,
                terminal_view_id,
                terminal_view,
                children: HashMap::new(),
                children_by_run_id: HashMap::new(),
                polling_handle: None,
                fetch_generation: 0,
                idle_due_to_no_children: false,
                pending_session_id_poll_handle: None,
                #[cfg(test)]
                metadata_fetch_dispatch_count: 0,
            };
            model.register_viewer_mode_consumer_if_possible(ctx);
            return model;
        }

        log::info!(
            "[orch-viewer] constructing OrchestrationViewerModel (legacy poll path) \
             parent_task_id={parent_task_id} terminal_view_id={terminal_view_id:?}"
        );
        // Legacy polling path. Kick to fast cadence on `AppendedExchange` so
        // follow-up input that spawns new children surfaces without waiting
        // for the next 30s idle poll.
        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, event, ctx| {
            me.maybe_kick_polling(event, ctx);
            me.maybe_backfill_parent_agent_ids(event, ctx);
        });

        let mut model = Self {
            parent_task_id,
            terminal_view_id,
            terminal_view,
            children: HashMap::new(),
            children_by_run_id: HashMap::new(),
            polling_handle: None,
            fetch_generation: 0,
            idle_due_to_no_children: false,
            pending_session_id_poll_handle: None,
            #[cfg(test)]
            metadata_fetch_dispatch_count: 0,
        };

        // Each fetch reschedules itself via its response callback.
        model.fetch_children(ctx);
        model
    }

    // ---- Streamer-driven path (FeatureFlag::OrchestrationViewerStreamer on)

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::SetActiveConversation {
                terminal_view_id, ..
            }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                terminal_view_id, ..
            } if *terminal_view_id == self.terminal_view_id => {
                log::debug!(
                    "[orch-viewer] history event for terminal_view_id={terminal_view_id:?} \
                     parent_task_id={} triggers register-retry",
                    self.parent_task_id
                );
                self.register_viewer_mode_consumer_if_possible(ctx);
            }
            _ => {}
        }
    }

    /// Identifies the orchestrator placeholder via fields that are reliably
    /// set on the viewer side under QUALITY-726 semantics:
    ///
    /// - `is_viewing_shared_session()` is stamped by `on_shared_init` on the
    ///   active conversation when the viewer joins a shared session.
    /// - `parent_conversation_id().is_none()` distinguishes the orchestrator
    ///   placeholder from child placeholders, which `register_child` sets up
    ///   via `start_new_child_conversation` (in turn calling
    ///   `set_parent_for_conversation`).
    ///
    /// We deliberately do NOT require `conversation.task_id() == Some(self.parent_task_id)`:
    /// under QUALITY-726, viewer-side parent placeholders are created from
    /// replayed StreamInit events whose `run_id` is intentionally empty
    /// (see `replay_agent_conversations.rs`), so `task_id` on the placeholder
    /// is always `None`. Requiring it makes this check structurally
    /// unsatisfiable and matches what the legacy REST polling path tolerates
    /// in `find_parent_conversation_id`, which just reads
    /// `active_conversation_id(terminal_view_id)` with no further checks.
    fn register_viewer_mode_consumer_if_possible(&self, ctx: &mut ModelContext<Self>) {
        let Some(parent_conversation_id) =
            BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.terminal_view_id)
        else {
            log::warn!(
                "[orch-viewer] no active conversation for terminal_view_id={:?} \
                 parent_task_id={}; viewer-mode consumer not yet registered",
                self.terminal_view_id,
                self.parent_task_id,
            );
            return;
        };
        let (is_viewing_shared_session, has_parent_conv, conv_task_id) =
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&parent_conversation_id)
                .map(|conversation| {
                    (
                        conversation.is_viewing_shared_session(),
                        conversation.parent_conversation_id().is_some(),
                        conversation.task_id(),
                    )
                })
                .unwrap_or((false, false, None));
        let is_parent_placeholder = is_viewing_shared_session && !has_parent_conv;
        if !is_parent_placeholder {
            log::warn!(
                "[orch-viewer] active conversation {:?} for terminal_view_id={:?} is not the \
                 expected parent placeholder (is_viewing_shared_session={is_viewing_shared_session}, \
                 has_parent_conv={has_parent_conv}, conv_task_id={conv_task_id:?}, \
                 expected parent_task_id={}); deferring registration",
                parent_conversation_id,
                self.terminal_view_id,
                self.parent_task_id,
            );
            return;
        }

        let parent_task_id = self.parent_task_id;
        let consumer_id = ctx.model_id();
        log::info!(
            "[orch-viewer] registering viewer-mode consumer parent_task_id={parent_task_id} \
             placeholder_conv_id={parent_conversation_id:?} consumer_id={consumer_id:?} \
             terminal_view_id={:?} (placeholder task_id={conv_task_id:?})",
            self.terminal_view_id,
        );
        OrchestrationEventStreamer::handle(ctx).update(ctx, move |streamer, ctx| {
            streamer.register_viewer_mode_consumer(
                parent_task_id,
                parent_conversation_id,
                consumer_id,
                ctx,
            );
        });
    }

    /// Routes broadcast events from [`OrchestrationEventStreamer`] into the
    /// per-pane placeholder map. Filters on `parent_task_id` so a single
    /// streamer singleton can serve multiple orchestrators concurrently.
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
                log::debug!(
                    "[orch-viewer] received ChildSpawned parent_task_id={parent_task_id} \
                     run_id={run_id:?}"
                );
                self.handle_child_spawned(run_id.clone(), ctx);
            }
            OrchestrationEventStreamerEvent::ChildStatusChanged {
                parent_task_id,
                run_id,
                status,
            } if *parent_task_id == self.parent_task_id => {
                log::debug!(
                    "[orch-viewer] received ChildStatusChanged parent_task_id={parent_task_id} \
                     run_id={run_id:?} status={status:?}"
                );
                self.handle_child_status_changed(run_id, status.clone(), ctx);
            }
            // Events for other orchestrators (or non-viewer-mode variants)
            // are intentionally ignored. Each viewer pane has its own model
            // filtered on its own `parent_task_id`.
            _ => {}
        }
    }

    /// First observation of a child run_id for this orchestrator. Fetches
    /// pill metadata via `get_ambient_agent_task`, creates the child
    /// placeholder conversation, and emits
    /// `EnsureSharedSessionViewerChildPane` if a session id is already
    /// known. If pill metadata can't be fetched (transient error, deleted
    /// task), the event is dropped — the next status event for the same
    /// run_id will re-attempt.
    fn handle_child_spawned(&mut self, run_id: String, ctx: &mut ModelContext<Self>) {
        let Ok(task_id) = run_id.parse::<AmbientAgentTaskId>() else {
            log::warn!("[orch-viewer] ChildSpawned with malformed run_id={run_id:?}; dropping");
            return;
        };
        if self.children.contains_key(&task_id) {
            // Already materialized (e.g. re-registered after reconnect).
            log::debug!(
                "[orch-viewer] ChildSpawned for already-known task_id={task_id} \
                 parent_task_id={}; skipping",
                self.parent_task_id
            );
            return;
        }

        log::info!(
            "[orch-viewer] fetching pill metadata for new child task_id={task_id} \
             parent_task_id={}",
            self.parent_task_id
        );
        self.spawn_task_metadata_fetch(task_id, "ChildSpawned", ctx);
    }

    /// Looks up the placeholder for `run_id` and writes the status through
    /// [`BlocklistAIHistoryModel::update_conversation_status`]. Drops
    /// silently if the local map has no entry for this run_id — the
    /// `ChildSpawned` handler will run next and re-create the placeholder.
    /// `TaskStatusSyncModel::on_conversation_status_updated` already
    /// early-returns for `is_viewing_shared_session()` conversations so
    /// these writes do not echo back to the server.
    ///
    /// If the placeholder is still pending materialization
    /// (`entry.session_id.is_none() || !entry.pane_materialization_requested`),
    /// a metadata refetch is dispatched so the claim-time `session_id`
    /// gets picked up. The polling path gets this for free by re-fetching
    /// the full child list every cycle; the streamer path needs an
    /// explicit hook because lifecycle events carry only status, not
    /// session_id. Once the entry has been materialized (i.e. once a
    /// `register_child` call has seen `session_id: Some(_)`), subsequent
    /// status changes do NOT refetch — they're just status-only writes.
    fn handle_child_status_changed(
        &mut self,
        run_id: &str,
        status: ConversationStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(task_id) = self.children_by_run_id.get(run_id).copied() else {
            // No placeholder yet; the corresponding ChildSpawned handler
            // will create one on first observation.
            log::debug!(
                "[orch-viewer] ChildStatusChanged for unknown run_id={run_id:?} \
                 parent_task_id={}; awaiting matching ChildSpawned",
                self.parent_task_id
            );
            return;
        };
        let Some(entry) = self.children.get(&task_id) else {
            log::debug!(
                "[orch-viewer] ChildStatusChanged: children_by_run_id has run_id={run_id:?} \
                 but children map missing task_id={task_id}; skipping"
            );
            return;
        };
        let conversation_id = entry.conversation_id;
        let needs_metadata_refetch =
            entry.session_id.is_none() || !entry.pane_materialization_requested;
        let terminal_view_id = self.terminal_view_id;
        log::debug!(
            "[orch-viewer] applying child status conversation_id={conversation_id:?} \
             task_id={task_id} status={status:?} needs_metadata_refetch={needs_metadata_refetch}"
        );
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.update_conversation_status(terminal_view_id, conversation_id, status, ctx);
        });

        if needs_metadata_refetch {
            // The placeholder hasn't picked up a `session_id` yet (child
            // was queued/pending at first observation), so the pill click
            // can't materialize a viewer pane. A status transition is our
            // cue that the server has more to tell us about this run —
            // refetch and re-register so the existing-entry branch of
            // `register_child` can flip `pane_materialization_requested`.
            log::info!(
                "[orch-viewer] refetching metadata for child task_id={task_id} \
                 parent_task_id={} (still pending materialization)",
                self.parent_task_id
            );
            self.spawn_task_metadata_fetch(task_id, "ChildStatusChanged", ctx);
        }
    }

    /// Dispatches a `get_ambient_agent_task(task_id)` fetch and routes the
    /// response through `register_child`. Shared between
    /// `handle_child_spawned` (first observation) and
    /// `handle_child_status_changed` (late-bind path for the claim-time
    /// `session_id`). The `trigger` label is logged on fetch failure so
    /// the two callers are distinguishable in the diagnostic stream.
    fn spawn_task_metadata_fetch(
        &mut self,
        task_id: AmbientAgentTaskId,
        trigger: &'static str,
        ctx: &mut ModelContext<Self>,
    ) {
        #[cfg(test)]
        {
            self.metadata_fetch_dispatch_count += 1;
        }
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let parent_task_id = self.parent_task_id;
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |me, result, ctx| {
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

    // ---- Shared child registration (used by both paths) -----------------

    /// Creates the local placeholder conversation for a child task,
    /// records it in the per-pane map, and emits
    /// `EnsureSharedSessionViewerChildPane` if a session id is already
    /// known. Idempotent: a second call for the same `task_id` updates
    /// status / session-id only.
    fn register_child(&mut self, task: AmbientAgentTask, ctx: &mut ModelContext<Self>) {
        // The server-side ancestor endpoint includes the parent itself in
        // the response; skip it.
        if task.task_id == self.parent_task_id {
            log::debug!(
                "[orch-viewer] register_child skipping parent self-entry task_id={}",
                task.task_id
            );
            return;
        }

        let task_id = task.task_id;
        let session_id = task
            .session_id
            .as_deref()
            .and_then(|s| s.parse::<SessionId>().ok());
        let new_state = task.state.clone();
        let conversation_status = conversation_status_from_state(&new_state);

        if let Some(entry) = self.children.get_mut(&task_id) {
            // Existing child: update status if it changed and fill in
            // session id once it becomes available. (Polling path replays
            // every cycle; streamer path can also re-register on reconnect.)
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
            let was_missing_session_id = entry.session_id.is_none();
            if entry.session_id.is_none() {
                entry.session_id = session_id;
            }
            if was_missing_session_id
                && entry.session_id.is_some()
                && !entry.pane_materialization_requested
            {
                let conversation_id = entry.conversation_id;
                let sid = entry.session_id.expect("session_id checked just above");
                entry.pane_materialization_requested = true;
                self.request_child_pane_materialization(conversation_id, sid, ctx);
            }
            // Existing-entry branch may have transitioned the entry to
            // fully-materialized; the helper short-circuits when no children
            // remain pending, so this is safe to call unconditionally.
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

        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                terminal_view_id,
                name,
                parent_conversation_id,
                harness,
                ctx,
            );
            // Suppress server-side status reporting (viewer-side); also
            // disambiguates viewer-spawned children downstream.
            history.set_viewing_shared_session_for_conversation(conversation_id, true);
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                if !fallback_title.is_empty() {
                    conversation.set_fallback_display_title(fallback_title);
                }
            }
            // Stamp the child's `run_id` / `task_id` and populate the
            // `agent_id_to_conversation_id` index so transcript references
            // (received-message, send-message, lifecycle blocks) resolve to
            // this child via `conversation_id_for_agent_id`. Replaces the
            // earlier `set_task_id` call, which set the conversation field
            // but never updated the reverse index.
            history.assign_run_id_for_conversation(
                conversation_id,
                task_id.to_string(),
                Some(task_id),
                terminal_view_id,
                ctx,
            );
            history.update_conversation_status(
                terminal_view_id,
                conversation_id,
                status_for_initial,
                ctx,
            );
            conversation_id
        });

        let pane_materialization_requested = session_id.is_some();
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

        if let Some(sid) = session_id {
            self.request_child_pane_materialization(conversation_id, sid, ctx);
        }

        // A freshly-registered entry with `session_id = None` (the most
        // common case for the streamer path) needs the periodic refetch
        // timer running so the claim-time session_id eventually flows in.
        // No-ops on the legacy polling path (its full-list refetch already
        // covers the same ground).
        self.maybe_schedule_pending_session_id_poll(ctx);
    }

    // ---- Pending-session_id polling (streamer path) -------------------

    /// Returns true iff at least one tracked child is still pending
    /// materialization. Used by [`Self::maybe_schedule_pending_session_id_poll`]
    /// as the timer's continuation gate.
    fn has_pending_session_id_children(&self) -> bool {
        self.children
            .values()
            .any(|entry| entry.session_id.is_none() || !entry.pane_materialization_requested)
    }

    /// Schedules the next [`PENDING_SESSION_ID_POLL_INTERVAL`] tick of the
    /// streamer-path session_id discovery refetch, iff:
    /// - the streamer flag is on (legacy path covers this via its full
    ///   list refetch),
    /// - no timer is currently in flight, and
    /// - at least one tracked child is still pending materialization.
    ///
    /// Safe to call unconditionally; the early-return guards keep it cheap.
    fn maybe_schedule_pending_session_id_poll(&mut self, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::OrchestrationViewerStreamer.is_enabled() {
            return;
        }
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

    /// Body of the pending-session_id timer tick. Dispatches a metadata
    /// refetch for every child whose `session_id` is still unknown or
    /// whose pane has never been materialized, then reschedules the timer
    /// if any pending children remain.
    ///
    /// Each refetch goes through [`Self::spawn_task_metadata_fetch`],
    /// which routes the response into the existing-entry branch of
    /// [`Self::register_child`]. That branch flips
    /// `pane_materialization_requested` and emits
    /// `EnsureSharedSessionViewerChildPane` once the server returns a
    /// claim-time `session_id`.
    fn run_pending_session_id_poll(&mut self, ctx: &mut ModelContext<Self>) {
        let pending: Vec<AmbientAgentTaskId> = self
            .children
            .iter()
            .filter(|(_, entry)| {
                entry.session_id.is_none() || !entry.pane_materialization_requested
            })
            .map(|(task_id, _)| *task_id)
            .collect();

        if pending.is_empty() {
            // The last tick's refetches already filled in session_id and
            // flipped pane_materialization_requested for every tracked
            // child; nothing left to do.
            return;
        }

        log::info!(
            "[orch-viewer-loading] PendingSessionIdPoll: refetching {} pending child(ren) \
             parent_task_id={}",
            pending.len(),
            self.parent_task_id,
        );

        for task_id in pending {
            self.spawn_task_metadata_fetch(task_id, "PendingSessionIdPoll", ctx);
        }

        // Reschedule the next tick. has_pending_session_id_children will
        // return false (and the timer will stop) once every pending
        // entry's refetch callback has completed and flipped
        // pane_materialization_requested via register_child's
        // existing-entry branch.
        self.maybe_schedule_pending_session_id_poll(ctx);
    }

    // ---- Legacy polling path (FeatureFlag::OrchestrationViewerStreamer off)

    /// Schedules the next poll: fast cadence while any child is
    /// non-terminal, slow once all are terminal. Skipped while
    /// [`Self::idle_due_to_no_children`] is set; [`Self::maybe_kick_polling`]
    /// resumes on the next orchestrator `AppendedExchange`.
    fn schedule_next_poll(&mut self, ctx: &mut ModelContext<Self>) {
        // `SpawnedFutureHandle` doesn't abort on drop, so abort
        // explicitly to avoid stacking parallel timer chains.
        if let Some(prior) = self.polling_handle.take() {
            prior.abort();
        }

        // Stay idle until an `AppendedExchange` on the orchestrator wakes
        // us up. `apply_children_fetch` is responsible for setting this
        // flag when an empty descendant list comes back.
        if self.idle_due_to_no_children {
            return;
        }

        let all_terminal = !self.children.is_empty()
            && self
                .children
                .values()
                .all(|child| child.last_state.is_terminal());
        let interval = if all_terminal {
            STATUS_POLL_INTERVAL_IDLE
        } else {
            STATUS_POLL_INTERVAL
        };

        let handle = ctx.spawn(
            async move {
                Timer::after(interval).await;
            },
            |me, _, ctx| me.fetch_children(ctx),
        );
        self.polling_handle = Some(handle);
    }

    /// Tightens polling on `AppendedExchange` during the idle→active
    /// transition, and resumes from `idle_due_to_no_children` on an
    /// orchestrator-scoped exchange. The idle-resume check runs first
    /// because it would otherwise be conflated with the
    /// "fetch in flight" state by the `polling_handle.is_none()` guard.
    fn maybe_kick_polling(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let BlocklistAIHistoryEvent::AppendedExchange {
            conversation_id, ..
        } = event
        else {
            return;
        };
        let conversation_id = *conversation_id;
        let is_orchestrator = self.find_parent_conversation_id(ctx) == Some(conversation_id);

        // Resume from idle-due-to-no-children. Only orchestrator-scoped
        // exchanges count: child events are ignored because we have no
        // tracked children to update yet, and an unrelated conversation's
        // exchange does not imply this orchestrator just spawned a child.
        if self.idle_due_to_no_children {
            if is_orchestrator {
                self.idle_due_to_no_children = false;
                if let Some(prior) = self.polling_handle.take() {
                    prior.abort();
                }
                self.fetch_children(ctx);
            }
            return;
        }

        let all_terminal = !self.children.is_empty()
            && self
                .children
                .values()
                .all(|child| child.last_state.is_terminal());
        if !all_terminal {
            return;
        }
        // `polling_handle = None` here means a kick fetch is already in
        // flight (the idle-due-to-no-children case is handled above);
        // skipping prevents pile-up when exchanges arrive in bursts.
        if self.polling_handle.is_none() {
            return;
        }
        let is_tracked_child = self
            .children
            .values()
            .any(|child| child.conversation_id == conversation_id);
        if !is_orchestrator && !is_tracked_child {
            return;
        }
        if let Some(prior) = self.polling_handle.take() {
            prior.abort();
        }
        self.fetch_children(ctx);
    }

    /// Backfills `parent_agent_id` on viewer-created children once the
    /// orchestrator receives its server token / run id. First-poll
    /// children are created with `parent_agent_id = None` because the
    /// orchestrator hasn't been identified yet; this fixes them up so
    /// `parent_conversation_id` resolution works.
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

    /// Issues a `GET /agent/runs?ancestor_run_id={parent_task_id}` request
    /// and routes the response into [`Self::apply_children_fetch`]. Errors
    /// are logged and ignored; the next poll retries.
    fn fetch_children(&mut self, ctx: &mut ModelContext<Self>) {
        // Bump generation BEFORE dispatch so any in-flight stale fetch
        // is invalidated when its response callback compares.
        self.fetch_generation = self.fetch_generation.wrapping_add(1);
        let fetch_generation = self.fetch_generation;

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let filter = TaskListFilter {
            ancestor_run_id: Some(self.parent_task_id.to_string()),
            ..TaskListFilter::default()
        };
        let parent_task_id = self.parent_task_id;

        ctx.spawn(
            async move {
                ai_client
                    .list_ambient_agent_tasks(CHILD_DISCOVERY_FETCH_LIMIT, filter)
                    .await
            },
            move |me, result, ctx| {
                // Stale fetch: a newer one's already in flight (or applied).
                // The newer fetch owns rescheduling.
                if me.fetch_generation != fetch_generation {
                    return;
                }
                match result {
                    Ok(tasks) => me.apply_children_fetch(tasks, ctx),
                    Err(err) => {
                        log::warn!(
                            "OrchestrationViewerModel: failed to fetch children for {parent_task_id}: {err:#}"
                        );
                    }
                }
                // Always reschedule (even on error) so transient failures
                // don't break the polling loop.
                me.schedule_next_poll(ctx);
            },
        );
    }

    /// Consumes a children list response, registering new children and
    /// updating statuses / session ids on existing ones. Each child goes
    /// through [`Self::register_child`] which is shared with the streamer
    /// path. Also manages the polling-path `idle_due_to_no_children` flag
    /// so an empty descendant list parks the timer chain until the next
    /// orchestrator `AppendedExchange` resumes it.
    fn apply_children_fetch(&mut self, tasks: Vec<AmbientAgentTask>, ctx: &mut ModelContext<Self>) {
        for task in tasks {
            self.register_child(task, ctx);
        }

        // Polling-cost mitigation: if no children are tracked after this
        // fetch, stop scheduling timers. The resume signal is an
        // `AppendedExchange` on the orchestrator (see
        // `maybe_kick_polling`). `schedule_next_poll` honours this flag
        // and bails before spawning a new timer.
        if self.children.is_empty() {
            self.idle_due_to_no_children = true;
            if let Some(prior) = self.polling_handle.take() {
                prior.abort();
            }
        } else {
            self.idle_due_to_no_children = false;
        }
    }

    // ---- Shared helpers ------------------------------------------------

    /// Resolves the orchestrator's local conversation id via the view's
    /// active conversation, which `on_shared_init` sets on first join.
    fn find_parent_conversation_id(&self, ctx: &ModelContext<Self>) -> Option<AIConversationId> {
        BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.terminal_view_id)
    }

    /// Tells the parent's `TerminalView` to materialize a hidden
    /// shared-session viewer pane for this child.
    fn request_child_pane_materialization(
        &self,
        conversation_id: AIConversationId,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let child_task_id = self.children_by_run_id.iter().find_map(|(_, task_id)| {
            self.children
                .get(task_id)
                .filter(|entry| entry.conversation_id == conversation_id)
                .map(|_| *task_id)
        });
        let Some(view) = self.terminal_view.upgrade(ctx) else {
            log::warn!(
                "[orch-viewer] cannot request child pane materialization for conv={conversation_id:?}: \
                 parent terminal view is gone"
            );
            return;
        };
        log::info!(
            "[orch-viewer-loading] dispatching EnsureSharedSessionViewerChildPane \
             conversation_id={conversation_id:?} session_id={session_id} \
             child_task_id={child_task_id:?} parent_task_id={} terminal_view_id={:?}",
            self.parent_task_id,
            self.terminal_view_id,
        );
        view.update(ctx, |_view, ctx| {
            ctx.emit(TerminalViewEvent::EnsureSharedSessionViewerChildPane {
                conversation_id,
                session_id,
            });
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
