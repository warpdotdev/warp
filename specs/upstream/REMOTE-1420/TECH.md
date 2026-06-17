# REMOTE-1420: Fix O(N×M) refresh storm when opening a shared session

## Context

Opening a shared-session link beachballs the desktop app and crashes Warp-on-Web. Main thread stalls inside `ConversationListViewModel::refresh_cached_items` → `AgentConversationsModel::conversation_ids_shadowed_by_tasks` → `BlocklistAIHistoryModel::find_conversation_id_by_server_token`.

Each lookup is O(conversations + metadata) (two linear scans), the miss path emits `log::info!`, and INFO is captured as a synchronous Sentry breadcrumb (JSON-encoded on main).

- **N** = ambient agent tasks in `AgentConversationsModel.tasks`.
- **M** = `conversations_by_id.len() + all_conversations_metadata.len()` scanned per task.

Shared-session hydration + streaming fire many `BlocklistAIHistoryEvent`s; each that reaches `ConversationListViewModel` triggers a full refresh → O(N × M) per event.

### Relevant code

- `app/src/ai/blocklist/history_model.rs:1835` — `find_conversation_id_by_server_token`: two linear scans, `log::info!` on miss (line 1860).
- `app/src/ai/blocklist/history_model.rs:229` — `all_conversations_metadata: HashMap<AIConversationId, AIConversationMetadata>` (current forward index by id).
- `app/src/ai/blocklist/history_model.rs:236` — `agent_id_to_conversation_id`, the existing reverse-index pattern to mirror.
- `app/src/ai/blocklist/history_model.rs:597-645` — `restore_conversations`: emits one `UpdatedConversationStatus` per restored convo + one `RestoredConversations`.
- `app/src/ai/blocklist/history_model.rs:806-873` — `initialize_output_for_response_stream` / `assign_run_id_for_conversation`: where live tokens first land; `agent_id_to_conversation_id` is maintained here.
- `app/src/ai/blocklist/history_model.rs:1500, 1897, 1922, 1939` — other `all_conversations_metadata` mutation sites.
- `app/src/ai/blocklist/history_model/conversation_loader.rs:335-423, 425-573` — `merge_cloud_conversation_metadata`, `initialize_historical_conversations`: bulk inserts into metadata.
- `app/src/ai/agent_conversations_model.rs:1113-1134` — `conversation_id_shadowed_by_task` / `conversation_ids_shadowed_by_tasks`: the hot call site, one lookup per task.
- `app/src/ai/agent_conversations_model.rs:1136-1201` — `handle_history_event`: translates `BlocklistAIHistoryEvent` → `AgentConversationsModelEvent`; `UpdatedConversationStatus` → `ConversationUpdated`.
- `app/src/workspace/view/conversation_list/view_model.rs:33-46` — subscription that fans every model event into `refresh_cached_items`.
- `app/src/workspace/view.rs:2838-2850`, `app/src/ai/agent_management/view.rs:1179-1206` — other `ConversationUpdated` consumers (transcript side panel, management details panel) that must keep receiving the event.

## Proposed changes

### 1. Reverse index `server_token → conversation_id` (primary fix)

Add a new field to `BlocklistAIHistoryModel`:

```rust path=null start=null
server_token_to_conversation_id: HashMap<ServerConversationToken, AIConversationId>,
```

Maintain it at every site that inserts, mutates, or removes an entry carrying a `server_conversation_token`. Mirror the existing `agent_id_to_conversation_id` pattern for symmetry and discoverability.

**Insert/update sites:**

- `initialize_historical_conversations` (`conversation_loader.rs:435`) — bulk build while iterating persisted metadata.
- `merge_cloud_conversation_metadata` (`conversation_loader.rs:389-412`) — both the matched-by-token and new-cloud-only branches.
- `mark_conversations_historical_for_terminal_view` (`history_model.rs:1897`).
- `insert_forked_conversation_from_tasks` (`history_model.rs:1922`).
- `initialize_output_for_response_stream` (`history_model.rs:814-838`) — first assignment of a live token.
- `assign_run_id_for_conversation` (`history_model.rs:845-873`) — v2 path; convenient audit point.
- `set_server_metadata_for_conversation` (`history_model.rs:449-479`) — only re-insert if the incoming token differs from the cached one.

**Removal sites:**

- `remove_conversation_from_memory` (`history_model.rs:1487-1529`).
- `reset` (`history_model.rs:1928-1941`) — clear.

Rewrite `find_conversation_id_by_server_token` as a single O(1) `HashMap::get` on the new index.

Resolve an invariant question while implementing: all callers currently treat "conversation in memory" and "conversation in metadata" as the same thing, so a single index keyed by token is sufficient if we guarantee entries are inserted whenever a token becomes known for either a live conversation or a metadata entry. Verify this by grepping all callers (`conversation_loader.rs:262`, `conversation_details_panel.rs`, `pane_group/mod.rs`, `workspace/view.rs`, `block/view_impl/output.rs`, `agent_view/orchestration_conversation_links.rs`, `conversation_list/view.rs`) and the two shadow-by-task callers.

**Effect:** per-refresh cost drops from O(N × M) to O(N).

### 2. Demote miss-log to `debug`

Change `log::info!` at `history_model.rs:1860` to `log::debug!`. INFO is forwarded to Sentry breadcrumbs and JSON-encoded synchronously on the main thread (visible in the desktop sample as `SentryCrashJSONCodec` frames). DEBUG is not captured.

INFO level isn't warranted here: a token miss is the expected outcome whenever a task references a conversation the local client hasn't loaded (shared-session tasks from other users, server-only tasks, pre-sync state). It's not an error, not actionable for production telemetry, and repeats identically per-task per-refresh so it provides zero new information after the first occurrence. DEBUG is the right level for local diagnostic use; if we ever need to investigate a specific lookup miss, developers can opt in.

Keep the log for diagnostics; just stop sending it to Sentry. Acts as a safety net if the index ever misses a site.

### 3. Stop rebuilding the list cache on `ConversationUpdated`

In `ConversationListViewModel::new` (`view_model.rs:33-46`), split the match arm: on `ConversationUpdated`, do not call `refresh_cached_items`. Emit `ConversationListViewModelEvent` directly so the view re-renders and reads fresh status at render time via `get_item_by_id`. Status is never cached in `cached_conversation_or_task_ids`, so the cache does not depend on it.

Keep `AgentConversationsModelEvent::ConversationUpdated` itself — the wasm transcript side panel (`workspace/view.rs:2844`) and the agent-management details panel (`agent_management/view.rs:1195`) still need it to refresh their status readouts.

This cuts a dominant source of per-event refreshes during post-restore streaming: every status flip (from `AIConversation::update_status_with_error_message`, many callers) no longer walks the task list.

## Testing and validation

Unit tests in `app/src/ai/blocklist/history_model_test.rs`:

- `find_conversation_id_by_server_token` returns `Some(id)` after each of: `initialize_historical_conversations`, `merge_cloud_conversation_metadata` (both branches), `initialize_output_for_response_stream`, `assign_run_id_for_conversation`, `insert_forked_conversation_from_tasks`, `mark_conversations_historical_for_terminal_view`.
- Returns `None` after `remove_conversation_from_memory` and `reset`.
- Returns the same id after `set_server_metadata_for_conversation` when token is unchanged.

Regression test for the view-model: subscribing a fake `AgentConversationsModel` emitter of `ConversationUpdated` does not change `cached_conversation_or_task_ids` but still emits `ConversationListViewModelEvent`.

Manual: open a shared-session link as a user with ≥ ~50 ambient-agent tasks. Confirm no main-thread stall and no repeating "No conversation found for server token" burst in logs. Compare desktop sample before/after to confirm `find_conversation_id_by_server_token` no longer appears in the hot path.

`./script/presubmit` (fmt + clippy + tests).

## Risks and mitigations

- **Missed index maintenance site** → stale lookup misses. Mitigation: unit test coverage per site above; log-level demotion keeps the miss path cheap even if we regress.
- **Token collisions** between `all_conversations_metadata` and `conversations_by_id` (same token, different `AIConversationId`). Shouldn't happen given existing dedup-by-token logic in `merge_cloud_conversation_metadata`, but add a `debug_assert!` on insert to catch regressions.
- **`ConversationUpdated` no longer refreshing the cache** could hide a case where a status flip should reorder/remove the item. Checked: the sort key is `last_updated()` (set at append time) and the filter keys don't include status. Leave a comment in `refresh_cached_items` documenting the invariant.

## Follow-ups

- Coalesce `refresh_cached_items` across a frame so bursty `TasksUpdated` from streaming collapse into one pass.
- Cache `conversation_ids_shadowed_by_tasks` on `AgentConversationsModel` and maintain incrementally on task/metadata mutations, so refresh becomes O(filtered items) instead of O(N).
