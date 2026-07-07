# Context
APP-4459 changes Cloud Mode setup-v2 failures so they render as the conversation-ended tombstone and hide input, instead of showing the failure in the agent message bar.
Today `AmbientAgentViewModel::handle_spawn_error` stores `Status::Failed { error_message }` and emits `AmbientAgentViewModelEvent::Failed` in `app/src/terminal/view/ambient_agent/model.rs:1318`.
`TerminalView::handle_ambient_agent_event` handles `Failed` by removing the queued prompt block, updating the active cloud conversation status, refreshing the details panel, and notifying in `app/src/terminal/view/ambient_agent/view_impl.rs:176`.
Setup-v2 failures currently reach `BlocklistAIStatusBar::render_cloud_mode_setup_terminal_message`, which renders `ambient_agent_model.error_message()` as a red message bar in `app/src/ai/blocklist/block/status_bar.rs:917`.
The existing tombstone insertion path is `TerminalView::insert_conversation_ended_tombstone` in `app/src/terminal/view/shared_session/view_impl.rs:1669`. It sets `conversation_ended_tombstone_view_id`.
Input hiding already keys off that tombstone ID in `TerminalView::is_input_box_visible` in `app/src/terminal/view.rs:7216`, so no separate input-hiding state is needed.
`ConversationEndedTombstoneView` already enriches from `AmbientAgentTask` and uses `task.status_message.message` when `task.state.is_failure_like()` in `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:140`. This is the right source for task-backed failures, especially third-party harness runs where the local `AIConversation` is only a UI vehicle.
`TaskStatusMessage` now carries optional `error_code` data in `app/src/ai/ambient_agents/task.rs:520`, with `environment_setup_failed` identifying setup-command failures that should not offer continue actions.
Setup failures can happen before an `AmbientAgentTask` exists but still have an active local `AIConversation`. The `Failed` handler writes the error into that conversation before inserting the tombstone, so the tombstone can read conversation status.

# Proposed changes
Keep this setup-v2 only. Setup-v1 uses the old full-screen/status-footer failure UI.

In `TerminalView::handle_ambient_agent_event`:
- Keep the existing `Failed` status update and details-panel refresh.
- When `FeatureFlag::CloudModeSetupV2.is_enabled()`, insert the tombstone after updating status.
- Avoid duplicate insertion by respecting `conversation_ended_tombstone_view_id`.
In `conversation_output_status_from_conversation`:
- Keep finished root-exchange statuses as the preferred source for success, cancelled, and exchange-backed errors.
- If there is no finished root exchange and `conversation.status()` is `ConversationStatus::Error`, convert `conversation.status_error_message()` into `AmbientConversationStatus::Error`.
- Use `RenderableAIError::Other` with `will_attempt_resume: false` and `waiting_for_network: false` for that fallback.

In `ConversationEndedTombstoneView`:
- Read error display data from conversation status.
- Mark display data as error when either `conversation.status().is_error()` or conversation output status has an error.
- Add `hide_continue_actions` to suppress both Continue locally and Continue in cloud.
- For pre-task setup failures, detect an error conversation with no task ID and no transcript, then set title to `Cloud agent failed to start`, clear credits, and hide continue actions.
- Keep async task enrichment, and let task failure metadata override the conversation error when available.
- When task enrichment sees failure-like state with `TaskStatusMessage.error_code == environment_setup_failed`, hide continue actions.
- Keep existing finished-exchange/conversation-derived errors for normal Oz tombstones.
In `BlocklistAIStatusBar::render_cloud_mode_setup_terminal_message`, remove or gate only the `ambient_agent_model.error_message()` branch. Keep the GitHub auth and cancelled branches unchanged.

In `TaskStatusMessage`:
- Add optional `error_code` with serde support for `errorCode`.
- Add `is_environment_setup_failure()` helper for tombstone display decisions.

Error source order should be:
1. `AmbientAgentTask.status_message.message` for task-backed failures.
2. Conversation status error, including pre-task setup failures.

# Testing and validation
Add focused Rust coverage:
- setup-v2 `AmbientAgentViewModelEvent::Failed` inserts one tombstone and removes the queued prompt block.
- `TerminalView::is_input_box_visible` returns false after the failure tombstone is inserted.
- duplicate `Failed` events still keep one tombstone rich-content view.
- pre-task failure tombstone uses `Cloud agent failed to start`, hides credits, and hides continue actions.
- task-backed failure tombstone renders `AmbientAgentTask.status_message.message`.
- task-backed environment setup failures hide continue actions via `TaskStatusMessage.error_code`.
- setup-v2 status bar no longer renders the generic failure message branch.
