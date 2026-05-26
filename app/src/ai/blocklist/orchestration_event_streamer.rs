use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::channel::mpsc;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{
    Entity, EntityId, GetSingletonModelHandle, ModelContext, SingletonEntity, UpdateModel,
};

use super::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use super::orchestration_events::{
    build_lifecycle_event, LifecycleEventDetailPayload, LifecycleEventDetailStage,
    OrchestrationEventService, PendingEvent, PendingEventDetail,
};
use crate::ai::agent::conversation::{AIAgentHarness, AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentExchangeId, AIAgentOutputMessageType, ReceivedMessageInput};
use crate::ai::agent_events::{
    run_agent_event_driver, AgentEventConsumer, AgentEventConsumerControlFlow,
    AgentEventDriverConfig, AgentEventFilter, AgentMessageEventMetadata, MessageHydrator,
    ServerApiAgentEventSource,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::retry_strategies::is_transient_http_error;
use crate::server::server_api::ai::{AIClient, AgentRunEvent, TaskListFilter};
use crate::server::server_api::{ServerApi, ServerApiProvider};

/// Backoff schedule (seconds) for the post-restore
/// `get_ambient_agent_task` retry on transient errors: 1s, 2s, 5s, then 10s max.
const RESTORE_FETCH_BACKOFF_STEPS: &[u64] = &[1, 2, 5, 10];
/// Slower backoff for permanent HTTP errors (e.g. 404 for deleted runs).
/// Retries still happen in case the error was spurious, but at a much
/// lower frequency to avoid log spam.
const RESTORE_FETCH_PERMANENT_BACKOFF_STEPS: &[u64] = &[30];
/// How often (milliseconds) the drain timer checks for SSE events.
const SSE_DRAIN_INTERVAL_MS: u64 = 500;
/// Cap killed-run tombstones while keeping normal sessions well below the limit.
const MAX_KILLED_RUN_IDS: usize = 1024;
/// Max child runs fetched per cold-start `?ancestor_run_id=` REST seed in
/// viewer mode. Matches the legacy `OrchestrationViewerModel` poller's value
/// (the server caps at 100 anyway).
const VIEWER_MODE_SEED_FETCH_LIMIT: i32 = 100;

/// Per-event item delivered from the SSE background task to the entity.
struct SseStreamItem {
    event: AgentRunEvent,
    fetched_message: Option<ReceivedMessageInput>,
}

/// State for a single active SSE connection.
struct SseConnectionState {
    /// Receives parsed events from the background SSE task.
    event_receiver: mpsc::UnboundedReceiver<SseStreamItem>,
    /// Generation counter; used to discard stale callbacks after reconnect.
    generation: u64,
    /// Abort handle for the spawned SSE driver task, used to cancel on teardown.
    abort_handle: futures::future::AbortHandle,
    /// Snapshot of `watched_run_ids` at the time this connection was opened;
    /// compared in `reevaluate_eligibility` to skip same-set reconnects.
    connected_run_ids: HashSet<String>,
}

struct SseForwardingConsumer {
    tx: mpsc::UnboundedSender<SseStreamItem>,
    self_run_id: String,
    hydrator: MessageHydrator,
    hydrate_new_messages: bool,
}

/// Per-event item delivered from the ancestor SSE background task to the
/// entity. Mirrors [`SseStreamItem`] but does not currently carry a
/// hydrated message: the only ancestor consumer today is viewer mode,
/// which surfaces only lifecycle transitions and so skips message
/// hydration. If/when the ancestor path picks up a non-viewer caller
/// (e.g. a local orchestrator subscribing to its own `ancestor_run_id`
/// stream in lieu of N per-run-ids streams for its children — see
/// [`AncestorForwardingConsumer`]), this struct would gain a hydrated-
/// message field analogous to [`SseStreamItem`].
struct AncestorSseStreamItem {
    event: AgentRunEvent,
}

/// Forwarding consumer used by the ancestor SSE driver. Mirrors
/// [`SseForwardingConsumer`] but does no message hydration: the only
/// current caller is the shared-session viewer's pill bar, which only
/// surfaces lifecycle events.
///
/// Future direction: a local orchestrator could subscribe to its own
/// `ancestor_run_id` stream (one SSE per parent family) instead of
/// having each local child open its own per-run-ids stream. At that
/// point this consumer would gain an opt-in hydrate flag analogous to
/// [`SseForwardingConsumer::hydrate_new_messages`].
struct AncestorForwardingConsumer {
    tx: mpsc::UnboundedSender<AncestorSseStreamItem>,
}

/// State for an ancestor SSE connection. Mirrors [`SseConnectionState`]
/// but parameterised on [`AncestorSseStreamItem`] because the only
/// current caller (viewer mode) does not hydrate messages.
struct AncestorSseConnectionState {
    event_receiver: mpsc::UnboundedReceiver<AncestorSseStreamItem>,
    generation: u64,
    abort_handle: futures::future::AbortHandle,
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventConsumer for AncestorForwardingConsumer {
    async fn on_event(
        &mut self,
        event: AgentRunEvent,
    ) -> anyhow::Result<AgentEventConsumerControlFlow> {
        self.tx
            .unbounded_send(AncestorSseStreamItem { event })
            .map_err(|_| anyhow!("ancestor SSE event receiver dropped"))?;
        Ok(AgentEventConsumerControlFlow::Continue)
    }
}

/// State for a wake-only listener. Unlike `SseConnectionState`, this listener
/// observes the first wake-triggering message event, then asks the controller
/// to cold-start the dormant Claude run so the parent bridge can take over
/// delivery.
struct WakeConnectionState {
    generation: u64,
    task: SpawnedFutureHandle,
}

struct DormantClaudeWakeConsumer {
    run_id: String,
    wake_message: Option<AgentMessageEventMetadata>,
}

impl DormantClaudeWakeConsumer {
    fn new(run_id: String) -> Self {
        Self {
            run_id,
            wake_message: None,
        }
    }
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventConsumer for DormantClaudeWakeConsumer {
    async fn on_event(
        &mut self,
        event: AgentRunEvent,
    ) -> anyhow::Result<AgentEventConsumerControlFlow> {
        if event.run_id != self.run_id {
            return Ok(AgentEventConsumerControlFlow::Continue);
        }
        let Some(wake_message) = AgentMessageEventMetadata::from_event(&event) else {
            return Ok(AgentEventConsumerControlFlow::Continue);
        };

        self.wake_message = Some(wake_message);
        Ok(AgentEventConsumerControlFlow::Stop)
    }
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventConsumer for SseForwardingConsumer {
    async fn on_event(
        &mut self,
        event: AgentRunEvent,
    ) -> anyhow::Result<AgentEventConsumerControlFlow> {
        let fetched_message = if self.hydrate_new_messages {
            self.hydrator
                .hydrate_event_for_recipient(&event, &self.self_run_id)
                .await
        } else {
            None
        };

        self.tx
            .unbounded_send(SseStreamItem {
                event,
                fetched_message,
            })
            .map_err(|_| anyhow!("SSE event receiver dropped"))?;

        Ok(AgentEventConsumerControlFlow::Continue)
    }
}

/// All per-conversation streaming state. Created lazily on first access
/// (via `entry().or_default()`) and dropped when the conversation is
/// removed from the history model.
#[derive(Default)]
struct ConversationStreamState {
    /// Run IDs the SSE filter watches for this conversation. When the
    /// conversation has any orchestration role, this contains its own
    /// `self_run_id` (its inbox — used both for parent→child traffic on
    /// children and child→parent traffic on parents); when it acts as a
    /// parent it additionally contains each registered child run_id.
    watched_run_ids: HashSet<String>,
    /// Last fully handled event sequence number. 0 means "no events
    /// processed yet".
    event_cursor: i64,
    /// Message IDs awaiting server-side `mark_delivered` confirmation,
    /// triggered when the recipient streams a `MessagesReceivedFromAgents`
    /// chunk through `BlocklistAIHistoryEvent::UpdatedStreamingExchange`.
    pending_message_ids: Vec<String>,
    /// Local consumers (terminal pane id for an open agent view, driver
    /// model id for `agent_sdk`) that need events delivered to this
    /// conversation.
    consumers: HashSet<EntityId>,
    /// Execution harness from the task row, when available. Local harness
    /// child conversations are created before they have server conversation
    /// metadata, so this lets us recognize dormant local Claude children
    /// without relying on `ServerAIConversationMetadata`.
    harness: Option<Harness>,
    /// Active SSE connection, if one is open.
    sse_connection: Option<SseConnectionState>,
    /// Active wake-only listener for dormant local Claude children, if one is
    /// open. This is separate from generic SSE because generic delivery would
    /// hydrate messages and advance the shared cursor before Claude's parent
    /// bridge can consume them.
    wake_connection: Option<WakeConnectionState>,
    /// Consecutive `get_ambient_agent_task` failure count for the
    /// post-restore retry loop; resets on success.
    restore_fetch_failures: usize,
}

/// Per-orchestrator SSE stream state. Parallels [`ConversationStreamState`]
/// but keyed on the orchestrator's `AmbientAgentTaskId` instead of on a
/// specific conversation, so a single ancestor-scoped SSE connection can
/// serve every consumer interested in that orchestrator's direct children.
///
/// Today the only consumers are shared-session viewer panes (registered via
/// [`Self::register_viewer_mode_consumer`]), which is why hydration and
/// server-cursor push are absent on the ancestor path. See the note on
/// [`AncestorForwardingConsumer`] for the future direction.
#[derive(Default)]
struct OrchestratorStreamState {
    /// Active viewer-mode consumers. Keyed on the consumer's `EntityId`
    /// (typically the viewer pane's `terminal_view_id`); the value is that
    /// pane's local orchestrator-placeholder `AIConversationId`. Multiple
    /// panes viewing the same orchestrator each register independently and
    /// the entry survives until the last one unregisters.
    consumers: HashMap<EntityId, AIConversationId>,
    /// Direct child `run_id`s observed via the ancestor SSE. Populated as
    /// lifecycle events arrive and seeded from the cold-start REST snapshot.
    /// Used to emit `ChildSpawned` exactly once per child; once a run_id is
    /// in the set, subsequent observations only emit `ChildStatusChanged`.
    known_children: HashSet<String>,
    /// Active ancestor SSE connection, if one is open.
    sse_connection: Option<AncestorSseConnectionState>,
    /// In-memory event cursor for the ancestor stream. Mirrors the
    /// `last_event_sequence` field on each viewer pane's local
    /// orchestrator-placeholder conversation; written through
    /// [`OrchestrationEventStreamer::persist_event_cursor`] on every
    /// advance, which also persists it to SQLite. Initialized from
    /// `max(child.last_event_sequence, locally persisted cursor)` on cold
    /// start.
    event_cursor: i64,
    /// `true` once the cold-start REST seed has been applied. Used to gate
    /// SSE-open until the seed has populated `known_children` and the
    /// cursor, so a replay does not generate spurious `ChildSpawned` events
    /// for already-known children.
    seeded: bool,
}

/// Async network coordinator for v2 orchestration event delivery via SSE.
///
/// Holds at most one long-lived SSE connection per conversation. The
/// streamer opens a connection only when a conversation has both an
/// active local consumer (an open agent view, or an `agent_sdk` driver
/// in CLI / cloud worker processes) and at least one orchestration role
/// in this process — being a child, or having registered child run_ids.
/// Without a local consumer the events would have nowhere to go, so the
/// connection stays closed and the cursor is used to backfill once a
/// consumer registers.
pub struct OrchestrationEventStreamer {
    ai_client: Arc<dyn AIClient>,
    server_api: Arc<ServerApi>,
    /// Per-conversation streaming state.
    streams: HashMap<AIConversationId, ConversationStreamState>,
    /// Per-orchestrator viewer-mode entries (one ancestor SSE per
    /// `parent_task_id`, shared across viewer panes).
    viewer_mode_orchestrators: HashMap<AmbientAgentTaskId, OrchestratorStreamState>,
    /// Monotonic counter for SSE connection generations. Ensures stale
    /// callbacks from replaced connections are discarded.
    next_sse_generation: u64,
    /// Monotonic counter for wake-only listener generations. Ensures stale
    /// callbacks from replaced listeners are discarded.
    next_wake_generation: u64,
    /// Run IDs killed locally; kept briefly to drop late server events.
    killed_run_ids: HashSet<String>,
    killed_run_id_order: VecDeque<String>,
}

pub enum OrchestrationEventStreamerEvent {
    DormantClaudeWakeReady {
        conversation_id: AIConversationId,
        wake_message: AgentMessageEventMetadata,
    },
    /// First time the streamer has seen a particular `run_id` under
    /// `parent_task_id`. Emitted exactly once per child by the viewer-mode
    /// ancestor consumer; viewer-pane subscribers translate it into a local
    /// placeholder conversation.
    #[allow(dead_code)]
    ChildSpawned {
        parent_task_id: AmbientAgentTaskId,
        run_id: String,
    },
    /// Lifecycle transition for a known child under `parent_task_id`.
    /// `status` is the result of the
    /// `LifecycleEventType -> ConversationStatus` mapping helper.
    /// Viewer-pane subscribers look up the child's local placeholder via
    /// their own `run_id -> conversation_id` map and write the status
    /// through `BlocklistAIHistoryModel`.
    #[allow(dead_code)]
    ChildStatusChanged {
        parent_task_id: AmbientAgentTaskId,
        run_id: String,
        status: ConversationStatus,
    },
}

impl OrchestrationEventStreamer {
    fn message_hydrator_for_run_id(&self, run_id: &str) -> MessageHydrator {
        match run_id.parse::<AmbientAgentTaskId>() {
            Ok(task_id) => MessageHydrator::for_task(self.server_api.clone(), task_id),
            Err(_) => MessageHydrator::new(self.ai_client.clone()),
        }
    }

    fn persist_event_cursor(
        &mut self,
        conversation_id: AIConversationId,
        sequence: i64,
        ctx: &mut ModelContext<Self>,
    ) {
        let sequence = self
            .streams
            .get(&conversation_id)
            .map(|stream| stream.event_cursor.max(sequence))
            .unwrap_or(sequence);
        self.streams
            .entry(conversation_id)
            .or_default()
            .event_cursor = sequence;

        BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
            model.update_event_sequence(conversation_id, sequence, ctx);
        });

        // Viewer-mode conversations must NOT push the cursor to the server.
        // The viewer does not own the task; writing to the server's task
        // cursor would interfere with the orchestrator-owner's process
        // resuming its own SSE. The local cursor is still persisted to
        // SQLite above (it represents "highest sequence seen across the
        // parent's direct child set" for viewer-mode conversations, a
        // different scope from owner-side).
        let (own_run_id, is_viewer_mode) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .map(|conversation| {
                (
                    conversation.run_id(),
                    conversation.is_viewing_shared_session(),
                )
            })
            .unwrap_or((None, false));
        if is_viewer_mode {
            return;
        }
        if let Some(run_id) = own_run_id {
            let ai_client = self.ai_client.clone();
            ctx.spawn(
                async move {
                    ai_client
                        .update_event_sequence_on_server(&run_id, sequence)
                        .await
                },
                move |_, result, _| {
                    if let Err(err) = result {
                        log::warn!(
                            "Failed to persist event cursor to server for {conversation_id:?}: {err:#}"
                        );
                    }
                },
            );
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn persist_dormant_claude_wake_cursor(
        &mut self,
        conversation_id: AIConversationId,
        wake_message: &AgentMessageEventMetadata,
        ctx: &mut ModelContext<Self>,
    ) {
        self.persist_event_cursor(conversation_id, wake_message.sequence, ctx);
    }
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let provider = ServerApiProvider::as_ref(ctx);
        let ai_client = provider.get_ai_client();
        let server_api = provider.get();
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });
        Self {
            ai_client,
            server_api,
            streams: HashMap::new(),
            viewer_mode_orchestrators: HashMap::new(),
            next_sse_generation: 0,
            next_wake_generation: 0,
            killed_run_ids: HashSet::new(),
            killed_run_id_order: VecDeque::new(),
        }
    }

    /// Constructs a streamer wired to the supplied (mock) clients instead of
    /// looking them up via `ServerApiProvider`. Lets unit tests inject a
    /// `MockAIClient` while still subscribing to `BlocklistAIHistoryModel`.
    #[cfg(test)]
    pub(super) fn new_with_clients_for_test(
        ai_client: Arc<dyn AIClient>,
        server_api: Arc<ServerApi>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });
        Self {
            ai_client,
            server_api,
            streams: HashMap::new(),
            viewer_mode_orchestrators: HashMap::new(),
            next_sse_generation: 0,
            next_wake_generation: 0,
            killed_run_ids: HashSet::new(),
            killed_run_id_order: VecDeque::new(),
        }
    }

    // ---- Public consumer registry API ---------------------------------

    /// Tombstone a killed run so late SSE events cannot resurrect it.
    pub fn mark_conversation_killed(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(run_id) = self.self_run_id(conversation_id, ctx) else {
            log::info!("mark_conversation_killed: conversation {conversation_id:?} has no run_id");
            return;
        };
        log::info!(
            "Marking orchestration run as killed: conversation_id={conversation_id:?} run_id={run_id}"
        );
        self.remember_killed_run_id(run_id);
    }

    fn remember_killed_run_id(&mut self, run_id: String) {
        if self.killed_run_ids.insert(run_id.clone()) {
            self.killed_run_id_order.push_back(run_id);
        }
        while self.killed_run_ids.len() > MAX_KILLED_RUN_IDS {
            let Some(evicted_run_id) = self.killed_run_id_order.pop_front() else {
                break;
            };
            self.killed_run_ids.remove(&evicted_run_id);
        }
    }

    /// Register a consumer for a conversation. Re-evaluates eligibility
    /// and opens the SSE connection if the conversation is newly
    /// eligible. Idempotent: re-registering an existing consumer is a
    /// no-op for the registry, but still triggers eligibility
    /// re-evaluation (which is itself idempotent).
    pub fn register_consumer(
        &mut self,
        conversation_id: AIConversationId,
        consumer_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let stream = self.streams.entry(conversation_id).or_default();
        stream.consumers.insert(consumer_id);
        // If the server-token event fired before this registration, pick
        // up the now-available child role here.
        self.ensure_self_run_id_watched(conversation_id, ctx);
        self.spawn_task_harness_fetch_if_needed(conversation_id, ctx);
        self.reevaluate_eligibility(conversation_id, ctx);
    }

    /// Unregister a consumer for a conversation. Re-evaluates eligibility
    /// and tears down the SSE connection if the conversation is no longer
    /// eligible (and the conversation is not also in the child role).
    pub fn unregister_consumer(
        &mut self,
        conversation_id: AIConversationId,
        consumer_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.streams
            .get_mut(&conversation_id)
            .map(|s| s.consumers.remove(&consumer_id));
        self.reevaluate_eligibility(conversation_id, ctx);
    }

    /// Registers a run_id to watch for events on a conversation. Called
    /// by the start_agent executor for child run_ids and by the
    /// streamer's own helpers for self_run_id (child / parent inbox).
    pub fn register_watched_run_id(
        &mut self,
        conversation_id: AIConversationId,
        run_id: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let inserted = self
            .streams
            .entry(conversation_id)
            .or_default()
            .watched_run_ids
            .insert(run_id);
        // Adding the first child flips the conversation into the parent
        // role; ensure self_run_id is also watched so child→parent
        // messages match the SSE filter (without it the parent only sees
        // child lifecycle events).
        let self_inserted = self.ensure_self_run_id_watched(conversation_id, ctx);
        if inserted || self_inserted {
            self.reevaluate_eligibility(conversation_id, ctx);
        }
    }

    // ---- Viewer-mode consumer registry (orchestration-viewer-polling) ----

    /// Registers a viewer-mode consumer (a shared-session viewer pane) for
    /// the orchestrator identified by `parent_task_id`. Reference-counted:
    /// multiple viewer panes viewing the same orchestrator share the
    /// per-orchestrator entry and the underlying ancestor SSE. Each consumer
    /// supplies its own pane-local orchestrator-placeholder
    /// `AIConversationId`; the streamer keeps that mapping so the
    /// cursor-persistence path can write through to every viewer pane's
    /// local conversation row.
    ///
    /// Idempotent: re-registering the same `consumer_id` updates the recorded
    /// placeholder conversation but does not double-count the refcount.
    ///
    /// On first registration for a `parent_task_id` (or on any registration
    /// before the cold-start seed has applied) this kicks off the seed REST
    /// fetch; once the seed lands the ancestor SSE opens automatically via
    /// [`Self::start_ancestor_sse_if_seeded`].
    ///
    /// `dead_code` stays suppressed because the only caller is
    /// `OrchestrationViewerModel` (gated on `FeatureFlag::OrchestrationViewerStreamer`);
    /// release builds without that path active never instantiate this method.
    #[allow(dead_code)]
    pub fn register_viewer_mode_consumer(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        orchestrator_placeholder_conv_id: AIConversationId,
        consumer_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let needs_seed;
        {
            let entry = self
                .viewer_mode_orchestrators
                .entry(parent_task_id)
                .or_default();
            entry
                .consumers
                .insert(consumer_id, orchestrator_placeholder_conv_id);
            needs_seed = !entry.seeded && entry.sse_connection.is_none();
        }
        // Hydrate the orchestrator placeholder's persisted cursor into the
        // per-orchestrator entry so a restart-from-disk picks up where the
        // previous session left off without waiting for the REST snapshot.
        let local_cursor = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&orchestrator_placeholder_conv_id)
            .and_then(|conversation| conversation.last_event_sequence())
            .unwrap_or(0);
        if let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id) {
            entry.event_cursor = entry.event_cursor.max(local_cursor);
        }
        if needs_seed {
            self.spawn_ancestor_seed_fetch(parent_task_id, ctx);
        } else {
            // Already seeded: open the SSE immediately if it's not running
            // (e.g. after a transient teardown).
            self.start_ancestor_sse_if_seeded(parent_task_id, ctx);
            self.emit_known_viewer_mode_children(parent_task_id, ctx);
        }
    }

    /// Pair to [`Self::register_viewer_mode_consumer`]. Drops `consumer_id`
    /// from `parent_task_id`'s entry; when the last viewer unregisters, the
    /// entry itself is removed *and* the ancestor SSE is torn down.
    /// Idempotent and safe to call on an already-removed entry — late
    /// `Drop` impls or double-unregisters race against the entry already
    /// being torn down by the last consumer.
    #[allow(dead_code)]
    pub fn unregister_viewer_mode_consumer(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        consumer_id: EntityId,
    ) {
        let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id) else {
            // Idempotent: late `Drop` impl, double-unregister, or unregister
            // after the entry was already torn down. Just no-op.
            return;
        };
        entry.consumers.remove(&consumer_id);
        let remaining = entry.consumers.len();
        if remaining == 0 {
            // Tear down the ancestor SSE for `parent_task_id` (last viewer
            // pane closed) before dropping the bookkeeping entry.
            if let Some(connection) = entry.sse_connection.take() {
                connection.abort_handle.abort();
            }
            self.viewer_mode_orchestrators.remove(&parent_task_id);
        }
    }

    /// Returns `true` iff the streamer's viewer-mode entry for
    /// `parent_task_id` has previously observed `run_id` (either through a
    /// lifecycle event on the ancestor SSE or via the cold-start REST
    /// snapshot). Used by the ancestor consumer's emission logic to emit
    /// `ChildSpawned` exactly once per child; returns `false` if no viewer-
    /// mode entry exists for the parent.
    #[allow(dead_code)]
    pub fn is_known_child(&self, parent_task_id: AmbientAgentTaskId, run_id: &str) -> bool {
        self.viewer_mode_orchestrators
            .get(&parent_task_id)
            .is_some_and(|entry| entry.known_children.contains(run_id))
    }

    /// Returns the placeholder `AIConversationId`s currently registered for
    /// the orchestrator identified by `parent_task_id`. The cursor-persist
    /// path writes to every registered placeholder so each viewer pane
    /// resumes from the same point on the next restart.
    fn viewer_mode_placeholders(
        &self,
        parent_task_id: AmbientAgentTaskId,
    ) -> Vec<AIConversationId> {
        self.viewer_mode_orchestrators
            .get(&parent_task_id)
            .map(|entry| entry.consumers.values().copied().collect())
            .unwrap_or_default()
    }

    fn emit_known_viewer_mode_children(
        &self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let run_ids = self
            .viewer_mode_orchestrators
            .get(&parent_task_id)
            .map(|entry| entry.known_children.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.emit_viewer_mode_child_spawns(parent_task_id, run_ids, ctx);
    }

    fn emit_viewer_mode_child_spawns(
        &self,
        parent_task_id: AmbientAgentTaskId,
        run_ids: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        for run_id in run_ids {
            ctx.emit(OrchestrationEventStreamerEvent::ChildSpawned {
                parent_task_id,
                run_id,
            });
        }
    }

    #[cfg(test)]
    pub(crate) fn viewer_mode_consumer_count_for_test(
        &self,
        parent_task_id: AmbientAgentTaskId,
    ) -> usize {
        self.viewer_mode_orchestrators
            .get(&parent_task_id)
            .map(|entry| entry.consumers.len())
            .unwrap_or(0)
    }

    // ---- Ancestor SSE consumer (viewer-mode driver wiring) -----------

    /// Issues a one-shot `GET /agent/runs?ancestor_run_id={parent}` to
    /// populate the per-orchestrator entry's known-child set and seed the
    /// SSE cursor to `max(child.last_event_sequence across direct children,
    /// locally persisted cursor on the orchestrator placeholder)`. Once the
    /// seed lands, [`Self::finish_ancestor_seed_fetch`] opens the ancestor
    /// SSE with `since=seed`.
    fn spawn_ancestor_seed_fetch(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = self.ai_client.clone();
        let filter = TaskListFilter {
            ancestor_run_id: Some(parent_task_id.to_string()),
            ..TaskListFilter::default()
        };
        ctx.spawn(
            async move {
                ai_client
                    .list_ambient_agent_tasks(VIEWER_MODE_SEED_FETCH_LIMIT, filter)
                    .await
            },
            move |me, result, ctx| {
                me.finish_ancestor_seed_fetch(parent_task_id, result, ctx);
            },
        );
    }

    /// Applies the cold-start REST seed: populates `known_children` from
    /// the response, advances `event_cursor` to `max(server, local)`, marks
    /// the entry seeded, and opens the ancestor SSE. Failures are logged
    /// and retried at registration time (the SSE never opens on a failed
    /// seed, so re-registering kicks the fetch off again).
    fn finish_ancestor_seed_fetch(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        result: anyhow::Result<Vec<crate::ai::ambient_agents::task::AmbientAgentTask>>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.viewer_mode_orchestrators.contains_key(&parent_task_id) {
            log::warn!(
                "[orch-viewer-streamer] ancestor seed fetch completed but viewer-mode entry \
                 for parent_task_id={parent_task_id} is gone; dropping"
            );
            return;
        };
        match result {
            Ok(tasks) => {
                let tasks_received = tasks.len();
                let mut seeded_run_ids = Vec::new();
                {
                    let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id)
                    else {
                        return;
                    };
                    let local_cursor = entry.event_cursor;
                    let mut seed = local_cursor;
                    for task in tasks {
                        if task.task_id == parent_task_id {
                            // The server endpoint may include the parent itself;
                            // skip it — only direct children are tracked.
                            continue;
                        }
                        let run_id = task.task_id.to_string();
                        entry.known_children.insert(run_id.clone());
                        seeded_run_ids.push(run_id);
                        if let Some(seq) = task.last_event_sequence {
                            seed = seed.max(seq);
                        }
                    }
                    entry.event_cursor = seed;
                    entry.seeded = true;
                    log::debug!(
                        "[orch-viewer-streamer] ancestor seed applied for parent_task_id={parent_task_id}: \
                         tasks_received={tasks_received} children_seeded={} known_children_total={} \
                         seed_cursor={seed} local_cursor_before={local_cursor}",
                        seeded_run_ids.len(),
                        entry.known_children.len(),
                    );
                }
                self.emit_viewer_mode_child_spawns(parent_task_id, seeded_run_ids, ctx);
                self.start_ancestor_sse_if_seeded(parent_task_id, ctx);
            }
            Err(err) => {
                log::warn!(
                    "[orch-viewer-streamer] ancestor seed fetch failed for \
                     parent_task_id={parent_task_id}: {err:#}"
                );
                // No retry timer here: the next viewer-mode registration
                // (or an explicit reconnect) re-issues the fetch. Closed
                // orchestrators with no consumers wouldn't benefit from
                // background retries anyway.
            }
        }
    }

    /// Opens the ancestor SSE for `parent_task_id` iff the entry has been
    /// seeded, has at least one viewer-mode consumer, and is not already
    /// connected. Idempotent.
    fn start_ancestor_sse_if_seeded(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self.viewer_mode_orchestrators.get(&parent_task_id) else {
            return;
        };
        if !entry.seeded || entry.consumers.is_empty() || entry.sse_connection.is_some() {
            return;
        }
        let cursor = entry.event_cursor;
        self.start_ancestor_sse(parent_task_id, cursor, ctx);
    }

    /// Opens the ancestor SSE driver for `parent_task_id`. Events are
    /// forwarded through an mpsc channel and drained by a periodic timer
    /// (mirroring the per-conversation pipeline). The driver itself reuses
    /// `run_agent_event_driver::retry_forever` so reconnect / backoff /
    /// proactive recycle (~14m) are inherited from the shared driver.
    fn start_ancestor_sse(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        cursor: i64,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = self.server_api.clone();
        let (tx, rx) = mpsc::unbounded();
        let generation = self.next_sse_generation;
        self.next_sse_generation += 1;

        log::info!(
            "Opening ancestor SSE for parent_task_id={parent_task_id} \
             (gen={generation}, since={cursor})"
        );

        let filter = AgentEventFilter::AncestorRunId(parent_task_id.to_string());
        let config = AgentEventDriverConfig::retry_forever(filter, cursor);
        let source = ServerApiAgentEventSource::new(server_api);

        let handle = ctx.spawn(
            async move {
                let mut consumer = AncestorForwardingConsumer { tx };
                run_agent_event_driver(source, config, &mut consumer).await
            },
            move |me, result, ctx| {
                let is_current = me
                    .viewer_mode_orchestrators
                    .get(&parent_task_id)
                    .and_then(|entry| entry.sse_connection.as_ref())
                    .is_some_and(|c| c.generation == generation);
                if !is_current {
                    return;
                }
                me.drain_ancestor_events(parent_task_id, ctx);
                if let Err(err) = result {
                    log::warn!(
                        "Ancestor SSE driver exited for parent_task_id={parent_task_id} \
                         (gen={generation}): {err:#}"
                    );
                    me.reconnect_ancestor_sse(parent_task_id, ctx);
                }
            },
        );

        if let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id) {
            entry.sse_connection = Some(AncestorSseConnectionState {
                event_receiver: rx,
                generation,
                abort_handle: handle.abort_handle(),
            });
        }

        self.start_ancestor_sse_drain_timer(parent_task_id, generation, ctx);
    }

    /// Periodically fires to drain buffered ancestor SSE events into the
    /// broadcast event dispatch path. Mirrors
    /// [`Self::start_sse_drain_timer`] for the ancestor pipeline.
    fn start_ancestor_sse_drain_timer(
        &self,
        parent_task_id: AmbientAgentTaskId,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(
            async move {
                Timer::after(Duration::from_millis(SSE_DRAIN_INTERVAL_MS)).await;
            },
            move |me, _, ctx| {
                let is_current = me
                    .viewer_mode_orchestrators
                    .get(&parent_task_id)
                    .and_then(|entry| entry.sse_connection.as_ref())
                    .is_some_and(|c| c.generation == generation);
                if !is_current {
                    return;
                }
                me.drain_ancestor_events(parent_task_id, ctx);
                me.start_ancestor_sse_drain_timer(parent_task_id, generation, ctx);
            },
        );
    }

    /// Drains buffered ancestor SSE events, dispatches `ChildSpawned` /
    /// `ChildStatusChanged` broadcast events to viewer subscribers, and
    /// advances the per-orchestrator cursor. `new_message` events arriving
    /// on the ancestor stream are dropped here — viewer-mode subscribers do
    /// not surface inter-agent messages, so the existing
    /// `OrchestrationEventService::enqueue_event_batch` integration is
    /// intentionally bypassed (the streamer's other call sites still feed
    /// the service for owner-side conversations).
    fn drain_ancestor_events(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut events = Vec::new();
        let mut cursor;
        {
            let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id) else {
                return;
            };
            cursor = entry.event_cursor;
            let Some(sse) = entry.sse_connection.as_mut() else {
                return;
            };
            while let Ok(Some(item)) = sse.event_receiver.try_next() {
                if item.event.sequence > cursor {
                    events.push(item.event);
                }
            }
        }
        if events.is_empty() {
            return;
        }

        for event in events {
            // Drop `new_message` events: viewer-mode consumers only surface
            // lifecycle transitions. We still advance the cursor so the
            // SSE replay on reconnect doesn't re-deliver them.
            cursor = cursor.max(event.sequence);
            let Some(lifecycle_type) = lifecycle_event_type_from_wire(event.event_type.as_str())
            else {
                continue;
            };
            let run_id = event.run_id.clone();
            // First observation of a child run_id under this parent: emit
            // `ChildSpawned` exactly once before any status events. The
            // cold-start seed populates `known_children` so already-known
            // children replayed on reconnect do NOT generate a spawn event.
            let is_new_child = self
                .viewer_mode_orchestrators
                .get_mut(&parent_task_id)
                .is_some_and(|entry| entry.known_children.insert(run_id.clone()));
            if is_new_child {
                ctx.emit(OrchestrationEventStreamerEvent::ChildSpawned {
                    parent_task_id,
                    run_id: run_id.clone(),
                });
            }
            let status = conversation_status_from_lifecycle_event_type(lifecycle_type);
            ctx.emit(OrchestrationEventStreamerEvent::ChildStatusChanged {
                parent_task_id,
                run_id,
                status,
            });
        }

        // Persist the advanced cursor to every registered viewer placeholder.
        // The local cursor advances even when all events were dropped (e.g.
        // a batch of `new_message` events) so reconnect-replay stays cheap.
        if let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id) {
            entry.event_cursor = entry.event_cursor.max(cursor);
        }
        for placeholder_conv_id in self.viewer_mode_placeholders(parent_task_id) {
            self.persist_event_cursor(placeholder_conv_id, cursor, ctx);
        }
    }

    /// Tears down and re-opens the ancestor SSE with the current cursor.
    /// Called from the spawn callback when the driver returns an error;
    /// drains buffered events first so we don't lose anything.
    fn reconnect_ancestor_sse(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.drain_ancestor_events(parent_task_id, ctx);
        let cursor;
        {
            let Some(entry) = self.viewer_mode_orchestrators.get_mut(&parent_task_id) else {
                return;
            };
            if let Some(connection) = entry.sse_connection.take() {
                connection.abort_handle.abort();
            }
            if entry.consumers.is_empty() || !entry.seeded {
                return;
            }
            cursor = entry.event_cursor;
        }
        self.start_ancestor_sse(parent_task_id, cursor, ctx);
    }

    // ---- Event subscriptions from BlocklistAIHistoryModel -------------

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id, ..
            } => self.on_server_token_assigned(*conversation_id, ctx),
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id,
                exchange_id,
                ..
            } => self.on_streaming_exchange_updated(*conversation_id, *exchange_id, ctx),
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id,
                run_id,
                ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id,
                run_id,
                ..
            } => {
                self.on_conversation_removed(*conversation_id, run_id.clone(), ctx);
            }
            BlocklistAIHistoryEvent::RestoredConversations {
                conversation_ids, ..
            } => {
                self.on_restored_conversations(conversation_ids.clone(), ctx);
            }
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata {
                conversation_id, ..
            } => self.reevaluate_eligibility(*conversation_id, ctx),
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::ReassignedExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
            | BlocklistAIHistoryEvent::ConversationOwnershipTransferred { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
        }
    }

    fn on_server_token_assigned(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.spawn_task_harness_fetch_if_needed(conversation_id, ctx);
        if self.ensure_self_run_id_watched(conversation_id, ctx) {
            self.reevaluate_eligibility(conversation_id, ctx);
        }
    }

    fn spawn_task_harness_fetch_if_needed(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .streams
            .get(&conversation_id)
            .is_some_and(|stream| stream.harness.is_some())
        {
            return;
        }
        let Some(run_id) = self.self_run_id(conversation_id, ctx) else {
            return;
        };
        let Ok(task_id) = run_id.parse::<crate::ai::ambient_agents::AmbientAgentTaskId>() else {
            return;
        };
        let local_cursor = self
            .streams
            .get(&conversation_id)
            .map(|stream| stream.event_cursor)
            .unwrap_or(0);
        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |me, result, ctx| {
                let task = match result {
                    Ok(task) => task,
                    Err(err) => {
                        log::warn!(
                            "Failed to fetch task harness for {conversation_id:?} task_id={task_id}: {err:#}"
                        );
                        return;
                    }
                };
                if let Some(stream) = me.streams.get_mut(&conversation_id) {
                    stream.harness = agent_task_harness(&task).or(stream.harness);
                    stream.event_cursor =
                        local_cursor.max(task.last_event_sequence.unwrap_or(0));
                }
                me.reevaluate_eligibility(conversation_id, ctx);
            },
        );
    }

    /// Inserts `self_run_id` into the conversation's watched set if the
    /// conversation has any orchestration role (child or parent) and is
    /// not a passive remote-run view. Returns whether anything was
    /// inserted; callers reevaluate eligibility on `true`. Idempotent.
    fn ensure_self_run_id_watched(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &warpui::AppContext,
    ) -> bool {
        let (run_id, is_child) = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation) = history.conversation(&conversation_id) else {
                return false;
            };
            // Passive views of agent runs hosted elsewhere (shared-session
            // viewers and remote-child placeholders) must not subscribe —
            // the actual agent (in another process) is the inbox.
            if conversation.is_viewing_shared_session() || conversation.is_remote_child() {
                return false;
            }
            let Some(run_id) = conversation.run_id() else {
                return false;
            };
            (run_id, conversation.has_parent_agent())
        };

        // Parent role: any watched run_id that isn't this conversation's
        // own self_run_id (i.e. a registered child).
        let is_parent = self
            .streams
            .get(&conversation_id)
            .is_some_and(|s| s.watched_run_ids.iter().any(|id| id != &run_id));

        if !is_child && !is_parent {
            return false;
        }

        self.streams
            .entry(conversation_id)
            .or_default()
            .watched_run_ids
            .insert(run_id)
    }

    fn on_streaming_exchange_updated(
        &mut self,
        conversation_id: AIConversationId,
        exchange_id: AIAgentExchangeId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Snapshot pending IDs so the immutable borrow on `self.streams`
        // doesn't collide with the history model lookup below.
        let pending_ids: HashSet<String> = match self.streams.get(&conversation_id) {
            Some(s) if !s.pending_message_ids.is_empty() => {
                s.pending_message_ids.iter().cloned().collect()
            }
            _ => return,
        };

        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        let Some(exchange) = conversation.exchange_with_id(exchange_id) else {
            return;
        };

        // Check if the exchange output contains any of the messages we're
        // waiting to confirm.
        let mut confirmed_ids = Vec::new();
        if let Some(output) = exchange.output_status.output() {
            for msg in &output.get().messages {
                if let AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } =
                    &msg.message
                {
                    for received in messages {
                        if pending_ids.contains(received.message_id.as_str()) {
                            confirmed_ids.push(received.message_id.clone());
                        }
                    }
                }
            }
        }

        if confirmed_ids.is_empty() {
            return;
        }

        // Remove confirmed messages from pending.
        if let Some(stream) = self.streams.get_mut(&conversation_id) {
            stream
                .pending_message_ids
                .retain(|id| !confirmed_ids.contains(id));
        }

        let hydrator =
            self.message_hydrator_for_run_id(conversation.run_id().as_deref().unwrap_or_default());
        ctx.spawn(
            async move {
                hydrator
                    .mark_messages_delivered_best_effort(confirmed_ids.iter().map(String::as_str))
                    .await
            },
            |_, failures, _| {
                for (message_id, err) in failures {
                    log::warn!("Failed to confirm message delivery for {message_id}: {err:#}");
                }
            },
        );
    }

    /// Cleans up local state for a removed/deleted conversation, then
    /// prunes the removed conversation's run_id from any *other*
    /// tracked conversation's watched set (in case it was a child of
    /// another parent we're still tracking) and re-evaluates eligibility
    /// for those parents.
    ///
    /// `removed_run_id` is the run_id of the conversation as captured by
    /// the history model just before it dropped its in-memory record.
    /// Looking it up here would return `None` because the history model
    /// emits the removal event after removing the record.
    fn on_conversation_removed(
        &mut self,
        conversation_id: AIConversationId,
        removed_run_id: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Drop all per-conversation streamer state in one go (cursor,
        // pending IDs, consumers, watched run_ids, SSE connection).
        // Dropping the SSE receiver causes the driver task's next send
        // to fail and exit; the drain timer's `is_current` check then
        // no-ops on its next tick.
        if let Some(mut stream) = self.streams.remove(&conversation_id) {
            if let Some(connection) = stream.sse_connection.take() {
                connection.abort_handle.abort();
            }
            if let Some(connection) = stream.wake_connection.take() {
                connection.task.abort();
            }
        }

        if let Some(run_id) = removed_run_id.as_deref() {
            let mut affected = Vec::new();
            for (other_id, stream) in self.streams.iter_mut() {
                if stream.watched_run_ids.remove(run_id) {
                    affected.push(*other_id);
                }
            }
            for other_id in affected {
                self.reevaluate_eligibility(other_id, ctx);
            }
        }
    }

    // ---- Restore-on-startup ------------------------------------------

    /// Re-establishes orchestration event delivery state for conversations
    /// loaded from disk on startup. Initializes the in-memory cursor from
    /// the SQLite-persisted `last_event_sequence`, registers each
    /// conversation's own run_id as watched, and (when a run_id is
    /// available) issues `GET /agent/runs/{run_id}` to repopulate child
    /// run_ids and merge the server-side cursor. SSE eligibility is then
    /// re-evaluated through the standard predicate — it opens an SSE iff
    /// a consumer registers and the conversation has a role in the tree.
    fn on_restored_conversations(
        &mut self,
        conversation_ids: Vec<AIConversationId>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Orchestration v2 owns the events endpoints and the cursor model.
        // V1 conversations may carry a run_id but the v2-only event APIs
        // would return spurious 4xx responses, so skip restore entirely
        // when V2 is disabled.
        if !FeatureFlag::OrchestrationV2.is_enabled() {
            return;
        }

        for conv_id in conversation_ids {
            let (run_id, cursor, is_remote_view) = {
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history.conversation(&conv_id) else {
                    continue;
                };
                let is_remote_view =
                    conversation.is_viewing_shared_session() || conversation.is_remote_child();
                let run_id = conversation.run_id();
                let cursor = conversation.last_event_sequence().unwrap_or(0);
                (run_id, cursor, is_remote_view)
            };

            // Passive views of remote runs (shared-session viewers,
            // remote-child placeholders) must not subscribe — the actual
            // agent in another process owns the inbox.
            if is_remote_view {
                continue;
            }

            // Initialize the in-memory cursor from the persisted SQLite
            // value, and register the conversation's own run_id so
            // lifecycle events for self are correctly filtered. A later
            // server `GET /agent/runs/{run_id}` response may advance the
            // cursor to `max(SQLite, server)` before delivery starts.
            let stream = self.streams.entry(conv_id).or_default();
            stream.event_cursor = cursor;
            if let Some(ref own) = run_id {
                stream.watched_run_ids.insert(own.clone());
            }

            // No run_id means we can't query the server for children or
            // for the canonical cursor. Re-evaluate eligibility based on
            // current state; a run_id assigned later flows through
            // `on_server_token_assigned`.
            let Some(run_id) = run_id else {
                self.reevaluate_eligibility(conv_id, ctx);
                continue;
            };

            let Ok(task_id) = run_id.parse::<crate::ai::ambient_agents::AmbientAgentTaskId>()
            else {
                log::warn!("could not parse run_id {run_id:?} for {conv_id:?}");
                self.reevaluate_eligibility(conv_id, ctx);
                continue;
            };

            self.spawn_restore_fetch(conv_id, task_id, cursor, ctx);
        }
    }

    /// Issues `GET /agent/runs/{task_id}` and routes the result through
    /// `finish_restore_fetch`. Used both for the initial post-restore
    /// fetch and for backoff-driven retries.
    fn spawn_restore_fetch(
        &mut self,
        conv_id: AIConversationId,
        task_id: crate::ai::ambient_agents::AmbientAgentTaskId,
        sqlite_cursor: i64,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |me, run_result, ctx| {
                me.finish_restore_fetch(conv_id, task_id, sqlite_cursor, run_result, ctx);
            },
        );
    }

    /// Completes the post-restore async fetch by merging the server cursor
    /// and installing the server-reported child run_ids. On a server-fetch
    /// failure, schedules a retry with exponential backoff: V2 children
    /// always have a server-side `ai_tasks` row, so the server is the
    /// authoritative source for the watched run_id set, and any local
    /// fallback would be incomplete. Without network connectivity event
    /// delivery wouldn't function anyway, so retrying is the right
    /// behavior.
    fn finish_restore_fetch(
        &mut self,
        conv_id: AIConversationId,
        task_id: crate::ai::ambient_agents::AmbientAgentTaskId,
        sqlite_cursor: i64,
        run_result: anyhow::Result<crate::ai::ambient_agents::task::AmbientAgentTask>,
        ctx: &mut ModelContext<Self>,
    ) {
        match run_result {
            Ok(task) => {
                // If the conversation was removed while the fetch was
                // in-flight, the removal handler already cleaned up all
                // streamer state. Return early to avoid recreating
                // state for a deleted conversation.
                let had_sse;
                let any_new_children;
                {
                    let Some(stream) = self.streams.get_mut(&conv_id) else {
                        return;
                    };

                    // Reset the retry counter on success.
                    stream.restore_fetch_failures = 0;
                    stream.harness = agent_task_harness(&task).or(stream.harness);

                    // Merge the server cursor: use the max of SQLite and
                    // server values so we don't re-deliver events the
                    // client already acknowledged locally.
                    let server_seq = task.last_event_sequence.unwrap_or(0);
                    stream.event_cursor = sqlite_cursor.max(server_seq);

                    // Insert any new children. If new run_ids were added
                    // and an SSE connection is already open (e.g. a
                    // status race opened SSE with only the parent's own
                    // run_id), reconnect so the new run_ids are included
                    // in the filter; otherwise re-evaluate eligibility.
                    had_sse = stream.sse_connection.is_some();
                    let mut added = false;
                    for child in task.children {
                        if stream.watched_run_ids.insert(child) {
                            added = true;
                        }
                    }
                    any_new_children = added;
                }
                if any_new_children && had_sse {
                    self.reconnect_sse(conv_id, ctx);
                } else {
                    self.reevaluate_eligibility(conv_id, ctx);
                }
            }
            Err(err) => {
                // If the conversation was removed mid-flight, drop the
                // retry on the floor. Without this guard the retry timer
                // would resurrect a stream entry via `entry().or_default()`
                // and re-issue `get_ambient_agent_task` indefinitely for a
                // deleted conversation.
                if !self.streams.contains_key(&conv_id) {
                    return;
                }
                if is_transient_http_error(&err) {
                    log::warn!(
                        "Restore: get_agent_run failed for {conv_id:?}: {err:#}; will retry"
                    );
                } else {
                    log::warn!("Restore: get_agent_run hit permanent error for {conv_id:?}: {err:#}; retrying with slow backoff");
                }
                self.start_restore_fetch_retry_timer(conv_id, task_id, sqlite_cursor, &err, ctx);
            }
        }
    }

    /// Schedules a retry of the post-restore `get_ambient_agent_task`
    /// fetch after an exponential backoff keyed on a per-conversation
    /// failure counter. Uses a fast schedule (1-10s) for transient errors
    /// and a slow schedule (30s) for permanent HTTP errors. The counter
    /// resets on success.
    fn start_restore_fetch_retry_timer(
        &mut self,
        conv_id: AIConversationId,
        task_id: crate::ai::ambient_agents::AmbientAgentTaskId,
        sqlite_cursor: i64,
        err: &anyhow::Error,
        ctx: &mut ModelContext<Self>,
    ) {
        let backoff_steps = if is_transient_http_error(err) {
            RESTORE_FETCH_BACKOFF_STEPS
        } else {
            RESTORE_FETCH_PERMANENT_BACKOFF_STEPS
        };
        let stream = self.streams.entry(conv_id).or_default();
        stream.restore_fetch_failures += 1;
        let failures = stream.restore_fetch_failures;
        let step_index = failures.saturating_sub(1).min(backoff_steps.len() - 1);
        let backoff = Duration::from_secs(backoff_steps[step_index]);
        ctx.spawn(
            async move { Timer::after(backoff).await },
            move |me, _, ctx| {
                // The conversation may have been removed in the meantime;
                // if so, drop the retry. Otherwise re-issue the fetch.
                if !me.streams.contains_key(&conv_id) {
                    return;
                }
                me.spawn_restore_fetch(conv_id, task_id, sqlite_cursor, ctx);
            },
        );
    }

    // ---- Eligibility predicate ---------------------------------------

    fn self_run_id(
        &self,
        conversation_id: AIConversationId,
        ctx: &warpui::AppContext,
    ) -> Option<String> {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.run_id())
    }

    /// Parent role: the conversation has at least one watched child
    /// run_id (i.e. a watched run_id that is not its own self_run_id).
    fn is_parent_agent_conversation(
        &self,
        conversation_id: AIConversationId,
        ctx: &warpui::AppContext,
    ) -> bool {
        let Some(stream) = self.streams.get(&conversation_id) else {
            return false;
        };
        let self_run_id = self.self_run_id(conversation_id, ctx);
        stream
            .watched_run_ids
            .iter()
            .any(|id| Some(id.as_str()) != self_run_id.as_deref())
    }

    fn has_active_consumer(&self, conversation_id: AIConversationId) -> bool {
        self.streams
            .get(&conversation_id)
            .is_some_and(|s| !s.consumers.is_empty())
    }

    /// True iff this conversation is a passive view of an agent run that
    /// is actually executing in another process — either a shared-session
    /// viewer or a placeholder for a remote child run spawned via
    /// `start_agent` with cloud `execution_mode`. Either way the actual
    /// run lives elsewhere (and that process owns the inbox), so this
    /// process should not open its own SSE for the conversation.
    ///
    /// Shared-session viewer placeholders are excluded *only* when the
    /// `OrchestrationViewerStreamer` flag is off. When the flag is on, the
    /// streamer serves them via the ancestor SSE on its viewer-mode entry
    /// path, so they must not be excluded here. `is_remote_child()` stays
    /// excluded unconditionally — owner-side remote children still receive
    /// events through their parent's existing per-run-ids SSE.
    fn is_remote_run_view(
        &self,
        conversation_id: AIConversationId,
        ctx: &warpui::AppContext,
    ) -> bool {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| {
                if c.is_remote_child() {
                    return true;
                }
                if c.is_viewing_shared_session() {
                    return !FeatureFlag::OrchestrationViewerStreamer.is_enabled();
                }
                false
            })
    }

    fn should_skip_sse_for_dormant_local_claude_child(
        &self,
        conversation_id: AIConversationId,
        ctx: &warpui::AppContext,
    ) -> bool {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return false;
        };
        conversation.is_child_agent_conversation()
            && !conversation.is_remote_child()
            && matches!(conversation.status(), ConversationStatus::Success)
            && (conversation
                .server_metadata()
                .is_some_and(|metadata| metadata.harness == AIAgentHarness::ClaudeCode)
                || self
                    .streams
                    .get(&conversation_id)
                    .and_then(|stream| stream.harness)
                    .is_some_and(|harness| harness == Harness::Claude))
    }

    /// True iff this conversation should currently hold an SSE connection.
    /// A subscription is needed only when there is an active consumer in
    /// this process (an open agent view or an agent_sdk driver) AND the
    /// conversation has a real role to consume events for. Passive views
    /// of agent runs hosted elsewhere are excluded regardless of state.
    fn is_eligible(&self, conversation_id: AIConversationId, ctx: &warpui::AppContext) -> bool {
        if !self.has_active_consumer(conversation_id) {
            return false;
        }
        if self.is_remote_run_view(conversation_id, ctx) {
            return false;
        }
        if self.should_skip_sse_for_dormant_local_claude_child(conversation_id, ctx) {
            log::info!(
                "Skipping generic SSE delivery for dormant local Claude child {conversation_id:?}; parent bridge will deliver wake events"
            );
            return false;
        }
        let has_parent = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| c.has_parent_agent());
        has_parent || self.is_parent_agent_conversation(conversation_id, ctx)
    }

    /// True iff this conversation should hold the wake-only listener used for
    /// dormant local Claude children. Generic SSE intentionally stays closed
    /// for these conversations so it cannot hydrate messages or advance the
    /// server cursor before Claude's parent bridge starts.
    fn is_dormant_claude_wake_listener_eligible(
        &self,
        conversation_id: AIConversationId,
        ctx: &warpui::AppContext,
    ) -> bool {
        self.has_active_consumer(conversation_id)
            && !self.is_remote_run_view(conversation_id, ctx)
            && self.should_skip_sse_for_dormant_local_claude_child(conversation_id, ctx)
            && self.self_run_id(conversation_id, ctx).is_some()
    }

    /// Returns the list of run_ids to subscribe to for `conversation_id`.
    /// Includes both the conversation's own `self_run_id` (when it is a
    /// child) and any registered child run_ids (when the conversation
    /// is a parent). Both contributions live in `watched_run_ids`
    /// already, so this is a straight clone.
    fn run_ids_for_sse(&self, conversation_id: AIConversationId) -> Vec<String> {
        self.streams
            .get(&conversation_id)
            .map(|s| s.watched_run_ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Re-evaluates eligibility and either opens / reconnects or tears
    /// down the SSE connection for the given conversation.
    fn reevaluate_eligibility(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let eligible = self.is_eligible(conversation_id, ctx);
        let connected = self
            .streams
            .get(&conversation_id)
            .is_some_and(|s| s.sse_connection.is_some());

        match (eligible, connected) {
            (true, false) => self.start_sse_connection(conversation_id, ctx),
            (true, true) => {
                // Status / metadata updates fire `reevaluate_eligibility` on
                // every exchange transition; only reconnect when the run-id
                // filter actually changed.
                if self.watched_run_ids_differ_from_connected(conversation_id) {
                    self.reconnect_sse(conversation_id, ctx);
                }
            }
            (false, true) => self.teardown_sse(conversation_id, ctx),
            (false, false) => {}
        }

        let wake_eligible = self.is_dormant_claude_wake_listener_eligible(conversation_id, ctx);
        let wake_connected = self
            .streams
            .get(&conversation_id)
            .is_some_and(|s| s.wake_connection.is_some());

        match (wake_eligible, wake_connected) {
            (true, false) => self.start_dormant_claude_wake_listener(conversation_id, ctx),
            (true, true) => {}
            (false, true) => self.teardown_dormant_claude_wake_listener(conversation_id),
            (false, false) => {}
        }
    }

    /// Opens a wake-only listener for a dormant local Claude child. The
    /// listener observes the child's run_id, stops on the first event, and
    /// emits the triggering message metadata so the controller can prime the
    /// Claude parent bridge before the CLI resumes.
    fn start_dormant_claude_wake_listener(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(run_id) = self.self_run_id(conversation_id, ctx) else {
            return;
        };

        let local_cursor = self
            .streams
            .get(&conversation_id)
            .map(|s| s.event_cursor)
            .unwrap_or(0);
        let generation = self.next_wake_generation;
        self.next_wake_generation += 1;

        log::info!(
            "Opening dormant Claude wake listener for {conversation_id:?} \
             (gen={generation}, run_id={run_id:?}, since={local_cursor})"
        );

        let ai_client = self.ai_client.clone();
        let source = ServerApiAgentEventSource::new(self.server_api.clone());
        let task_run_id = run_id.clone();
        let handle = ctx.spawn(
            async move {
                let since_sequence = resolve_dormant_claude_wake_cursor(
                    ai_client,
                    task_run_id.clone(),
                    local_cursor,
                )
                .await;
                let config = AgentEventDriverConfig::retry_forever_run_ids(
                    vec![task_run_id.clone()],
                    since_sequence,
                );
                let mut consumer = DormantClaudeWakeConsumer::new(task_run_id);
                run_agent_event_driver(source, config, &mut consumer).await?;
                Ok::<_, anyhow::Error>(consumer.wake_message)
            },
            move |me, result, ctx| {
                let is_current = me
                    .streams
                    .get(&conversation_id)
                    .and_then(|s| s.wake_connection.as_ref())
                    .is_some_and(|c| c.generation == generation);
                if !is_current {
                    return;
                }

                me.finish_dormant_claude_wake_listener(conversation_id, generation, result, ctx);
            },
        );

        let stream = self.streams.entry(conversation_id).or_default();
        stream.wake_connection = Some(WakeConnectionState {
            generation,
            task: handle,
        });
    }

    fn finish_dormant_claude_wake_listener(
        &mut self,
        conversation_id: AIConversationId,
        generation: u64,
        result: anyhow::Result<Option<AgentMessageEventMetadata>>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(stream) = self.streams.get_mut(&conversation_id) {
            stream.wake_connection = None;
        }

        match result {
            Ok(Some(wake_message)) => {
                log::info!(
                    "Dormant Claude wake listener observed wake message for \
                     {conversation_id:?} at sequence {} message_id={}",
                    wake_message.sequence,
                    wake_message.message_id
                );
                // Leave the durable cursor untouched here. The controller only
                // persists the wake sequence after Claude wake preparation
                // successfully stages/surfaces the message into the parent
                // bridge, so failed prepares can still replay the event.
                ctx.emit(OrchestrationEventStreamerEvent::DormantClaudeWakeReady {
                    conversation_id,
                    wake_message,
                });
            }
            Ok(None) => {
                log::warn!(
                    "Dormant Claude wake listener stopped for {conversation_id:?} \
                     without observing an event"
                );
                if self.is_dormant_claude_wake_listener_eligible(conversation_id, ctx) {
                    self.start_dormant_claude_wake_listener(conversation_id, ctx);
                }
            }
            Err(err) => {
                log::warn!(
                    "Dormant Claude wake listener failed for {conversation_id:?} \
                     (gen={generation}): {err:#}"
                );
                if self.is_dormant_claude_wake_listener_eligible(conversation_id, ctx) {
                    self.start_dormant_claude_wake_listener(conversation_id, ctx);
                }
            }
        }
    }

    fn teardown_dormant_claude_wake_listener(&mut self, conversation_id: AIConversationId) {
        if let Some(stream) = self.streams.get_mut(&conversation_id) {
            if let Some(connection) = stream.wake_connection.take() {
                log::info!(
                    "Tearing down dormant Claude wake listener for {conversation_id:?} \
                     (gen={})",
                    connection.generation
                );
                connection.task.abort();
            }
        }
    }

    /// Opens a long-lived SSE connection for `conversation_id`. Events
    /// are sent through an mpsc channel and drained by a periodic timer.
    fn start_sse_connection(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let run_ids = self.run_ids_for_sse(conversation_id);
        if run_ids.is_empty() {
            return;
        }

        let cursor = self
            .streams
            .get(&conversation_id)
            .map(|s| s.event_cursor)
            .unwrap_or(0);

        let server_api = self.server_api.clone();

        let self_run_id = self.self_run_id(conversation_id, ctx).unwrap_or_default();

        let (tx, rx) = mpsc::unbounded();
        let generation = self.next_sse_generation;
        self.next_sse_generation += 1;

        log::info!(
            "Opening SSE stream for {conversation_id:?} (gen={generation}, \
             run_ids={run_ids:?}, since={cursor})"
        );

        let config = AgentEventDriverConfig::retry_forever_run_ids(run_ids.clone(), cursor);
        let source = ServerApiAgentEventSource::new(server_api);
        let hydrator = self.message_hydrator_for_run_id(&self_run_id);

        let handle = ctx.spawn(
            async move {
                let mut consumer = SseForwardingConsumer {
                    tx,
                    self_run_id,
                    hydrator,
                    hydrate_new_messages: true,
                };
                run_agent_event_driver(source, config, &mut consumer).await
            },
            move |me, result, ctx| {
                let is_current = me
                    .streams
                    .get(&conversation_id)
                    .and_then(|s| s.sse_connection.as_ref())
                    .is_some_and(|c| c.generation == generation);
                if !is_current {
                    return;
                }

                me.drain_sse_events(conversation_id, ctx);

                if let Err(err) = result {
                    log::warn!(
                        "SSE driver exited for {conversation_id:?} (gen={generation}): {err:#}"
                    );
                    me.reconnect_sse(conversation_id, ctx);
                }
            },
        );

        let connected_run_ids: HashSet<String> = run_ids.iter().cloned().collect();
        let stream = self.streams.entry(conversation_id).or_default();
        stream.sse_connection = Some(SseConnectionState {
            event_receiver: rx,
            generation,
            abort_handle: handle.abort_handle(),
            connected_run_ids,
        });

        // Start periodic event drain.
        self.start_sse_drain_timer(conversation_id, generation, ctx);
    }

    /// True iff the open SSE's recorded run-id set is stale relative to the
    /// conversation's current `watched_run_ids`.
    fn watched_run_ids_differ_from_connected(&self, conversation_id: AIConversationId) -> bool {
        let Some(stream) = self.streams.get(&conversation_id) else {
            return false;
        };
        let Some(connection) = stream.sse_connection.as_ref() else {
            return false;
        };
        stream.watched_run_ids != connection.connected_run_ids
    }

    /// Periodically fires to drain buffered SSE events into the event
    /// service.
    fn start_sse_drain_timer(
        &self,
        conversation_id: AIConversationId,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(
            async move {
                Timer::after(Duration::from_millis(SSE_DRAIN_INTERVAL_MS)).await;
            },
            move |me, _, ctx| {
                let is_current = me
                    .streams
                    .get(&conversation_id)
                    .and_then(|s| s.sse_connection.as_ref())
                    .is_some_and(|c| c.generation == generation);
                if !is_current {
                    return;
                }
                me.drain_sse_events(conversation_id, ctx);
                me.start_sse_drain_timer(conversation_id, generation, ctx);
            },
        );
    }

    /// Drains all buffered SSE events and feeds them through the
    /// `handle_event_batch` sink.
    fn drain_sse_events(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let cursor;
        let mut events = Vec::new();
        let mut messages = Vec::new();
        {
            let Some(stream) = self.streams.get_mut(&conversation_id) else {
                return;
            };
            cursor = stream.event_cursor;
            let Some(sse) = stream.sse_connection.as_mut() else {
                return;
            };

            while let Ok(Some(item)) = sse.event_receiver.try_next() {
                // Deduplicate: discard events at or below the cursor.
                if item.event.sequence > cursor {
                    if let Some(msg) = item.fetched_message {
                        messages.push(msg);
                    }
                    events.push(item.event);
                }
            }
        }

        if events.is_empty() {
            return;
        }

        let self_run_id = self.self_run_id(conversation_id, ctx).unwrap_or_default();

        self.handle_event_batch(conversation_id, &self_run_id, cursor, events, messages, ctx);
    }

    /// Feeds a batch of fetched events through the OrchestrationEventService,
    /// updating the in-memory and persisted cursors and tracking message
    /// IDs awaiting delivery confirmation.
    fn handle_event_batch(
        &mut self,
        conversation_id: AIConversationId,
        self_run_id: &str,
        previous_cursor: i64,
        mut events: Vec<AgentRunEvent>,
        mut messages: Vec<ReceivedMessageInput>,
        ctx: &mut ModelContext<Self>,
    ) {
        let max_seq = events
            .iter()
            .map(|e| e.sequence)
            .max()
            .unwrap_or(previous_cursor);
        // Advance the cursor before filtering so dropped killed-run events
        // are not replayed later.
        self.persist_event_cursor(conversation_id, max_seq, ctx);

        if !self.killed_run_ids.is_empty() {
            let dropped_message_ids: HashSet<String> = events
                .iter()
                .filter(|event| self.killed_run_ids.contains(&event.run_id))
                .filter_map(|event| event.ref_id.clone())
                .collect();
            let event_count_before = events.len();
            events.retain(|event| !self.killed_run_ids.contains(&event.run_id));
            messages.retain(|message| {
                !dropped_message_ids.contains(&message.message_id)
                    && !self.killed_run_ids.contains(&message.sender_agent_id)
            });
            let dropped_event_count = event_count_before - events.len();
            if dropped_event_count > 0 {
                log::info!(
                    "Dropped {dropped_event_count} orchestration events for killed run IDs while handling {conversation_id:?}"
                );
            }
        }
        // Track message IDs for server-side mark_delivered calls.
        let message_ids: Vec<String> = messages
            .iter()
            .map(|message| message.message_id.clone())
            .collect();
        if !message_ids.is_empty() {
            self.streams
                .entry(conversation_id)
                .or_default()
                .pending_message_ids
                .extend(message_ids);
        }

        let lifecycle_events = convert_lifecycle_events(&events, self_run_id);
        if messages.is_empty() && lifecycle_events.is_empty() {
            return;
        }

        let pending = build_pending_events(&events, messages, lifecycle_events);
        OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
            svc.enqueue_event_batch(conversation_id, pending, ctx);
        });
    }

    /// Tears down the current SSE connection and (if still eligible)
    /// opens a new one with the latest run_ids list and cursor.
    fn reconnect_sse(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        // Drain buffered events before dropping the channel so we don't
        // discard already-fetched message bodies.
        self.drain_sse_events(conversation_id, ctx);
        if let Some(stream) = self.streams.get_mut(&conversation_id) {
            if let Some(connection) = stream.sse_connection.take() {
                connection.abort_handle.abort();
            }
        }

        if self.is_eligible(conversation_id, ctx) {
            self.start_sse_connection(conversation_id, ctx);
        }
    }

    /// Drops the SSE connection for a no-longer-eligible conversation.
    /// Leaves `watched_run_ids` and `consumers` alone — those reflect
    /// external state and are pruned through their own paths.
    fn teardown_sse(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        // Drain anything buffered so we don't lose hydrated messages.
        self.drain_sse_events(conversation_id, ctx);
        if let Some(stream) = self.streams.get_mut(&conversation_id) {
            if let Some(connection) = stream.sse_connection.take() {
                log::info!("Tearing down SSE for {conversation_id:?} (no longer eligible)");
                connection.abort_handle.abort();
            }
        }
    }
}

impl Entity for OrchestrationEventStreamer {
    type Event = OrchestrationEventStreamerEvent;
}

impl SingletonEntity for OrchestrationEventStreamer {}

async fn resolve_dormant_claude_wake_cursor(
    ai_client: Arc<dyn AIClient>,
    run_id: String,
    local_cursor: i64,
) -> i64 {
    let Ok(task_id) = run_id.parse::<AmbientAgentTaskId>() else {
        return local_cursor;
    };

    match ai_client.get_ambient_agent_task(&task_id).await {
        Ok(task) => local_cursor.max(task.last_event_sequence.unwrap_or(0)),
        Err(err) => {
            log::warn!(
                "Failed to read server cursor for dormant Claude wake listener \
                 run {run_id}: {err:#}; using local cursor {local_cursor}"
            );
            local_cursor
        }
    }
}

fn agent_task_harness(task: &crate::ai::ambient_agents::task::AmbientAgentTask) -> Option<Harness> {
    task.agent_config_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.harness.as_ref())
        .map(|config| config.harness_type)
        .filter(|harness| *harness != Harness::Unknown)
}

fn parse_occurred_at(s: &str) -> prost_types::Timestamp {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        })
        .unwrap_or_else(|_| {
            let now = chrono::Utc::now();
            prost_types::Timestamp {
                seconds: now.timestamp(),
                nanos: now.timestamp_subsec_nanos() as i32,
            }
        })
}

/// Maps an `api::LifecycleEventType` (server-sourced) to the
/// `ConversationStatus` used by the shared-session viewer's orchestration
/// pill bar.
///
/// This mirrors the collapsing rules in
/// `orchestration_viewer_model::conversation_status_from_state`
/// (`AmbientAgentTaskState` → `ConversationStatus`): working states all
/// collapse to `InProgress`, terminals map one-for-one, and the
/// forward-compat catch-all (`Unspecified`) maps to `Error` to match how
/// `AmbientAgentTaskState::Unknown` is treated today.
///
/// `Blocked` is mapped with an empty `blocked_action`: the wire event does
/// not currently carry a `blocked_action` payload, matching the REST path.
#[allow(deprecated)]
pub(super) fn conversation_status_from_lifecycle_event_type(
    event_type: api::LifecycleEventType,
) -> ConversationStatus {
    match event_type {
        // Working states. Legacy `Started` and `Restarted` collapse to
        // `InProgress`, matching `convert_lifecycle_events` above and the
        // viewer's `AmbientAgentTaskState::{Queued,Pending,Claimed,InProgress}`
        // → `InProgress` rule.
        api::LifecycleEventType::InProgress
        | api::LifecycleEventType::Started
        | api::LifecycleEventType::Restarted => ConversationStatus::InProgress,
        // Terminals.
        api::LifecycleEventType::Succeeded | api::LifecycleEventType::Idle => {
            ConversationStatus::Success
        }
        // Both `Failed` and `Errored` collapse to `Error`, matching the
        // viewer's `AmbientAgentTaskState::{Failed,Error}` rule.
        api::LifecycleEventType::Failed | api::LifecycleEventType::Errored => {
            ConversationStatus::Error
        }
        api::LifecycleEventType::Cancelled => ConversationStatus::Cancelled,
        api::LifecycleEventType::Blocked => ConversationStatus::Blocked {
            blocked_action: String::new(),
        },
        // Forward-compat catch-all: matches the viewer's
        // `AmbientAgentTaskState::Unknown` → `Error` behaviour.
        api::LifecycleEventType::Unspecified => ConversationStatus::Error,
    }
}

/// Maps a wire `event_type` string from the server's `AgentRunEvent`
/// payload onto the corresponding [`api::LifecycleEventType`]. Returns
/// `None` for `new_message` (a message event, handled separately) and for
/// unrecognised event types (forward-compat).
///
/// Shared by [`OrchestrationEventStreamer::drain_ancestor_events`] (which
/// dispatches events to viewer-mode subscribers) and
/// [`convert_lifecycle_events`] (which builds owner-side
/// `PendingEventDetail::Lifecycle` items). Keeping the wire-string table
/// in one place ensures both paths agree on which legacy variants are
/// recognised.
fn lifecycle_event_type_from_wire(event_type: &str) -> Option<api::LifecycleEventType> {
    match event_type {
        // New canonical event types aligned with task states.
        "run_in_progress" => Some(api::LifecycleEventType::InProgress),
        "run_succeeded" => Some(api::LifecycleEventType::Succeeded),
        "run_failed" => Some(api::LifecycleEventType::Failed),
        // Legacy event types mapped to new variants for backward compat.
        #[allow(deprecated)]
        "run_started" => Some(api::LifecycleEventType::InProgress),
        #[allow(deprecated)]
        "run_idle" => Some(api::LifecycleEventType::Succeeded),
        #[allow(deprecated)]
        "run_restarted" => Some(api::LifecycleEventType::InProgress),
        "run_errored" => Some(api::LifecycleEventType::Errored),
        "run_cancelled" => Some(api::LifecycleEventType::Cancelled),
        "run_blocked" => Some(api::LifecycleEventType::Blocked),
        _ => None,
    }
}

fn convert_lifecycle_events(events: &[AgentRunEvent], self_run_id: &str) -> Vec<api::AgentEvent> {
    events
        .iter()
        .filter(|e| e.event_type != "new_message" && e.run_id != self_run_id)
        .filter_map(|event| {
            let lifecycle_type = lifecycle_event_type_from_wire(event.event_type.as_str())?;
            let timestamp = parse_occurred_at(&event.occurred_at);
            // TODO: Parse richer detail payloads (reason, error_message) from
            // the server event log once the schema supports them.
            let detail = match lifecycle_type {
                api::LifecycleEventType::Errored => LifecycleEventDetailPayload {
                    stage: Some(LifecycleEventDetailStage::Runtime),
                    reason: event.ref_id.clone(),
                    ..Default::default()
                },
                _ => LifecycleEventDetailPayload::default(),
            };
            let event_id = Uuid::new_v4().to_string();
            Some(build_lifecycle_event(
                event_id,
                event.run_id.clone(),
                lifecycle_type,
                timestamp,
                &detail,
            ))
        })
        .collect()
}

fn build_pending_events(
    events: &[AgentRunEvent],
    messages: Vec<ReceivedMessageInput>,
    lifecycle_events: Vec<api::AgentEvent>,
) -> Vec<PendingEvent> {
    let mut pending = Vec::with_capacity(messages.len() + lifecycle_events.len());
    for msg in &messages {
        let metadata = events
            .iter()
            .find(|event| {
                event.event_type == "new_message"
                    && event.ref_id.as_deref() == Some(msg.message_id.as_str())
            })
            .map(|event| (event.sequence, event.occurred_at.clone()));
        let (sequence, occurred_at) =
            metadata.unwrap_or_else(|| (0, chrono::Utc::now().to_rfc3339()));
        pending.push(PendingEvent {
            event_id: msg.message_id.clone(),
            source_agent_id: msg.sender_agent_id.clone(),
            attempt_count: 0,
            detail: PendingEventDetail::Message {
                sequence,
                message_id: msg.message_id.clone(),
                addresses: msg.addresses.clone(),
                subject: msg.subject.clone(),
                message_body: msg.message_body.clone(),
                occurred_at,
            },
        });
    }
    for event in lifecycle_events {
        pending.push(PendingEvent {
            event_id: event.event_id.clone(),
            source_agent_id: String::new(),
            attempt_count: 0,
            detail: PendingEventDetail::Lifecycle { event },
        });
    }
    pending
}

// ---- Free-function consumer registration helpers ---------------------
//
// Wrap the feature-flag check + singleton handle update so call sites
// in `ActiveAgentViewsModel` and the agent_sdk driver don't have to
// repeat the boilerplate. The generic bound covers both
// `&mut AppContext` and `&mut ModelContext<T>` / `&mut ViewContext<T>`.
//
// Consumers are identified by an `EntityId` — the terminal pane's id
// for an agent view, the driver model's id for `agent_sdk`. The
// streamer never branches on consumer kind, so a single pair of helpers
// covers both call sites.

/// Registers a consumer of orchestration agent events for
/// `conversation_id`. No-op when `OrchestrationV2` is disabled.
pub fn register_agent_event_consumer<C>(
    conversation_id: AIConversationId,
    consumer_id: EntityId,
    ctx: &mut C,
) where
    C: GetSingletonModelHandle + UpdateModel,
{
    if !FeatureFlag::OrchestrationV2.is_enabled() {
        return;
    }
    OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
        streamer.register_consumer(conversation_id, consumer_id, ctx);
    });
}

/// Pair to [`register_agent_event_consumer`].
pub fn unregister_agent_event_consumer<C>(
    conversation_id: AIConversationId,
    consumer_id: EntityId,
    ctx: &mut C,
) where
    C: GetSingletonModelHandle + UpdateModel,
{
    if !FeatureFlag::OrchestrationV2.is_enabled() {
        return;
    }
    OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
        streamer.unregister_consumer(conversation_id, consumer_id, ctx);
    });
}

#[cfg(test)]
#[path = "orchestration_event_streamer_tests.rs"]
mod tests;
