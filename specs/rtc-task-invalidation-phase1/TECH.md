# Tech spec: RTC task invalidation — Phase 1 (client-only)

## Context

RTC invalidations for cloud agent tasks cause excessive `GET /api/v1/agent/runs` requests. During a bug bash with multiple concurrent agents on a team, this triggered 429 rate limiting that blocked spawning new agents. See [investigation report](/Users/liliwilson/Downloads/05_18_2025%20429s%20when%20starting%20cloud%20agents.md) for full context.

### Current flow

The server sends `AmbientTaskUpdated { TaskId, Timestamp }` over the websocket on every task state transition, session link update, conversation ID update, and task creation. The client receives this in the `Listener` → `UpdateManager` → `AgentConversationsModel` chain:

1. `listener.rs:113-116` — `ObjectUpdateMessage::AmbientTaskUpdated { task_id, timestamp }` arrives with both fields.
2. `update_manager.rs:1119-1125` — `handle_ambient_task_changed` **discards** `task_id` (param named `_task_id`) and emits `UpdateManagerEvent::AmbientTaskUpdated { timestamp }` with only the timestamp.
3. `agent_conversations_model.rs:673-696` — `handle_update_manager_event` throttles at 5s, then calls `fetch_tasks_updated_after(timestamp)` which hits `GET /api/v1/agent/runs?limit=100&updated_after={ts}` — a list fetch of all recently-updated tasks.

### Three consumer surfaces

- **Conversation details panel** (`terminal/view/ambient_agent/view_impl.rs:938-977`): pane-level sidebar showing one task. Uses `get_or_async_fetch_task_data(task_id)` which hits `GET /agent/runs/{task_id}` with per-task dedup. Not connected to RTC directly — free-rides on the list-fetch populating `self.tasks`. Today this works because the list-fetch fires unconditionally on every RTC event. But once we gate the list-fetch on whether views are open (change 3), the details panel loses its data source and needs its own RTC path.
- **Agent management view** (`workspace/view.rs:8023-8048`): full-page dashboard. Shows all tasks (personal + team). Registers with `register_view_open`/`register_view_closed`.
- **Conversation list view** (`workspace/view/left_panel.rs:1030-1047`): left panel sidebar. Shows **personal tasks only** (`OwnerFilter::PersonalOnly` at `conversation_list/view_model.rs:73`). Also registers with `register_view_open`/`register_view_closed`.

### Problems

1. `task_id` discarded → forces broad list-fetch on every RTC event
2. Details panel has no direct RTC path → relying indirectly on `AgentConversationsModel`.
3. Every RTC event triggers a list-fetch even if no list view is open
4. No recovery if websocket misses a message (polling fully disabled when RTC is on, `agent_conversations_model.rs:981-983`)

### Out of scope: spawn.rs session polling

`ambient_agents/spawn.rs` has a separate polling loop (`poll_run_until_joinable_session`, `spawn.rs:165-308`) that polls `GET /agent/runs/{task_id}` every 3s (`TASK_STATUS_POLL_INTERVAL`, `spawn.rs:23`) to detect when a session becomes joinable. **Not affected by these changes.**

The tab IS registered in `ActiveAgentViewsModel` on `TaskSpawned` (`model.rs:1261-1262`), so the RTC handler (change 2a) will see `has_open_tab = true` and trigger redundant re-fetches during spawn. This is a minor inefficiency (~4-5 extra single-task requests per spawn) deferred for now — the big win is eliminating list-fetches.

RTC cannot replace spawn.rs because:
- spawn.rs drives the session state machine (`WaitingForSession` → `AgentRunning`) by emitting `AmbientAgentEvent::SessionStarted` (`spawn.rs:292-295`), which triggers the shared session join (`model.rs:1311-1346`). RTC only refreshes cached task data.
- spawn.rs handles timeouts, error/terminal states, followup stale-state skipping, and cancellation.
- spawn.rs extracts `SessionJoinInfo::from_task` (`spawn.rs:278`) each poll; RTC events only carry `task_id` + `timestamp`.

### Relevant files

- `app/src/server/cloud_objects/listener.rs:113-116` — websocket message type with `task_id`
- `app/src/server/cloud_objects/update_manager.rs:1119-1126` — discards `task_id`
- `app/src/server/cloud_objects/update_manager.rs:137-142` — `UpdateManagerEvent` enum
- `app/src/ai/agent_conversations_model.rs:56` — `RTC_TASK_REFRESH_THROTTLE` (5s)
- `app/src/ai/agent_conversations_model.rs:673-696` — `handle_update_manager_event`
- `app/src/ai/agent_conversations_model.rs:735-768` — `fetch_tasks_updated_after`
- `app/src/ai/agent_conversations_model.rs:932-961` — `register_view_open`/`register_view_closed`
- `app/src/ai/agent_conversations_model.rs:975-1001` — `should_be_polling`
- `app/src/ai/agent_conversations_model.rs:1519-1601` — `get_or_async_fetch_task_data`
- `app/src/ai/active_agent_views_model.rs:83-93` — tracks focused conversations and ambient sessions
- `app/src/terminal/view/ambient_agent/view_impl.rs:938-977` — details panel data fetch
- `app/src/workspace/view/conversation_list/view_model.rs:68-91` — personal-only filter

## Proposed changes

### 1. Pass `task_id` through the event chain

In `update_manager.rs`, add `task_id` to the event:

```rust path=null start=null
// update_manager.rs:137-142
enum UpdateManagerEvent {
    // ...
    AmbientTaskUpdated { task_id: AmbientAgentTaskId, timestamp: DateTime<Utc> },
}
```

Rename `_task_id` → `task_id` in `handle_ambient_task_changed` and include it in the emitted event. This requires importing `AmbientAgentTaskId` in `update_manager.rs`.

### 2. Per-surface RTC dispatch in `AgentConversationsModel`

Replace the current `handle_update_manager_event` (which unconditionally list-fetches) with a dispatch that routes based on what's open.

New `handle_update_manager_event` logic:

```rust path=null start=null
fn handle_update_manager_event(&mut self, event: &UpdateManagerEvent, ctx: &mut ModelContext<Self>) {
    let UpdateManagerEvent::AmbientTaskUpdated { task_id, timestamp } = event else {
        return;
    };

    // (a) Details panel: if any window has this task focused, do a targeted single-task fetch.
    //     This uses get_or_async_fetch_task_data which already deduplicates in-flight requests.
    if self.is_task_actively_viewed(*task_id, ctx) {
        self.refresh_single_task(*task_id, ctx);
    }

    // (b) List views: if management view or conversation list is open, do a throttled list-fetch.
    if self.has_any_active_data_consumers() {
        self.handle_rtc_for_list_views(*timestamp, ctx);
    } else {
        // (c) Nothing open: mark dirty for later.
        self.dirty_task_ids.insert(*task_id);
    }
}
```

#### 2a. `is_task_actively_viewed`

Check `ActiveAgentViewsModel` for whether the `task_id` has an open ambient session tab:

```rust path=null start=null
fn is_task_actively_viewed(&self, task_id: AmbientAgentTaskId, ctx: &AppContext) -> bool {
    ActiveAgentViewsModel::as_ref(ctx)
        .get_terminal_view_id_for_ambient_task(task_id)
        .is_some()
}
```

This covers any window where the task is open in a tab (not just the focused window). The details panel refresh is cheap — `get_or_async_fetch_task_data` won't spawn a second HTTP request if one is already in flight for the same task_id (it checks `TaskFetchState::InFlight` at `agent_conversations_model.rs:1532` and returns `None` immediately).

#### 2b. `refresh_single_task`

Call `get_or_async_fetch_task_data` which already exists, has per-task dedup (`TaskFetchState::InFlight`), backoff for failures, and emits `TasksUpdated` on completion. No new code needed — just call it.

#### 2c. `handle_rtc_for_list_views`

Extract the existing throttle logic from today's `handle_update_manager_event` into this method. Identical behavior to today — throttled list-fetch.

We keep the list-fetch (rather than batching single-task fetches) because: (1) the management view shows team tasks, so it needs to discover new tasks created by teammates — single-task fetches can only refresh known task_ids; (2) during bursts (20 tasks changing in a 5s window), 1 list-fetch is cheaper than 20 individual requests; (3) the big win is gating — not doing the list-fetch at all when the view isn't open.

#### 2d. `has_any_active_data_consumers`

```rust path=null start=null
fn has_any_active_data_consumers(&self) -> bool {
    self.active_data_consumers_per_window
        .values()
        .any(|views| !views.is_empty())
}
```

### 3. Dirty-on-open flush

Add a `dirty_task_ids: HashSet<AmbientAgentTaskId>` field to `AgentConversationsModel`.

In `register_view_open`, after the existing logic, flush dirty tasks:

```rust path=null start=null
if !self.dirty_task_ids.is_empty() {
    let dirty_ids: Vec<_> = self.dirty_task_ids.drain().collect();
    for task_id in dirty_ids {
        self.get_or_async_fetch_task_data(&task_id, ctx);
    }
}
```

This uses single-task fetches (not a list-fetch) so it's bounded by the number of tasks that changed while the view was closed.

Cap the dirty set at ~200 entries. If it overflows, clear it and do a full list-fetch on next view open instead.

### Summary of request reduction

Before: every RTC event → 1 list-fetch (`GET /agent/runs?updated_after=...`), throttled to 1 per 5s.

After:
- Details panel open, nothing else → 1 single-task fetch per event (deduped by `TaskFetchState`)
- Management/convo list open → same list-fetch as today (throttled)
- Nothing open → 0 requests, dirty marks only

No safety-net periodic poll for now. The websocket `Listener` already triggers an object refresh on reconnection (`listener.rs:232`), which covers the main failure mode (disconnect). If we see data gaps from silently dropped messages, we can add a slow poll later.

For a team of 10 running 5 agents with 4-5 state changes each: before = ~250 list-fetches across team in 5 min. After = ~0 list-fetches for users not looking at views, plus ~1 single-task fetch per user per agent they have open.

## Testing and validation

**Unit tests** in `agent_conversations_model_tests.rs`:
- RTC event with `task_id` when details panel has that task open → `get_or_async_fetch_task_data` called
- RTC event when no views open → task_id added to `dirty_task_ids`, no fetch
- `register_view_open` with dirty tasks → each dirty task fetched via single-task endpoint
- Dirty set overflow (>200) → cleared, list-fetch on next open

**Manual verification**:
- Add `log::info!("[lili] ...")` in `handle_update_manager_event` to count fetches before/after
- Run multiple cloud agents on a team, verify no list-fetches when management view is closed
- Open details panel for an agent, verify state changes appear within ~3s
- Close all views, let agents run, reopen management view → verify dirty tasks load

## Parallelization

Not beneficial — all changes are in `agent_conversations_model.rs` and `update_manager.rs` with tight coupling between them. Single-agent serial implementation is the right approach.
