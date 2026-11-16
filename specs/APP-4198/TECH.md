# Cloud task consistency tech spec
## Context
Cloud agent runs can currently show inconsistent status across client surfaces. A live/restored conversation and its tombstone may show a terminal state while the cloud-mode details panel and agent management row still show the cached task as `In progress`, including a cancel action that should no longer be visible.
There is no sibling `PRODUCT.md` for this work. The intended user-visible behavior is that every client surface representing the same ambient agent run should show the same status, filter into the same status bucket, and expose actions that match that user-visible status.
`AgentConversationsModel` owns the task cache used by the affected surfaces. `ConversationOrTask::status()` maps raw `AmbientAgentTaskState` into `ConversationStatus` for task rows, and also reads live conversation status for non-task conversation rows in `app/src/ai/agent_conversations_model.rs (171-221)`. The same model intentionally hides local conversation rows when a task represents the same run in `app/src/ai/agent_conversations_model.rs (1109-1217)`, so task-backed rows are the visible representation when both records exist.
The management view renders task row status and status filtering through `ConversationOrTask` in `app/src/ai/agent_management/view.rs (762-961, 1450-1733)`. The details panel is populated from raw task state through `ConversationDetailsData::from_task()` in `app/src/ai/conversation_details_panel.rs (287-340)`, and its action buttons use `AmbientAgentTaskState::is_cancellable()` through `ActionButtonsConfig::for_task()` in `app/src/ai/agent_management/details_action_buttons.rs (45-73)`.
The cloud-mode details panel also reads cached task data from `AgentConversationsModel::get_or_async_fetch_task_data()` in `app/src/terminal/view/ambient_agent/view_impl.rs (594-626)`. That method returns cached task data without forcing a refresh when the task is already present in `app/src/ai/agent_conversations_model.rs (1283-1316)`.
The terminal/tombstone path derives completed state from the live/restored `AIConversation`. `ConversationEndedTombstoneView` reads `conversation_output_status_from_conversation()` in `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs (52-127)`. `TaskStatusSyncModel` separately maps `AIConversation.status()` changes to server task states in `app/src/ai/blocklist/task_status_sync_model.rs (77-214)`.
The current model event stream separates task and conversation updates. `AgentConversationsModel::handle_history_event()` emits `ConversationUpdated` for `UpdatedConversationStatus`, while task fetch/RTC paths emit `TasksUpdated` or `NewTasksReceived` in `app/src/ai/agent_conversations_model.rs (1047-1107)`. Since task-backed rows shadow local conversation rows, downstream task surfaces need a model-layer status abstraction that responds to both streams without mutating either raw state source.
## Proposed changes
Keep `AmbientAgentTaskState` and `ConversationStatus` separate. Do not project conversation status into cached `AmbientAgentTask` records, and do not overwrite task states fetched from the server.
Add a dedicated model-layer display status type near `AgentConversationsModel` / `ConversationOrTask`, for example `AgentRunDisplayStatus` or `AmbientConversationDisplayStatus`. It should not live in UI components. It should represent the user-visible status for a row/details panel/action configuration and should distinguish raw task state from raw conversation state in its variant names.
The display status type should provide the behavior that status-dependent callers need:
- display text
- icon and color
- status filter bucket
- whether cancel should be shown
- whether continue locally should be available
- whether the run should be considered working or terminal for UI purposes
The derived status algorithm should be exhaustively matched and easy to audit:
- `Queued`, `Pending`, and `Claimed` task states use task-derived setup/pre-session status.
- `InProgress` task state uses the matching live/restored `AIConversation.status()` when one is available, and falls back to task-derived in-progress status when no matching conversation is available.
- Terminal task states use task-derived terminal status, even if a matching conversation exists.
- `Unknown` should be treated as failure-like, matching existing `AmbientAgentTaskState` behavior.
Use existing matching logic to associate a task with a conversation. `conversation_id_shadowed_by_task()` already matches by orchestration task/run id and then by server conversation token in `app/src/ai/agent_conversations_model.rs (1077-1096)`. The new display-status helper should reuse that relationship rather than introducing a separate matching path.
Expose the computed status through `ConversationOrTask` or a nearby helper that has access to `AppContext`, because computing status may require reading `BlocklistAIHistoryModel`. Avoid forcing each caller to independently choose between raw task state and raw conversation status.
Route status-dependent behavior through the computed status:
- Management row icon/text should render the computed status.
- Status filtering should use the computed status filter bucket so a row that displays Done does not remain under Working.
- Details panel status badges should render the computed status rather than raw `task_state` when the panel is showing an ambient task.
- Cancel button visibility should follow computed cancellability, not raw `AmbientAgentTaskState::is_cancellable()`. If raw task state is `InProgress` but the matching conversation is terminal, cancel should be hidden.
- Continue-local availability should follow computed working/terminal status so terminal conversations can be continued even if the raw server task is stale.
Update status rendering to consume the new display status type. `app/src/ai/conversation_status_ui.rs` currently renders `ConversationStatus`, while `AmbientAgentTaskState` has separate icon/text helpers in `app/src/ai/ambient_agents/task.rs (296-402)`. Centralizing display behavior on the new type avoids forcing setup states into `ConversationStatus`.
Add robust eventing for derived-status changes. At minimum, all consumers that cache row/action/details data must refresh when either `TasksUpdated`/`NewTasksReceived` or `ConversationUpdated` fires. If this remains ambiguous after implementation, add a clearer `AgentConversationsModelEvent` variant such as `AmbientConversationStatusUpdated { task_id }` and emit it when `UpdatedConversationStatus` changes the computed status for a shadowing task even though the raw task record is unchanged.
Preserve server task fetch and RTC behavior as authoritative for task data and metadata. Incoming server task records continue to update cached task state normally; the computed status layer decides what to show without mutating task state or conversation state.
## End-to-end flow
When a cloud task is in a setup phase, the task row, details panel, and actions render from the task state.
When the task reaches `InProgress`, the associated conversation becomes the source for user-visible progress if it is available. A terminal `ConversationStatus` then causes rows, filters, details, and actions to update even if the cached server task still says `InProgress`.
When the server task later reaches a terminal state, that raw task state becomes the source for user-visible status. This preserves the server task lifecycle while avoiding stale in-progress UI during the live-session window.
## Testing and validation
Add unit tests in `app/src/ai/agent_conversations_model_tests.rs` for the display-status algorithm:
- setup/pre-session task states render task-derived statuses
- `InProgress` task with no matching conversation renders task-derived in-progress status
- `InProgress` task with matching live conversation success, cancelled, blocked, and error renders conversation-derived statuses
- terminal task states render task-derived terminal statuses even when a matching conversation has a conflicting status
- `Unknown` maps to a failure-like display status
Add tests proving the same display status drives status filter buckets. In particular, a task whose raw state is `InProgress` but whose matching conversation is `Success` should appear in the Done bucket and not the Working bucket.
Add tests proving action helpers use the computed status. A stale raw `InProgress` task with a terminal conversation should not be cancellable and should be eligible for continue-local behavior when a conversation token is available.
Add eventing tests around `UpdatedConversationStatus`. When a task row shadows the conversation whose status changes, the model should emit an event that causes task-backed consumers to refresh even if the cached task record did not change.
Run focused tests for `agent_conversations_model_tests` and a targeted compile/check command for the app crate. For a final PR, follow the repository PR workflow for formatting and linting, without running file-specific `cargo fmt`.
## Risks and mitigations
The largest risk is introducing another partial status path. Mitigate by making the display status type the only API used by row rendering, filtering, details status, cancel visibility, and continue-local availability.
Another risk is stale UI due to derived status changing without a task update. Mitigate by either wiring all consumers to refresh on both conversation and task events or by adding a specific derived-status event.
There is also a naming risk if the new type looks like raw server state. Use display/status naming that makes it clear the type is user-visible and derived.
## Parallelization
Implementation can split cleanly after approval. One agent can implement the display-status type, precedence algorithm, and unit tests in `AgentConversationsModel`. Another can audit and update UI consumers so row display, filters, details, cancel actions, and continue-local behavior all use the computed status abstraction. Validation should run after both parts are integrated.
