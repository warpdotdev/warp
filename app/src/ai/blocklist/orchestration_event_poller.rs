use super::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use super::orchestration_events::{OrchestrationEventService, PendingEvent, PendingEventDetail};
use crate::ai::agent::{
    conversation::{AIConversationId, ConversationStatus},
    ReceivedMessageInput,
};
use crate::ai::agent_events::{
    run_agent_event_driver, AgentEventConsumer, AgentEventConsumerControlFlow,
    AgentEventDriverConfig, MessageHydrator, ServerApiAgentEventSource,
};
use crate::server::server_api::ai::{AIClient, AgentRunEvent};
use crate::server::server_api::{ServerApi, ServerApiProvider};
use anyhow::anyhow;
use async_trait::async_trait;
use futures::channel::mpsc;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

/// Adaptive polling backoff: 1s, 2s, 5s, then 10s max. Resets to 1s when
/// events are found.
const POLL_BACKOFF_STEPS: &[u64] = &[1, 2, 5, 10];
/// Keep each catch-up poll bounded so the event poller can drain backlog without overfetching.
const EVENT_POLL_BATCH_LIMIT: i32 = 100;
/// How often (milliseconds) the drain timer checks for SSE events.
const SSE_DRAIN_INTERVAL_MS: u64 = 500;

/// Tracks messages awaiting server-side delivery confirmation.
struct PendingDeliveryConfirmation {
    message_ids: Vec<String>,
}

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
}

struct SseForwardingConsumer {
    tx: mpsc::UnboundedSender<SseStreamItem>,
    self_run_id: String,
    hydrator: MessageHydrator,
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventConsumer for SseForwardingConsumer {
    async fn on_event(
        &mut self,
        event: AgentRunEvent,
    ) -> anyhow::Result<AgentEventConsumerControlFlow> {
        let fetched_message = self
            .hydrator
            .hydrate_event_for_recipient(&event, &self.self_run_id)
            .await;

        self.tx
            .unbounded_send(SseStreamItem {
                event,
                fetched_message,
            })
            .map_err(|_| anyhow!("SSE event receiver dropped"))?;

        Ok(AgentEventConsumerControlFlow::Continue)
    }
}

/// Async network coordinator for v2 orchestration event delivery.
/// Owns polling, adaptive backoff, event cursors, watched run_ids,
/// lifecycle reporting, delivery confirmation, and self-registration.
///
/// When the `OrchestrationEventPush` feature flag is enabled the poller
/// opens a persistent SSE connection to the server instead of short-polling.
/// SSE retries with exponential backoff on failure.
pub struct OrchestrationEventPoller {
    ai_client: Arc<dyn AIClient>,
    server_api: Arc<ServerApi>,
    watched_run_ids: HashMap<AIConversationId, HashSet<String>>,
    event_cursor: HashMap<AIConversationId, i64>,
    poll_backoff_index: HashMap<AIConversationId, usize>,
    pending_delivery: HashMap<AIConversationId, PendingDeliveryConfirmation>,
    conversation_statuses: HashMap<AIConversationId, ConversationStatus>,
    poll_in_flight: HashSet<AIConversationId>,
    // ---- SSE state ----
    /// Active SSE connections keyed by conversation.
    sse_connections: HashMap<AIConversationId, SseConnectionState>,
    /// Monotonic counter for SSE connection generations. Ensures stale
    /// callbacks from replaced connections are discarded.
    next_sse_generation: u64,
    /// Consecutive failure count for the post-restore `get_ambient_agent_task`
    /// fetch (resets on success). Drives exponential backoff for retries.
    restore_fetch_failures: HashMap<AIConversationId, usize>,
}

pub enum OrchestrationEventPollerEvent {
    // Reserved for future use (e.g., status signals to the controller).
}

impl OrchestrationEventPoller {
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
            watched_run_ids: HashMap::new(),
            event_cursor: HashMap::new(),
            poll_backoff_index: HashMap::new(),
            pending_delivery: HashMap::new(),
            conversation_statuses: HashMap::new(),
            poll_in_flight: HashSet::new(),
            sse_connections: HashMap::new(),
            next_sse_generation: 0,
            restore_fetch_failures: HashMap::new(),
        }
    }

    /// Constructs a poller wired to the supplied (mock) clients instead of
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
            watched_run_ids: HashMap::new(),
            event_cursor: HashMap::new(),
            poll_backoff_index: HashMap::new(),
            pending_delivery: HashMap::new(),
            conversation_statuses: HashMap::new(),
            poll_in_flight: HashSet::new(),
            sse_connections: HashMap::new(),
            next_sse_generation: 0,
            restore_fetch_failures: HashMap::new(),
        }
    }

    /// Registers a run_id to watch for events on a conversation.
    /// Called by the start_agent executor for child run_ids and by
    /// self-registration for the conversation's own token.
    ///
    /// If SSE mode is active for this conversation, the current connection is
    /// torn down and a new one is opened with the updated run_id set.
    pub fn register_watched_run_id(
        &mut self,
        conversation_id: AIConversationId,
        run_id: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.watched_run_ids
            .entry(conversation_id)
            .or_default()
            .insert(run_id);

        // Reconnect SSE with the updated run_ids when a new child is spawned.
        if self.sse_connections.contains_key(&conversation_id) {
            self.reconnect_sse(conversation_id, ctx);
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                is_restored,
                ..
            } => {
                if !*is_restored {
                    self.on_conversation_status_updated(*conversation_id, ctx);
                }
            }
            BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id, ..
            } => self.on_server_token_assigned(*conversation_id, ctx),
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id,
                exchange_id,
                ..
            } => self.on_streaming_exchange_updated(*conversation_id, *exchange_id, ctx),
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                self.watched_run_ids.remove(conversation_id);
                self.event_cursor.remove(conversation_id);
                self.poll_backoff_index.remove(conversation_id);
                self.pending_delivery.remove(conversation_id);
                self.conversation_statuses.remove(conversation_id);
                self.poll_in_flight.remove(conversation_id);
                self.restore_fetch_failures.remove(conversation_id);
                // SSE cleanup
                // task's next send to fail, which terminates the task.
                self.sse_connections.remove(conversation_id);
            }
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
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. } => {}
            BlocklistAIHistoryEvent::RestoredConversations {
                conversation_ids, ..
            } => {
                self.on_restored_conversations(conversation_ids.clone(), ctx);
            }
        }
    }

    /// Handles restoration of conversations on startup (or driver re-attach).
    ///
    /// Re-establishes orchestration event delivery state that is not persisted
    /// directly in memory: watched run_ids, the per-conversation event cursor,
    /// and — for `Success` parents with watched children — the poll/SSE loop.
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
            let (run_id, cursor, status, is_viewer) = {
                let history_model = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history_model.conversation(&conv_id) else {
                    continue;
                };
                let is_viewer = conversation.is_viewing_shared_session();
                let run_id = conversation.run_id();
                let cursor = conversation.last_event_sequence().unwrap_or(0);
                let status = conversation.status().clone();
                (run_id, cursor, status, is_viewer)
            };

            // Shared-session viewers receive updates through session sharing;
            // polling here would re-inject events the session has already
            // processed.
            if is_viewer {
                continue;
            }

            // Initialize the in-memory cursor from the persisted SQLite value.
            // A later server `GET /agent/runs/{run_id}` response may advance
            // it to `max(SQLite, server)` before delivery starts.
            //
            // Note: a status transition arriving in the window before
            // finish_restore_fetch completes may trigger
            // start_event_delivery with only the SQLite cursor. This is
            // acceptable — worst case is one extra batch of duplicate
            // events.
            self.event_cursor.insert(conv_id, cursor);
            self.conversation_statuses.insert(conv_id, status.clone());

            // Register the conversation's own run_id so lifecycle events for
            // self are correctly filtered and the SSE/poll loop has a set
            // of run_ids to open against.
            if let Some(ref own) = run_id {
                self.watched_run_ids
                    .entry(conv_id)
                    .or_default()
                    .insert(own.clone());
            }

            // No run_id means we can't query the server for children or for
            // the canonical cursor. There's nothing more to do here; if a
            // run_id gets assigned later the standard self-registration path
            // will pick it up.
            let Some(run_id) = run_id else {
                self.maybe_start_delivery_after_restore(conv_id, &status, ctx);
                continue;
            };

            let Ok(task_id) = run_id.parse::<crate::ai::ambient_agents::AmbientAgentTaskId>()
            else {
                log::warn!("could not parse run_id {run_id:?} for {conv_id:?}");
                self.maybe_start_delivery_after_restore(conv_id, &status, ctx);
                continue;
            };

            self.spawn_restore_fetch(conv_id, task_id, cursor, ctx);
        }
    }

    /// Issues `GET /agent/runs/{task_id}` and routes the result through
    /// `finish_restore_fetch`. Used both for the initial post-restore fetch
    /// and for backoff-driven retries.
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

    /// Completes the post-restore async fetch by merging the server cursor,
    /// installing the server-reported child run_ids, and — if the parent is
    /// `Success` — starting event delivery. On a server-fetch failure,
    /// schedules a retry with exponential backoff: V2 children always have a
    /// server-side `ai_tasks` row, so the server is the authoritative source
    /// for the watched run_id set, and any local fallback would be incomplete
    /// anyway. Without network connectivity event delivery wouldn't function,
    /// so retrying is the right behavior.
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
                // If the conversation was removed while the fetch was in-flight,
                // the removal handler already cleaned up all poller state. Return
                // early to avoid recreating watched_run_ids for a deleted conversation.
                if !self.event_cursor.contains_key(&conv_id) {
                    self.restore_fetch_failures.remove(&conv_id);
                    return;
                }

                // Reset the retry counter on success.
                self.restore_fetch_failures.remove(&conv_id);

                // Merge the server cursor: use the max of SQLite and server
                // values so we don't re-deliver events the client already
                // acknowledged locally.
                let server_seq = task.last_event_sequence.unwrap_or(0);
                let merged = sqlite_cursor.max(server_seq);
                self.event_cursor.insert(conv_id, merged);

                // The server response includes `children` inline on
                // `AmbientAgentTask`; this is the authoritative set of
                // direct child run_ids for the parent.
                //
                // Insert children and reconnect SSE once if any new run_ids
                // were added and a connection is already open (e.g. because a
                // status transition raced with this fetch and opened SSE with
                // only the parent's own run_id).
                let had_sse = self.sse_connections.contains_key(&conv_id);
                let watched = self.watched_run_ids.entry(conv_id).or_default();
                let mut any_new_children = false;
                for child in task.children {
                    if watched.insert(child) {
                        any_new_children = true;
                    }
                }
                if any_new_children && had_sse {
                    self.reconnect_sse(conv_id, ctx);
                }

                let status = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conv_id)
                    .map(|c| c.status().clone())
                    .unwrap_or(ConversationStatus::Success);
                self.maybe_start_delivery_after_restore(conv_id, &status, ctx);
            }
            Err(err) => {
                log::warn!("Restore: get_agent_run failed for {conv_id:?}: {err:#}; will retry");
                self.start_restore_fetch_retry_timer(conv_id, task_id, sqlite_cursor, ctx);
            }
        }
    }

    /// Schedules a retry of the post-restore `get_ambient_agent_task` fetch
    /// after an exponential backoff. The backoff schedule reuses
    /// `POLL_BACKOFF_STEPS` (1s, 2s, 5s, 10s capped) keyed on a per-conversation
    /// failure counter. The counter resets on success.
    fn start_restore_fetch_retry_timer(
        &mut self,
        conv_id: AIConversationId,
        task_id: crate::ai::ambient_agents::AmbientAgentTaskId,
        sqlite_cursor: i64,
        ctx: &mut ModelContext<Self>,
    ) {
        let failures = self
            .restore_fetch_failures
            .entry(conv_id)
            .and_modify(|c| *c += 1)
            .or_insert(1);
        let step_index = failures.saturating_sub(1).min(POLL_BACKOFF_STEPS.len() - 1);
        let backoff = Duration::from_secs(POLL_BACKOFF_STEPS[step_index]);
        ctx.spawn(
            async move { Timer::after(backoff).await },
            move |me, _, ctx| {
                // The conversation may have been removed in the meantime;
                // if so, drop the retry. Otherwise re-issue the fetch.
                if !me.event_cursor.contains_key(&conv_id) {
                    me.restore_fetch_failures.remove(&conv_id);
                    return;
                }
                me.spawn_restore_fetch(conv_id, task_id, sqlite_cursor, ctx);
            },
        );
    }

    /// Starts event delivery for a restored conversation if the parent is
    /// currently `Success` and has at least one watched run_id. `InProgress`
    /// parents are deferred to `on_conversation_status_updated` once they
    /// next transition to `Success`.
    fn maybe_start_delivery_after_restore(
        &mut self,
        conv_id: AIConversationId,
        status: &ConversationStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        let has_watched = self
            .watched_run_ids
            .get(&conv_id)
            .is_some_and(|w| !w.is_empty());
        if !has_watched {
            return;
        }
        if matches!(status, ConversationStatus::Success) {
            self.start_event_delivery(conv_id, ctx);
        }
    }

    fn on_conversation_status_updated(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let (current_status, previous_status) = {
            let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
            else {
                self.conversation_statuses.remove(&conversation_id);
                return;
            };
            let prev = self
                .conversation_statuses
                .insert(conversation_id, conversation.status().clone());
            (conversation.status().clone(), prev)
        };

        let became_success = matches!(&current_status, ConversationStatus::Success)
            && !matches!(previous_status.as_ref(), Some(ConversationStatus::Success));

        // Trigger event delivery when a conversation with watched run_ids
        // becomes idle. With the event-push flag this opens an SSE stream;
        // otherwise it falls back to the existing polling loop.
        if became_success && self.watched_run_ids.contains_key(&conversation_id) {
            self.start_event_delivery(conversation_id, ctx);
        }
    }

    fn on_server_token_assigned(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let run_id = {
            let Some(conversation) =
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
            else {
                return;
            };
            // Shared session viewers must not poll for events — the actual
            // agent handles event delivery. Polling here would re-inject
            // events the session has already processed.
            if conversation.is_viewing_shared_session() {
                return;
            }
            let Some(run_id) = conversation.run_id() else {
                return;
            };
            run_id
        };
        self.register_watched_run_id(conversation_id, run_id, ctx);
    }

    fn on_streaming_exchange_updated(
        &mut self,
        conversation_id: AIConversationId,
        exchange_id: crate::ai::agent::AIAgentExchangeId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(pending) = self.pending_delivery.get(&conversation_id) else {
            return;
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
        let pending_ids: HashSet<&str> = pending.message_ids.iter().map(String::as_str).collect();
        let mut confirmed_ids = Vec::new();
        if let Some(output) = exchange.output_status.output() {
            for msg in &output.get().messages {
                if let crate::ai::agent::AIAgentOutputMessageType::MessagesReceivedFromAgents {
                    messages,
                } = &msg.message
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
        if let Some(pending) = self.pending_delivery.get_mut(&conversation_id) {
            pending.message_ids.retain(|id| !confirmed_ids.contains(id));
            if pending.message_ids.is_empty() {
                self.pending_delivery.remove(&conversation_id);
            }
        }

        let hydrator = MessageHydrator::new(self.ai_client.clone());
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

    /// Polls the server for events and feeds them into the service queue.
    fn poll_and_inject(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        if self.poll_in_flight.contains(&conversation_id) {
            return;
        }
        let Some(watched) = self.watched_run_ids.get(&conversation_id) else {
            return;
        };
        if watched.is_empty() {
            return;
        }
        self.poll_in_flight.insert(conversation_id);
        let watched: Vec<String> = watched.iter().cloned().collect();
        let cursor = self
            .event_cursor
            .get(&conversation_id)
            .copied()
            .unwrap_or(0);

        let ai_client = self.ai_client.clone();
        let hydrator = MessageHydrator::new(ai_client.clone());

        // Capture own run_id to filter out self-originated lifecycle events.
        let self_run_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.run_id())
            .map(|s| s.to_string())
            .unwrap_or_default();

        struct PollResult {
            events: Vec<crate::server::server_api::ai::AgentRunEvent>,
            fetched_messages: Vec<ReceivedMessageInput>,
        }

        let self_run_id_clone = self_run_id.clone();
        ctx.spawn(
            async move {
                let events = ai_client
                    .poll_agent_events(&watched, cursor, EVENT_POLL_BATCH_LIMIT)
                    .await?;

                let mut fetched_messages = Vec::new();
                for event in &events {
                    if let Some(message) = hydrator
                        .hydrate_event_for_recipient(event, &self_run_id)
                        .await
                    {
                        fetched_messages.push(message);
                    }
                }

                Ok::<_, anyhow::Error>(PollResult {
                    events,
                    fetched_messages,
                })
            },
            move |me, result, ctx| {
                me.poll_in_flight.remove(&conversation_id);
                let self_run_id = self_run_id_clone;
                let poll_result = match result {
                    Ok(r) => r,
                    Err(err) => {
                        log::warn!("V2 event poll failed for {conversation_id:?}: {err:#}");
                        me.start_idle_poll_timer(conversation_id, ctx);
                        return;
                    }
                };

                if poll_result.events.is_empty() {
                    me.start_idle_poll_timer(conversation_id, ctx);
                    return;
                }

                me.handle_poll_result(
                    conversation_id,
                    &self_run_id,
                    cursor,
                    poll_result.events,
                    poll_result.fetched_messages,
                    ctx,
                );
                me.start_idle_poll_timer(conversation_id, ctx);
            },
        );
    }

    fn handle_poll_result(
        &mut self,
        conversation_id: AIConversationId,
        self_run_id: &str,
        previous_cursor: i64,
        events: Vec<crate::server::server_api::ai::AgentRunEvent>,
        messages: Vec<ReceivedMessageInput>,
        ctx: &mut ModelContext<Self>,
    ) {
        let max_seq = events
            .iter()
            .map(|e| e.sequence)
            .max()
            .unwrap_or(previous_cursor);
        self.event_cursor.insert(conversation_id, max_seq);

        // Persist the cursor to SQLite so that after a restart we can resume
        // event delivery from this sequence number without re-delivering
        // events the parent has already acted on.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |model, ctx| {
            model.update_event_sequence(conversation_id, max_seq, ctx);
        });

        // Also persist the cursor to the server so driver / cloud restarts
        // can resume without local SQLite state. Fire-and-forget: log on
        // failure, don't block event delivery. The server persists the
        // cursor on `ai_tasks.last_event_sequence`.
        let own_run_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.run_id());
        if let Some(run_id) = own_run_id {
            // TODO: consider debouncing this server write (see
            // specs/replay-agent-events-on-restore/TECH.md Risks).
            let ai_client = self.ai_client.clone();
            ctx.spawn(
                async move {
                    ai_client
                        .update_event_sequence_on_server(&run_id, max_seq)
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

        // Track message IDs for server-side mark_delivered calls.
        let message_ids: Vec<String> = events
            .iter()
            .filter(|e| e.event_type == "new_message" && e.run_id == self_run_id)
            .filter_map(|e| e.ref_id.clone())
            .collect();
        if !message_ids.is_empty() {
            self.pending_delivery
                .entry(conversation_id)
                .or_insert_with(|| PendingDeliveryConfirmation {
                    message_ids: Vec::new(),
                })
                .message_ids
                .extend(message_ids);
        }

        let lifecycle_events = convert_lifecycle_events(&events, self_run_id);
        if messages.is_empty() && lifecycle_events.is_empty() {
            return;
        }

        // Only reset backoff when events actually produce pending items.
        self.poll_backoff_index.remove(&conversation_id);

        let pending = build_pending_events(messages, lifecycle_events);
        OrchestrationEventService::handle(ctx).update(ctx, |svc, ctx| {
            svc.enqueue_polled_events(conversation_id, pending, ctx);
        });
    }

    /// Starts a background poll timer with adaptive backoff.
    fn start_idle_poll_timer(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.watched_run_ids.contains_key(&conversation_id) {
            return;
        }

        let index = self.poll_backoff_index.entry(conversation_id).or_insert(0);
        let interval_secs = POLL_BACKOFF_STEPS[(*index).min(POLL_BACKOFF_STEPS.len() - 1)];
        *index = (*index + 1).min(POLL_BACKOFF_STEPS.len() - 1);

        ctx.spawn(
            async move { Timer::after(Duration::from_secs(interval_secs)).await },
            move |me, _, ctx| {
                // Re-check that the conversation is still idle before polling.
                let is_success = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .is_some_and(|c| matches!(c.status(), ConversationStatus::Success));
                if is_success && me.watched_run_ids.contains_key(&conversation_id) {
                    me.poll_and_inject(conversation_id, ctx);
                }
            },
        );
    }

    // ---- SSE event-push methods ----

    /// Chooses between SSE and polling based on the feature flag, then starts
    /// the appropriate event delivery loop for the given conversation.
    fn start_event_delivery(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.should_use_sse() {
            if !self.sse_connections.contains_key(&conversation_id) {
                self.start_sse_connection(conversation_id, ctx);
            }
        } else {
            self.poll_and_inject(conversation_id, ctx);
        }
    }

    fn should_use_sse(&self) -> bool {
        FeatureFlag::OrchestrationEventPush.is_enabled()
    }

    /// Opens a long-lived SSE connection for `conversation_id`. Events are
    /// sent through an mpsc channel and drained by a periodic timer.
    fn start_sse_connection(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(watched) = self.watched_run_ids.get(&conversation_id) else {
            return;
        };
        if watched.is_empty() {
            return;
        }

        let watched: Vec<String> = watched.iter().cloned().collect();
        let cursor = self
            .event_cursor
            .get(&conversation_id)
            .copied()
            .unwrap_or(0);

        let server_api = self.server_api.clone();
        let ai_client = self.ai_client.clone();

        let self_run_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.run_id())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let (tx, rx) = mpsc::unbounded();
        let generation = self.next_sse_generation;
        self.next_sse_generation += 1;

        self.sse_connections.insert(
            conversation_id,
            SseConnectionState {
                event_receiver: rx,
                generation,
            },
        );

        log::info!(
            "Opening SSE stream for {conversation_id:?} (gen={generation}, \
             run_ids={watched:?}, since={cursor})"
        );

        let config = AgentEventDriverConfig::retry_forever(watched.clone(), cursor);
        let source = ServerApiAgentEventSource::new(server_api);
        let hydrator = MessageHydrator::new(ai_client);

        ctx.spawn(
            async move {
                let mut consumer = SseForwardingConsumer {
                    tx,
                    self_run_id,
                    hydrator,
                };
                run_agent_event_driver(source, config, &mut consumer).await
            },
            move |me, result, ctx| {
                let is_current = me
                    .sse_connections
                    .get(&conversation_id)
                    .is_some_and(|s| s.generation == generation);
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

        // Start periodic event drain.
        self.start_sse_drain_timer(conversation_id, generation, ctx);
    }

    /// Periodically fires to drain buffered SSE events into the event service.
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
                    .sse_connections
                    .get(&conversation_id)
                    .is_some_and(|s| s.generation == generation);
                if !is_current {
                    return;
                }
                me.drain_sse_events(conversation_id, ctx);
                me.start_sse_drain_timer(conversation_id, generation, ctx);
            },
        );
    }

    /// Drains all buffered SSE events and feeds them through the normal
    /// `handle_poll_result` path.
    fn drain_sse_events(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(sse) = self.sse_connections.get_mut(&conversation_id) else {
            return;
        };

        let cursor = self
            .event_cursor
            .get(&conversation_id)
            .copied()
            .unwrap_or(0);

        let mut events = Vec::new();
        let mut messages = Vec::new();

        while let Ok(Some(item)) = sse.event_receiver.try_next() {
            // Deduplicate: discard events at or below the cursor.
            if item.event.sequence > cursor {
                if let Some(msg) = item.fetched_message {
                    messages.push(msg);
                }
                events.push(item.event);
            }
        }

        if events.is_empty() {
            return;
        }

        let self_run_id = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.run_id())
            .map(|s| s.to_string())
            .unwrap_or_default();

        self.handle_poll_result(conversation_id, &self_run_id, cursor, events, messages, ctx);
    }

    /// Tears down the current SSE connection and opens a new one with the
    /// latest watched run_ids and cursor.
    fn reconnect_sse(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        // Drain buffered events before dropping the channel so we don't
        // discard already-fetched message bodies.
        self.drain_sse_events(conversation_id, ctx);
        self.sse_connections.remove(&conversation_id);

        if self.watched_run_ids.contains_key(&conversation_id) {
            self.start_sse_connection(conversation_id, ctx);
        }
    }
}

impl Entity for OrchestrationEventPoller {
    type Event = OrchestrationEventPollerEvent;
}

impl SingletonEntity for OrchestrationEventPoller {}

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

fn convert_lifecycle_events(
    events: &[crate::server::server_api::ai::AgentRunEvent],
    self_run_id: &str,
) -> Vec<api::AgentEvent> {
    events
        .iter()
        .filter(|e| e.event_type != "new_message" && e.run_id != self_run_id)
        .filter_map(|event| {
            let lifecycle_type = match event.event_type.as_str() {
                // New canonical event types aligned with task states.
                "run_in_progress" => api::LifecycleEventType::InProgress,
                "run_succeeded" => api::LifecycleEventType::Succeeded,
                "run_failed" => api::LifecycleEventType::Failed,
                // Legacy event types mapped to new variants for backward compat.
                #[allow(deprecated)]
                "run_started" => api::LifecycleEventType::InProgress,
                #[allow(deprecated)]
                "run_idle" => api::LifecycleEventType::Succeeded,
                #[allow(deprecated)]
                "run_restarted" => api::LifecycleEventType::InProgress,
                "run_errored" => api::LifecycleEventType::Errored,
                "run_cancelled" => api::LifecycleEventType::Cancelled,
                "run_blocked" => api::LifecycleEventType::Blocked,
                _ => return None,
            };
            let timestamp = parse_occurred_at(&event.occurred_at);
            // TODO: Parse richer detail payloads (reason, error_message) from
            // the server event log once the schema supports them.
            let detail = match lifecycle_type {
                api::LifecycleEventType::Errored => {
                    super::orchestration_events::LifecycleEventDetailPayload {
                        stage: Some(
                            super::orchestration_events::LifecycleEventDetailStage::Runtime,
                        ),
                        reason: event.ref_id.clone(),
                        ..Default::default()
                    }
                }
                _ => super::orchestration_events::LifecycleEventDetailPayload::default(),
            };
            let event_id = Uuid::new_v4().to_string();
            Some(super::orchestration_events::build_lifecycle_event(
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
    messages: Vec<ReceivedMessageInput>,
    lifecycle_events: Vec<api::AgentEvent>,
) -> Vec<PendingEvent> {
    let mut pending = Vec::with_capacity(messages.len() + lifecycle_events.len());
    for msg in &messages {
        pending.push(PendingEvent {
            event_id: msg.message_id.clone(),
            source_agent_id: msg.sender_agent_id.clone(),
            attempt_count: 0,
            detail: PendingEventDetail::Message {
                message_id: msg.message_id.clone(),
                addresses: msg.addresses.clone(),
                subject: msg.subject.clone(),
                message_body: msg.message_body.clone(),
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

#[cfg(test)]
#[path = "orchestration_event_poller_tests.rs"]
mod tests;
