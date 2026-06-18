# Orchestration Conversation Restore — Tech Spec

## Context
See `PRODUCT.md` for user-visible behavior.

### Current restore path
On startup, `AgentConversation` records are read from SQLite and converted to `AIConversation` via `new_restored` (`app/src/ai/agent/conversation.rs (281-451)`). These are placed in the `RestoredAgentConversations` singleton. When a terminal view opens, `restore_conversations_on_view_creation` calls `BlocklistAIHistoryModel::restore_conversations`, which loads conversations into memory and emits `BlocklistAIHistoryEvent::RestoredConversations`.

For the driver case, `AgentDriver::new` (`app/src/ai/agent_sdk/driver.rs (474-608)`) accepts a `ConversationRestorationInNewPaneType::Historical` conversation pre-loaded from the server. It flows through `restore_conversation_after_view_creation` → `restore_conversations_from_block_params` → `BlocklistAIHistoryModel::restore_conversations`, emitting the same `RestoredConversations` event.

### What is correctly restored today
- `parent_agent_id`, `parent_conversation_id`, `run_id` — persisted in `AgentConversationData` JSON, restored via `new_restored`.
- `children_by_parent` index — rebuilt at startup from all local DB conversations in `initialize_historical_conversations` (`conversation_loader.rs (430-588)`).
- `agent_id_to_conversation_id` routing index — populated in `restore_conversations` for each loaded conversation.

### What is NOT restored (the three gaps)
1. **`OrchestrationEventPoller.watched_run_ids`** (`orchestration_event_poller.rs:65`) — the set of child run_ids to poll per parent conversation. Populated only when `start_agent` runs at runtime. `handle_history_event` explicitly ignores `RestoredConversations` (line 183). After restart, no SSE/poll loop opens and no events arrive.
2. **`OrchestrationEventPoller.event_cursor`** (`orchestration_event_poller.rs:66`) — per-conversation i64 sequence number. Always initializes to 0; `AgentConversationData` has no cursor field. All historical events are re-fetched on the first poll.
3. **`OrchestrationEventService.lifecycle_subscription_routes`** (`orchestration_events.rs:101`) — maps child conversation_id → parent agent_id for V1 local lifecycle dispatch. Not restored; V1 parents miss child status transitions.

## Proposed Changes

### 1. Persist and restore the event cursor sequence number (covers invariants 7, 8, 9, 10)

The in-progress branch `katarina/quality-503-driver-owned-parent-bridge` in `wc-pine` establishes the right primitives here. It introduces a shared `AgentEventConsumer` trait with a `persist_cursor(sequence: i64)` callback (in `app/src/ai/agent_events/driver.rs`) and refactors both the Oz SSE path and the non-Oz Claude Code parent bridge to use a common `run_agent_event_driver`. The i64 sequence number is the existing API cursor — no new parameter type is needed.

The cursor is currently persisted locally only:
- **Non-Oz (parent bridge)**: `ParentBridgeEventConsumer::persist_cursor` writes to a local file (`~/.claude-code/oz-parent-bridge/{session_id}/last-sequence`) and initializes from it via `read_parent_bridge_last_sequence` when the bridge starts.
- **Oz (`SseForwardingConsumer`)**: uses the default no-op `persist_cursor` — the cursor is not persisted today.

For the driver/cloud-restart case, local-only persistence is insufficient: the session changes between runs, so the previous session's local file is not found, and a server-loaded conversation has no local SQLite state. The fix is to persist the cursor to **both** local storage and server-side conversation metadata, then use whichever source is available on restore.

**Local persistence for Oz — call site**
The in-memory cursor is advanced in one place in this repo: `handle_poll_result` at `orchestration_event_poller.rs:413-418` (`self.event_cursor.insert(conversation_id, max_seq)`). The SSE path drains through this same function, so a single write here covers both modes. Add `last_event_sequence: Option<i64>` to `AgentConversationData` and `AIConversation`; immediately after the `event_cursor.insert` call, invoke a new `BlocklistAIHistoryModel::update_event_sequence(conversation_id, max_seq)` helper that calls `write_updated_conversation_state`. This is per-batch (up to `EVENT_POLL_BATCH_LIMIT = 100` events per batch) — acceptable granularity.

Note: The WIP branch `katarina/quality-503-driver-owned-parent-bridge` in `wc-pine` refactors this path to introduce an `SseForwardingConsumer` type with a `persist_cursor` callback. If that branch merges before this one, the call site moves to that callback instead. If this feature lands first, the inline write at `handle_poll_result` is sufficient and the WIP migration can adapt it.

**Server-side persistence (both Oz and non-Oz)**
Add `last_event_sequence: Option<i64>` to `Task` in warp-server (on `ai_tasks`). This field is part of this feature's scope; the companion warp-server change adds it to `GET /agent/runs/:run_id` and a new `PATCH /agent/runs/:run_id/event-sequence` endpoint. When `update_event_sequence` fires, call this endpoint fire-and-forget (log on failure). Losing a cursor update is recoverable — next best cursor is used on restore.

**Restore initialization**
In the `RestoredConversations` handler in the poller, initialize the cursor for each conversation by taking `max` of all available sources:
1. `conversation.last_event_sequence()` — from `AgentConversationData` in local SQLite (present for local restores)
2. The result of `GET /agent/runs/{run_id}` — fetched asynchronously as part of the same async restore query that fetches child run_ids (see change 2). The run response includes `last_event_sequence` directly on the `Task`.

The async server fetch happens alongside the child-run discovery query. When both return, the cursor is set to `max(SQLite value, server value)` and event delivery starts.

### 2. Restore watched_run_ids and event delivery on `RestoredConversations` (covers invariants 1–11)

**`app/src/ai/blocklist/orchestration_event_poller.rs`**

Add a helper to scan task messages for child run_ids (driver/remote-worker case). Gate on `OrchestrationV2` because in V2 the `agent_id` in `StartAgentV2` results is the child's `run_id`; in V1 it is a different identifier and the scan would produce wrong values:
```rust
fn child_run_ids_from_task_messages(conversation: &AIConversation) -> Vec<String> {
    if !FeatureFlag::OrchestrationV2.is_enabled() {
        return vec![];
    }
    let mut out = Vec::new();
    for task in conversation.all_tasks() {
        let Some(api_task) = task.source() else { continue };
        for msg in &api_task.messages {
            let Some(api::message::Message::ToolCallResult(tcr)) = msg.message.as_ref() else { continue };
            let Some(api::tool_call_result::Type::StartAgentV2(r)) = tcr.r#type.as_ref() else { continue };
            let Some(api::start_agent_v2_result::Result::Success(s)) = r.result.as_ref() else { continue };
            if !s.agent_id.is_empty() {
                out.push(s.agent_id.clone());
            }
        }
    }
    out
}
```

Handle `RestoredConversations` in `handle_history_event` (replacing the current no-op at line 183):
- For each `conv_id` in `conversation_ids`, read the conversation from the history model.
- Skip shared-session viewers (`conversation.is_viewing_shared_session()`). (invariant 14)
- Cursor (initial): set `self.event_cursor[conv_id]` to `conversation.last_event_sequence().unwrap_or(0)` from local SQLite. The server value will be merged once the async run fetch completes (see below).
- Own run_id: if `conversation.run_id()` is `Some`, insert into `self.watched_run_ids[conv_id]`.
- Child run_ids and server cursor: spawn two concurrent async calls — `GET /agent/runs/{run_id}/children` and `GET /agent/runs/{run_id}` — and wait for **both** before starting event delivery. Starting delivery before the run response arrives could cause the cursor to be initialized from SQLite only, and a later server cursor merge might not advance it in time to prevent duplicate events.
  - **Both succeed**: merge `event_cursor[conv_id] = max(SQLite, run.last_event_sequence)`; insert child run_ids; start event delivery.
  - **Run fetch fails, children succeed**: keep SQLite cursor; insert child run_ids; start event delivery.
  - **Children fetch fails (with or without run fetch)**: fall back to `child_conversation_ids_of(conv_id)` and `child_run_ids_from_task_messages(conversation)`; start event delivery.
- Start event delivery for any `conv_id` where `watched_run_ids` is non-empty and status is `Success`. If `InProgress`, defer to `on_conversation_status_updated` as usual. (invariants 3, 4)

The `child_run_ids_from_task_messages` helper is still needed as the failure fallback path. The restored cursor value flows naturally into the existing `poll_and_inject` and `start_sse_connection` paths, which both read `self.event_cursor.get(&conversation_id).copied().unwrap_or(0)`. No plumbing change is needed in those methods.

**Delete/remove handler update.** `last_event_sequence` lives in `AgentConversationData`, whose SQLite row is deleted alongside the conversation row. No change to the existing `RemoveConversation`/`DeletedConversation` arm in the poller is needed beyond the existing removals of `watched_run_ids`, `event_cursor`, `poll_backoff_index`, etc.

### 3. Restore V1 lifecycle subscriptions (covers invariant 12)

**`app/src/ai/blocklist/orchestration_events.rs`**

Extend the existing `RestoredConversations` handler (currently lines 355-361) to also re-register subscriptions. For each restored conversation that `is_child_agent_conversation()`:
```rust
if !FeatureFlag::OrchestrationV2.is_enabled() {
    if let Some(parent_agent_id) = history_model
        .conversation(&child_conv.parent_conversation_id()?)
        .and_then(|p| p.server_conversation_token())
        .map(|t| t.as_str().to_string())
    {
        self.register_lifecycle_subscription(conv_id, parent_agent_id, None);
    }
}
```
This mirrors the runtime registration in `terminal_pane.rs (1158-1165)`.

## End-to-end flow

There are two separate entry points for event delivery; both must work after this change:

```
[Entry point 1 — restore-time, new]
Warp restarts
  → BlocklistAIHistoryModel::restore_conversations()
      → conversations inserted into conversations_by_id
      → RestoredConversations { conversation_ids } emitted
          → OrchestrationEventService (subscription independent, no ordering guarantee)
              sync conversation statuses
              re-register V1 lifecycle subscriptions for restored children [new]
          → OrchestrationEventPoller [new handler]
              for each conv_id:
                event_cursor[conv_id] ← SQLite last_event_sequence (if present)
                watched_run_ids[conv_id] += own run_id
                spawn async GET /agent/runs/{run_id} + GET /agent/runs/{run_id}/children
                  → run response: event_cursor[conv_id] = max(SQLite, server last_event_sequence)
                  → children response: watched_run_ids[conv_id] += child run_ids
                  → on failure: fallback to DB index ∪ task message scan
                  → if Status=Success and watched_run_ids non-empty:
                       start_event_delivery() → poll/SSE with cursor = event_cursor[conv_id]
                         → persist_cursor after each event → updates SQLite + server metadata
              for InProgress convs: delivery deferred until next Success transition (entry point 2)

[Entry point 2 — status transition, pre-existing]
Parent conversation transitions to Success (InProgress → Success)
  → BlocklistAIHistoryEvent::UpdatedConversationStatus emitted
      → OrchestrationEventPoller::on_conversation_status_updated
          if watched_run_ids contains entries: start_event_delivery()
            (cursor already initialized from restore; persist_cursor keeps it current)
```

## Risks and Mitigations

**Schema additions are backward-compatible**: Adding `last_event_sequence` to `AgentConversationData` (with `#[serde(default)]`) and to `ai_tasks` (with no NOT NULL constraint) is non-breaking. Older clients or missing fields fall back to cursor=0 and re-deliver events — the same behavior as today.

**V1 lifecycle subscription filter is lost on restart**: At runtime, `register_lifecycle_subscription` is called with the original `request.lifecycle_subscription` (which may be a filtered subset of event types). After restart the filter is not persisted, so re-registration uses `None` (subscribe to all types). The parent will receive lifecycle types it was not originally subscribed to. This is acceptable given V1 is legacy and the behavior is strictly wider coverage, not narrower.

**`persist_cursor` frequency**: The callback fires after every event. Server writes on every event could be noisy for active orchestrations. The fire-and-forget nature (log-on-failure, no blocking) mitigates latency impact. If server write frequency becomes a concern, the implementation can debounce (e.g., only write to the server if the cursor advanced by more than N or more than T seconds have elapsed since the last server write).

**Mid-response crash**: `persist_cursor` is called after each event is processed, before the agent's response (echo) is received. If the process crashes after a `persist_cursor` write but before the agent processes the event, the cursor would reflect an event the agent hasn't acted on yet. On restart, those events would not be re-delivered. This is acceptable: the agent already received the event as input in a previous request, and the server-side task messages record that the input was sent.

**Child run_ids from task messages may include finished children**: A `StartAgentV2 { Success { agent_id } }` result means the child was launched, but it may have long since finished. The poller will open a connection for that run_id and receive zero events (all before the restored cursor), then idle. This is harmless.

**Startup cost of task message scan**: `child_run_ids_from_task_messages` is an in-memory scan over data already decoded during restoration. The matched message type (`StartAgentV2` results) is rare. O(total messages), near-zero constant, no I/O.

## Testing and Validation

Reference `PRODUCT.md` for invariant numbers.

- **Invariant 1, 2** (parent receives child events after restart — polling path): Unit test in `orchestration_event_poller_tests.rs` — emit `RestoredConversations` with `OrchestrationEventPush` disabled; verify `watched_run_ids` is populated and `poll_and_inject` uses the restored `event_cursor` value (not 0) as its `cursor` argument on the first call.
- **Invariant 1, 2** (parent receives child events after restart — SSE path): Same test with `OrchestrationEventPush` enabled; verify `start_sse_connection` uses the restored `event_cursor` value as `since_sequence` on the first open and continues using the in-memory cursor (updated via `persist_cursor`) on reconnects.
- **Invariant 2** (terminal events during downtime): Unit test — emit `RestoredConversations` for a `Success` parent with watched run_ids; verify that delivery starts immediately without any `InProgress` → `Success` transition.
- **Invariant 4** (delivery deferred for InProgress parent): Unit test — emit `RestoredConversations` for an `InProgress` parent; verify `watched_run_ids` is populated but `start_event_delivery` is not called; then emit `UpdatedConversationStatus` → `Success` and verify delivery starts.
- **Invariant 5** (no children, no polling): Unit test — emit `RestoredConversations` for a conversation with no `run_id` and no children; verify `watched_run_ids` is not populated and no delivery is started.
- **Invariant 7, 8** (no duplicate delivery via SQLite cursor): Unit test — emit `RestoredConversations` for a conversation whose `last_event_sequence` in `AgentConversationData` is 42; verify `event_cursor[conv_id]` is initialized to 42 and the first poll call uses `cursor=42`.
- **Invariant 7, 8** (no duplicate delivery via server run response): Unit test — same setup but `AgentConversationData.last_event_sequence` is absent and the server run response returns `last_event_sequence = 42`; verify `event_cursor[conv_id]` is set to 42 when the async query returns.
- **Invariant 7, 8** (cursor write at `handle_poll_result`): Unit test — call `handle_poll_result` with a batch whose max sequence is 42; verify `BlocklistAIHistoryModel::update_event_sequence` is called with 42.
- **Invariant 6** (children_by_parent index pre-populated): Unit test asserting `BlocklistAIHistoryModel::child_conversation_ids_of(parent)` returns the child IDs before `RestoredConversations` fires (i.e., that `initialize_historical_conversations` at `conversation_loader.rs:466-477` builds the index at startup).
- **Invariant 10** (driver child run_ids from task messages): Unit test for `child_run_ids_from_task_messages` with `OrchestrationV2` enabled — build a conversation with a `StartAgentV2` success result; verify the agent_id is returned. Also test with `OrchestrationV2` disabled; verify an empty vec is returned.
- **Invariant 10** (driver case: no local DB child): Unit test — emit `RestoredConversations` where `child_conversation_ids_of` returns empty but task messages contain a `StartAgentV2` success result; verify the child run_id is still added to `watched_run_ids`.
- **Invariant 11** (umbrella): Covered by invariants 1–9 and 10 above; no separate test.
- **Invariant 12** (V1 lifecycle subscriptions after restart): Unit test in `orchestration_events_tests.rs` with legacy local lifecycle-dispatch mode — emit `RestoredConversations` with a child conversation having `parent_conversation_id` set; verify `lifecycle_subscription_routes` is populated with `None` event-type filter (subscribe-all). Note: `None` is intentional and broader than the original subscription filter, which is not persisted (see Risks).
- **Invariant 13** (non-orchestration conversations unaffected): Existing restore tests must continue to pass without modification.
- **Invariant 14** (shared-session viewers excluded): Unit test — emit `RestoredConversations` for a `is_viewing_shared_session = true` conversation; verify no entry in `watched_run_ids` and `event_cursor` is not initialized for it.
- **Invariant 15** (orphan child restored standalone): Unit test — emit `RestoredConversations` for a child whose `parent_conversation_id` points at an id not present in `conversations_by_id`; verify no lifecycle subscription is registered and no error is surfaced.
- **Invariant 16** (cleanup on delete): Unit test — populate `watched_run_ids` and `event_cursor` for a conversation; emit `DeletedConversation`; verify both are removed.
- **Invariant 9** (transient failures retried): Covered by existing SSE failure/backoff tests in `orchestration_event_poller_tests.rs`; verify those tests still pass.
- **Manual (local child, OrchestrationV2 on)**: Run parent + local child with V2 enabled, quit Warp mid-run, restart; confirm child's final status is shown and no previously seen messages are re-delivered.
- **Manual (local child, OrchestrationV2 off)**: Same scenario with V2 disabled; confirm V1 lifecycle subscriptions propagate the child's status after restart.
- **Manual (driver/remote child)**: `warp agent run --conversation <parent-id>` where children ran on remote workers; confirm child run_ids are discovered from task messages and events are delivered from the correct resume point.

## Non-Oz harness considerations

Non-Oz harnesses cannot currently be parents in an orchestration session, so this feature does not directly affect them. The notes below are forward-looking.

The WIP branch `katarina/quality-503-driver-owned-parent-bridge` in `wc-pine` introduces `AgentEventConsumer::persist_cursor` and implements it in `ParentBridgeEventConsumer` (for a Claude Code **child** receiving parent messages) by writing to a local `last-sequence` file. That is a different role from a non-Oz conversation acting as a parent, but the `persist_cursor` hook is the same primitive.

When non-Oz parents become supported, they can reuse the server-side `last_event_sequence` field on `ai_tasks` added by change 1 above: wiring their `persist_cursor` callback to also call `PATCH /agent/runs/{run_id}/event-sequence` gives cloud/driver restore without any harness-specific logic.

**Why `AgentConversationData` alone is insufficient for cloud/driver non-Oz**: `AgentConversationData` is exclusively local SQLite (never sent to server). The non-Oz parent bridge state directory (`last-sequence` file) is session-scoped, so a new `warp agent run --conversation` invocation won't find the previous session's file. The server-side field is the only path that works for driver restarts.

**Child run_id discovery for non-Oz parents**: The two synchronous fallback sources used today (`children_by_parent` DB index and `child_run_ids_from_task_messages`) both depend on Oz's structured task messages and will not apply to non-Oz parents. The primary path — `GET /agent/runs/{parent_run_id}/children` — is already harness-agnostic (the `ai_tasks` table stores `parent_run_id` regardless of harness) and will cover non-Oz parents without any additional changes.

## Follow-ups
- Consider scanning V1 `StartAgent` tool results (`ToolCallResultType::StartAgent`) alongside `StartAgentV2` once V1 orchestration is fully deprecated, for completeness.
- When non-Oz event delivery is fully implemented, wire `persist_cursor` in the non-Oz parent bridge to also call the server-side `last_event_sequence` update, mirroring the Oz path.
