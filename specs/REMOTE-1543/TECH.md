# Queued Prompts UI — Technical Spec
See `specs/REMOTE-1543/PRODUCT.md` for user-visible behavior. This document covers implementation only.
## Context
The new queued prompts panel is more capable than the previous pending-prompt indicator, but it should not replace the old path for all users yet. The new panel must be behind a new feature flag, enabled in dogfood only. When that flag is off, Warp must keep using the previous pending queued prompt UI and lifecycle code, including Cloud Mode setup.
The implementation should therefore be additive:
- Keep the legacy pending-query code restored and wired exactly as the feature-off path.
- Keep the new multi-row queue model and input-adjacent panel as the feature-on path.
- Avoid rewriting the old UI/lifecycle code just to support the new path.
## Feature flag
Add a new runtime feature flag:
- Cargo feature: `new_queued_prompt_ui` in `app/Cargo.toml`.
- Runtime flag: `FeatureFlag::NewQueuedPromptUI` in `crates/warp_features/src/lib.rs`.
- App registration: `#[cfg(feature = "new_queued_prompt_ui")] FeatureFlag::NewQueuedPromptUI` in `app/src/lib.rs`.
- Dogfood enablement: include `FeatureFlag::NewQueuedPromptUI` in `DOGFOOD_FLAGS`.
Do not add `new_queued_prompt_ui` to the default Cargo feature list. Stable / non-dogfood users should see the old queued prompt UI until this feature is promoted.
## Rollout behavior
The rollout matrix is:
- `QueueSlashCommand` off: queue trigger surfaces that depend on `/queue` remain disabled as they do today.
- `QueueSlashCommand` on and `NewQueuedPromptUI` off: use the legacy pending queued prompt UI and single-slot callback behavior.
- `QueueSlashCommand` on and `NewQueuedPromptUI` on: use the new multi-row queue model and `QueuedPromptsPanelView`.
`PendingUserQueryIndicator` remains the gate for whether the legacy pending prompt indicator renders. `NewQueuedPromptUI` is the gate for replacing that legacy indicator with the new panel.
## Legacy feature-off path
Restore the old code as-is where possible:
- `app/src/ai/blocklist/block/pending_user_query_block.rs`
- `app/src/terminal/view/pending_user_query.rs`
- `RichContentMetadata::PendingUserQuery` and `RichContent::is_pending_user_query` in `app/src/terminal/view/rich_content.rs`
- `PendingUserQueryKind`, `pending_user_query_view_id`, `pending_user_query_kind`, and `queued_prompt_callback` in `app/src/terminal/view.rs`
- selected-text plumbing for `PendingUserQueryBlock` in `app/src/terminal/model/blocks/selection.rs` and `TerminalView::pending_user_query_selected_text`
The legacy path should continue using `TerminalView::send_user_query_after_next_conversation_finished` for user-managed queued prompts. It should not be reimplemented on top of `QueuedQueryModel` when `NewQueuedPromptUI` is off.
Legacy behavior:
- `/queue <prompt>` while an agent response is in progress shows the old pending user query block when `PendingUserQueryIndicator` is enabled.
- `/compact-and <prompt>` and `/fork-and-compact <prompt>` show the old pending user query block while the summarization/fork summarization is running.
- The old block supports the existing dismiss and send-now affordances according to the old call-site options.
- On successful completion, the old callback removes the block and submits the queued prompt through `Input::submit_queued_prompt`.
- On error/cancel, the old callback removes the block and restores the prompt into the input when the input is empty.
## New feature-on path
When `FeatureFlag::NewQueuedPromptUI.is_enabled()` is true, use the new multi-row queue implementation:
- `QueuedQueryModel` in `app/src/ai/blocklist/queued_query.rs`
- `QueuedPromptsPanelView` in `app/src/ai/blocklist/queued_prompts_panel.rs`
- `Input::queued_prompts_panel` in `app/src/terminal/input.rs`
- `TerminalView::drain_queued_prompts` in `app/src/terminal/view.rs`
`QueuedPromptsPanelView::should_render` must require:
- `FeatureFlag::QueueSlashCommand.is_enabled()`
- `FeatureFlag::NewQueuedPromptUI.is_enabled()`
- an active conversation with queued rows
It should not require `PendingUserQueryIndicator`; the new feature flag is the rollout switch for the new UI.
### `QueuedQueryModel`
`QueuedQueryModel` owns only the new feature-on behavior:
- `queues: HashMap<AIConversationId, Vec<QueuedQuery>>`
- `editing: Option<EditingRow>`
- `collapsed: HashSet<AIConversationId>`
- `queue_next_prompt_enabled: bool`
`QueuedQueryOrigin::InitialCloudMode` remains non-user-managed. It renders in the new panel when the flag is on, but cannot be edited, deleted, reordered, or auto-fired by the local queue drain.
### Queue trigger routing
Every trigger surface must branch on `FeatureFlag::NewQueuedPromptUI`:
- Feature on: append to `QueuedQueryModel`.
- Feature off: call the legacy `send_user_query_after_next_conversation_finished` path.
Required call-site behavior:
- `Input::maybe_queue_input_for_in_progress_conversation`:
  - Feature on: append `QueuedQueryOrigin::AutoQueueToggle` to `QueuedQueryModel`.
  - Feature off: call into the terminal view / workspace action path that ultimately uses `send_user_query_after_next_conversation_finished`.
- `/queue <prompt>` in `app/src/terminal/input/slash_commands/mod.rs`:
  - Feature on: append `QueuedQueryOrigin::QueueSlashCommand`.
  - Feature off: use the legacy pending-query path with close and send-now enabled.
- `/compact-and <prompt>` in `Workspace::summarize_active_ai_conversation`:
  - Feature on: summarize immediately, then append `QueuedQueryOrigin::CompactAnd`.
  - Feature off: summarize immediately, then queue the prompt through the legacy pending-query callback with send-now disabled.
- `/fork-and-compact <prompt>` in `Workspace::handle_forked_conversation_prompts`:
  - Feature on: summarize the fork immediately, then append `QueuedQueryOrigin::ForkAndCompact`.
  - Feature off: summarize the fork immediately, then queue the prompt through the legacy pending-query callback with send-now disabled.
- `WorkspaceAction::QueuePromptForConversation`:
  - Feature on: append `QueuedQueryOrigin::AutoQueueToggle`.
  - Feature off: use the legacy pending-query path.
## Cloud Mode setup
Cloud Mode setup is the most important compatibility requirement.
When `NewQueuedPromptUI` is off:
- Keep using the old `insert_cloud_mode_queued_user_query_block(prompt, ctx)` path in `app/src/terminal/view/pending_user_query.rs`.
- Do not render the new input-adjacent queue panel for Cloud Mode setup.
- Remove the old Cloud Mode pending block with the legacy removal function when the run lifecycle hands off to real transcript content, auth, cancellation, or failure.
When `NewQueuedPromptUI` is on:
- Use `QueuedQueryModel` with `QueuedQueryOrigin::InitialCloudMode`.
- Store the returned `QueuedQueryId` on `AmbientAgentViewModel::cloud_mode_queued_query_id`.
- Remove that row on the same lifecycle handoff events that removed the legacy block.
- Keep the row across `Failed` only when the Cloud Mode setup UI intentionally needs the prompt visible above the failure/tombstone state.
Cloud Mode lifecycle handlers in `app/src/terminal/view/ambient_agent/view_impl.rs` should branch once and call the appropriate removal/insertion helper for the active feature path. The old helper should remain old-code-shaped; the new helper should remain queue-model-shaped.
## Terminal view wiring
`TerminalView::new` should only construct and attach `QueuedPromptsPanelView` when `FeatureFlag::NewQueuedPromptUI.is_enabled()` is true. When the flag is off, `Input::queued_prompts_panel` stays `None` and the old rich-content block is responsible for queued prompt display.
`TerminalView::handle_ai_controller_event` should branch on the feature flag when a conversation finishes:
- Feature on: call `drain_queued_prompts(conversation_id, finish_reason, ctx)`.
- Feature off: run the restored legacy `queued_prompt_callback` after unrelated `conversation_completed_callbacks`, matching the old ordering.
When an active AI block is detected for a different conversation, the legacy feature-off path should keep the old guard that clears the pending block to avoid firing a stale callback later. The feature-on path should keep relying on `QueuedQueryModel` conversation scoping.
## Rich content and selection
Because the old UI returns as the feature-off path, restore the rich-content metadata and selection support:
- `RichContentMetadata::PendingUserQuery { pending_user_query_block_handle }`
- `RichContent::is_pending_user_query`
- `read_selected_text_from_pending_user_query_block`
- `TerminalView::pending_user_query_selected_text`
This code should be used by the legacy path only, but it can remain compiled unconditionally to minimize churn and keep the restored code close to the old implementation.
## Telemetry
New panel-specific telemetry should be emitted only from `QueuedPromptsPanelView`, which only exists/renders when `NewQueuedPromptUI` is enabled:
- `QueuedPrompt.Edited`
- `QueuedPrompt.Deleted`
- `QueuedPrompt.Reordered`
- `QueuedPrompt.PanelCollapseToggled`
The enablement state for those telemetry events should be `FeatureFlag::NewQueuedPromptUI`, not `QueueSlashCommand`, because these events describe the new panel UI rather than queue trigger availability.
Legacy feature-off queuing should keep existing telemetry behavior from slash-command acceptance and existing prompt submission paths. Do not add new telemetry to the restored legacy block.
## Tests
Update tests to cover both rollout paths.
Feature-off tests:
- `/queue` or the queue workspace action inserts the old `PendingUserQueryBlock` rich content.
- Cloud Mode `DispatchedAgent` inserts the old pending user query block when `NewQueuedPromptUI` is off.
- Cloud Mode lifecycle removal removes the old block when the transcript/harness handoff arrives.
Feature-on tests:
- Existing `QueuedQueryModel` and `QueuedPromptsPanelView` tests should set `FeatureFlag::NewQueuedPromptUI` where rendering or panel construction depends on it.
- Cloud Mode `DispatchedAgent` appends an `InitialCloudMode` row and records `cloud_mode_queued_query_id`.
- `drain_queued_prompts` only runs the model drain when the new flag is on.
Regression checks:
- With `NewQueuedPromptUI` off, the new panel is not constructed or rendered.
- With `NewQueuedPromptUI` off, Cloud Mode setup never displays the new queued prompt panel.
- With `NewQueuedPromptUI` on, legacy `pending_user_query_view_id` remains unused for new queue rows.
## Validation
Run:
- `cargo fmt`
- A targeted compile/test pass for the touched client code, preferably the queued prompt and terminal view tests.
- Full presubmit before PR submission.
Do not run the app as part of this change.
## Risks and mitigations
- **Accidentally rewriting legacy behavior**: restore the old files and keep the feature-off path calling the old helpers. Only add conditional routing at call sites.
- **Two sources of queue truth**: `QueuedQueryModel` is feature-on only for user-visible queued prompt management. The legacy callback remains feature-off only. Branch at trigger and drain call sites so both systems do not process the same prompt.
- **Cloud Mode setup regression**: explicitly branch Cloud Mode insertion/removal helpers on `NewQueuedPromptUI` and add tests for both paths.
- **Telemetry misattribution**: gate panel telemetry on `NewQueuedPromptUI` so the new UI metrics do not fire for legacy users.
