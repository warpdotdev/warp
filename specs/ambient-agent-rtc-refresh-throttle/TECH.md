# Ambient agent RTC refresh throttling — Tech Spec
## Context
Ambient agent task updates arrive through the Warp Drive RTC subscription. `WarpDriveUpdate::AmbientTaskUpdated` is converted into `ObjectUpdateMessage::AmbientTaskUpdated` in `app/src/workspaces/gql_convert.rs:1124`, then forwarded through `Listener::get_warp_drive_updates` in `app/src/server/cloud_objects/listener.rs:351`. `UpdateManager::received_message_from_server` gates the event on `FeatureFlag::AmbientAgentsRTC` in `app/src/server/cloud_objects/update_manager.rs:1278` and emits `UpdateManagerEvent::AmbientTaskUpdated { timestamp }` from `handle_ambient_task_changed` in `app/src/server/cloud_objects/update_manager.rs:1119`.
`AgentConversationsModel::new` subscribes to `UpdateManager` when `AmbientAgentsRTC` is enabled in `app/src/ai/agent_conversations_model.rs:568`. Today `AgentConversationsModel::handle_update_manager_event` immediately calls `fetch_tasks_updated_after` for every task update event in `app/src/ai/agent_conversations_model.rs:641`. That method calls `AIClient::list_ambient_agent_tasks` with `TaskListFilter { updated_after: Some(timestamp - 1 second) }`, which maps to `GET /api/v1/agent/runs` in `app/src/server/server_api/ai.rs:1726`.
This creates request volume proportional to team-wide ambient task update volume. For users in large Warp teams, many unrelated cloud agent task updates can trigger many list-run requests from every client. The non-RTC periodic polling path in `AgentConversationsModel::poll_for_tasks` uses a 30 second interval and is disabled while `AmbientAgentsRTC` is enabled in `should_be_polling` at `app/src/ai/agent_conversations_model.rs:893`. The orchestration viewer has a 5 second polling cadence in `app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs`, but it is not an event-stream throttle and should not be reused directly.
## Proposed changes
Add an RTC-only leading/trailing throttle inside `AgentConversationsModel`. Keep `Listener` and `UpdateManager` as generic event plumbing; the request-volume policy belongs at the single consumer that owns `list_ambient_agent_tasks`.
Add a module-local constant:
- `RTC_TASK_REFRESH_THROTTLE: Duration = Duration::from_secs(5)`
Add an internal throttle state to `AgentConversationsModel`:
- `Idle` when no throttle window is active.
- `CoolingDown { pending_timestamp, timer_abort_handle }` after a refresh has fired and the 5 second cooldown timer is active.
When `handle_update_manager_event` receives `UpdateManagerEvent::AmbientTaskUpdated`:
- If state is `Idle`, call `fetch_tasks_updated_after(timestamp, ctx)` immediately and start a 5 second cooldown timer.
- If state is `CoolingDown`, merge the event into `pending_timestamp` without issuing a request.
When the cooldown timer expires:
- If `pending_timestamp` is present, call `fetch_tasks_updated_after(pending_timestamp, ctx)` and start a new cooldown timer.
- If there is no pending timestamp, return to `Idle`.
The pending timestamp must be the earliest timestamp observed during the cooldown window. The server filter is `updated_after`, so latest-timestamp coalescing could skip runs updated earlier in the window after the leading request began. Earliest-timestamp coalescing may over-fetch a small window, but it preserves correctness and still caps request volume.
Leave `fetch_tasks_updated_after` as the raw fetch method so initial sync, manual filter fetches, and future direct callers do not inherit RTC throttling accidentally. Keep the existing 1 second subtraction buffer because the server filter is strict and server/client clocks can differ.
On `reset`, abort any active RTC throttle timer and clear pending state to avoid a trailing fetch after logout. Keep the existing `spawn_with_retry_on_error` behavior for each logical fetch; this change caps logical fetch starts, and retry policy can be revisited separately if backend attempt volume remains too high.
## Testing and validation
Add focused unit coverage for the throttle's timestamp coalescing helper:
- no pending timestamp records the first timestamp;
- later timestamps do not replace the earliest pending timestamp;
- earlier timestamps replace the pending timestamp.
If a future patch adds an injectable `AIClient` seam or fake clock for `AgentConversationsModel`, add model-level tests for:
- a burst of RTC events causes one immediate fetch and one trailing fetch;
- the trailing fetch uses the earliest pending timestamp;
- `reset` cancels any pending trailing refresh;
- RTC disabled keeps the existing periodic polling behavior unchanged.
Run targeted validation:
- `cargo nextest run -E 'test(rtc_task_refresh)'`
- `cargo check -p warp`
Before opening or updating a PR, run the repository-required format and clippy checks. Do not use `cargo fmt --all` or file-specific `cargo fmt`.
## Parallelization
Do not split implementation across sub-agents. The change is small and tightly coupled: the spec, throttle state, event handling, reset cleanup, and tests all live around `AgentConversationsModel`. Parallel edits would touch the same files and add merge overhead without meaningful wall-clock savings.
