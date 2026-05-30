# APP-4365: Use Queued Query UI for Oz Cloud Mode Queries — Tech Spec
## Context
`PRODUCT.md` defines the user-visible target: initial and follow-up Oz cloud mode prompts should use the same queued-query UI as third-party cloud agents, setup-command rich content should remain unchanged, and the real transcript query should render normally when it arrives.
Cloud mode panes are deferred shared-session viewers. `create_cloud_mode_view` creates a deferred viewer manager, subscribes it to `AmbientAgentViewModel`, and connects or appends a session on `SessionReady` / `FollowupSessionReady` in `app/src/terminal/view/ambient_agent/mod.rs (42-105)`. `is_cloud_agent_pre_first_exchange` identifies the setup interval after a session is ready but before the first exchange arrives, including the third-party harness-start escape hatch in `app/src/terminal/view/ambient_agent/mod.rs (110-162)`.
`AmbientAgentViewModel` owns cloud run state. It tracks `Status::WaitingForSession { kind: InitialRun | Followup }`, the selected harness, task/session IDs, pending follow-up prompt, and today also tracks Oz optimistic-query state via `has_inserted_cloud_mode_user_query_block` and `optimistically_rendered_user_queries` in `app/src/terminal/view/ambient_agent/model.rs (64-145)`. Initial cloud runs call `spawn_internal`, store the request, transition to `WaitingForSession`, and emit `DispatchedAgent` in `app/src/terminal/view/ambient_agent/model.rs (660-679)`. Follow-up runs call `submit_cloud_followup`, set `pending_followup_prompt`, transition to `WaitingForSession`, and emit `FollowupDispatched` in `app/src/terminal/view/ambient_agent/model.rs (492-529)`.
The queued-query visual pattern already exists as `PendingUserQueryBlock`: it renders the prompt with user avatar, dimmed text, and a `Queued` badge, with optional close/send-now controls in `app/src/ai/blocklist/block/pending_user_query_block.rs (16-175)`. `TerminalView::insert_cloud_mode_queued_user_query_block` inserts that block without buttons and without a queued callback, specifically for cloud mode lifecycle-owned prompts, in `app/src/terminal/view/pending_user_query.rs (76-91)`. It is inserted as rich content with `RichContentMetadata::PendingUserQuery` and `PinToBottom` in `app/src/terminal/view/pending_user_query.rs (28-74)` and `app/src/terminal/view/rich_content.rs (183-244)`.
The current event handling creates divergent UI. On `DispatchedAgent`, third-party harnesses rebuild the display prompt with `display_user_query_with_mode` and call `insert_cloud_mode_queued_user_query_block`, while Oz creates `CloudModeInitialUserQuery`, inserts it as rich content, sets `has_inserted_cloud_mode_user_query_block`, and records the stripped prompt as optimistic UI in `app/src/terminal/view/ambient_agent/view_impl.rs (129-189)`. On `FollowupDispatched`, Oz creates `CloudModeFollowupUserQuery`, inserts it, and records the optimistic prompt in `app/src/terminal/view/ambient_agent/view_impl.rs (200-229)`.
The bespoke optimistic views live in `app/src/terminal/view/ambient_agent/block/query.rs (20-159)`. They share normal AI query rendering and show a `Failed` label when the ambient model has an error. They are re-exported through `app/src/terminal/view/ambient_agent/block.rs (1-8)`.
The duplicate-hiding logic lives in the AI block renderer. `should_hide_ai_block_query_and_header` hides a real AI block query/header for the first cloud exchange or any recorded optimistic follow-up prompt while the viewer is live and not receiving replay in `app/src/ai/blocklist/block/view_impl.rs (99-123)`. `AIBlock::render` asks the ambient model for `has_inserted_cloud_mode_user_query_block` and `has_optimistic_user_query` before deciding whether to render the actual query/header in `app/src/ai/blocklist/block/view_impl.rs (842-907)`. Tests currently assert this hiding behavior in `app/src/ai/blocklist/block/view_impl/cloud_mode_setup_tests.rs (1-37)`.
Setup-command rich content is independent of the optimistic prompt blocks. `maybe_insert_setup_command_blocks` gates on CloudModeSetupV2 and `is_cloud_agent_pre_first_exchange`, then inserts the setup intro and setup-command block before the real terminal block in `app/src/terminal/view/ambient_agent/view_impl.rs (373-453)`. When the first Oz exchange is appended, `handle_ai_history_model_event` turns off `is_executing_oz_environment_startup_commands` before inserting the real AI block in `app/src/terminal/view.rs (5086-5159)`.
Follow-up submission is already routed away from the stale shared-session network. `try_submit_pending_cloud_followup` intercepts `InputEvent::SendAgentPrompt`, validates the task, calls `submit_cloud_followup`, and resets the input in `app/src/terminal/view.rs (19735-19794)`. If not intercepted, `TerminalViewEvent::SendAgentPrompt` still goes to the viewer network in `app/src/terminal/shared_session/viewer/terminal_manager.rs (1323-1398)`. Existing follow-up and tombstone tests cover this route in `app/src/terminal/view_test.rs (552-576)` and `app/src/terminal/view/shared_session/view_impl_test.rs (638-805)`.
## Proposed changes
### Use one cloud queued-query insertion path
Update `TerminalView::handle_ambient_agent_event` so both Oz and third-party cloud runs use `insert_cloud_mode_queued_user_query_block` while waiting for the real cloud transcript.
For `DispatchedAgent`:
- Keep the existing viewer short-circuit and `CloudModeSetupV2` gate.
- Remove the `is_third_party_harness()` branch that sends Oz down the bespoke optimistic path.
- Build the display prompt from `AmbientAgentViewModel::request()` using the existing `display_user_query_with_mode(request.mode, &request.prompt)` helper, matching the current third-party path and satisfying `PRODUCT.md` Behavior 4.
- Insert the queued-query block when the display prompt is non-empty.
- Do not call `set_has_inserted_cloud_mode_user_query_block` or `record_optimistic_user_query`.
For `FollowupDispatched`:
- Keep the conversation-status update to `ConversationStatus::InProgress`.
- Read `pending_followup_prompt()` and call `insert_cloud_mode_queued_user_query_block(prompt, ctx)` instead of creating `CloudModeFollowupUserQuery`.
- Do not record the prompt as optimistic UI.
This intentionally reuses `PendingUserQueryBlock` rather than `QueuedQueryModel`. `QueuedQueryModel` in `app/src/ai/blocklist/queued_query.rs (1-67)` is not currently wired into the terminal rich-content path, while `insert_cloud_mode_queued_user_query_block` is the existing third-party cloud UI surface the product spec names as the reference.
### Remove the queued item only when the real transcript user query arrives
Add cloud-specific removal at the point the real Oz AI exchange with a renderable user query is appended. In `handle_ai_history_model_event`, after turning off `is_executing_oz_environment_startup_commands` for ambient sessions and before inserting the `AIBlock`, call `remove_pending_user_query_block(ctx)` only when:
- `CloudModeSetupV2` is enabled,
- the terminal is an ambient agent session,
- the appended exchange belongs to the visible root task that will produce an AI block,
- the exchange contains an input that will render a user query for the submitted prompt, and
- `pending_user_query_view_id` is set.
The key invariant is that the queued item survives session attach and any intermediate shared-session output until the actual user-query element from the real shared-session response exists. `SessionReady`, `FollowupSessionReady`, progress updates, setup-command blocks, harness-start transitions, and generic output without a renderable user query must not remove the queued item.
The removal should not depend on prompt string matching unless the existing exchange/input model exposes a cheap exact display-query comparison. The cloud queued-query helper owns a single lifecycle-managed pending item, and the first visible real user-query exchange for that ambient conversation is the replacement surface. This avoids retaining a stale queued item while also avoiding the old real-query hiding logic. If the exchange is filtered out by `blocklist_filter::should_show_task_in_blocklist`, or if it does not contain a renderable user query, leave the pending item untouched until a later visible user-query exchange, failure, cancellation, auth event, or other terminal lifecycle event handles it.
Keep the existing error/auth/cancel cleanup at the top of `handle_ambient_agent_event` in `app/src/terminal/view/ambient_agent/view_impl.rs (111-126)`. That cleanup already removes lifecycle-owned cloud queued items and should continue to apply to Oz and third-party runs. Do not remove the queued item on `SessionReady` or `FollowupSessionReady`; those events mean a shared session exists, not that the real user-query transcript has rendered.
### Delete Oz optimistic-query rendering and tracking
Remove `CloudModeInitialUserQuery`, `CloudModeFollowupUserQuery`, and the private `render_user_query` helper from `app/src/terminal/view/ambient_agent/block/query.rs`. If that leaves the file empty, remove `mod query;` and `pub use query::*;` from `app/src/terminal/view/ambient_agent/block.rs`.
Remove `AmbientAgentViewModel` fields and methods that only support optimistic query hiding:
- `has_inserted_cloud_mode_user_query_block`
- `optimistically_rendered_user_queries`
- `has_inserted_cloud_mode_user_query_block()`
- `set_has_inserted_cloud_mode_user_query_block(...)`
- `record_optimistic_user_query(...)`
- `has_optimistic_user_query(...)`
Also remove their initialization and reset logic in `new` and `reset_for_new_cloud_prompt`.
Remove `should_hide_ai_block_query_and_header` from `app/src/ai/blocklist/block/view_impl.rs`, delete the ambient-model lookup in `AIBlock::render`, and let the existing `query_and_index` path render the real query/header normally. This implements `PRODUCT.md` Behavior 11-14 directly: the queued rich content is the transient placeholder, and the AI block is the real transcript representation.
### Preserve setup-command rich content
Do not change `maybe_insert_setup_command_blocks`, `CloudModeSetupTextBlock`, `CloudModeSetupCommandBlock`, `is_cloud_agent_pre_first_exchange`, or the block-list startup-command flags. The queued-query change should be limited to which pending prompt rich content is inserted and when it is removed. This preserves `PRODUCT.md` Behavior 7-8 and avoids changing the setup-command grouping/order behavior already covered by CloudModeSetupV2.
### Update comments and names where they become stale
Update comments on `insert_cloud_mode_queued_user_query_block` from “non-oz Cloud Mode run” to “Cloud Mode run waiting for the real transcript” so it accurately covers Oz and third-party runs. Update comments in `pending_user_query_view_id` only if needed; the field remains shared by normal `/queue` prompts and lifecycle-owned cloud queued prompts, but cloud insertion still uses the callback-free helper.
No new feature flag is needed. Initial-run behavior remains under `CloudModeSetupV2`, and cloud follow-up behavior remains reachable only through the existing `HandoffCloudCloud` follow-up entrypoints.
## End-to-end flow
1. User submits an initial Oz cloud prompt.
2. `AmbientAgentViewModel::spawn_internal` stores the stripped request and emits `DispatchedAgent`.
3. `TerminalView::handle_ambient_agent_event` reconstructs the display prompt and inserts a callback-free `PendingUserQueryBlock`.
4. Setup commands, if any, continue through `maybe_insert_setup_command_blocks`.
5. When the real shared-session response appends an Oz exchange with a renderable user query, `handle_ai_history_model_event` removes the queued-query rich content and inserts the normal `AIBlock`.
6. `AIBlock::render` renders the real query/header because the optimistic-query hiding gate is gone.
7. For follow-up prompts, `try_submit_pending_cloud_followup` calls `submit_cloud_followup`, `FollowupDispatched` inserts the same queued-query block, and the next appended exchange that contains the actual follow-up user query replaces it the same way.
## Testing and validation
Unit tests should cover the product invariants without depending on pixel rendering:
- Add or update a terminal view test that enables `CloudModeSetupV2`, dispatches an Oz initial cloud run, and asserts that `pending_user_query_view_id` is set and the corresponding rich content metadata is `PendingUserQuery`. This covers Behavior 1, 3, 5, 18, 20, and 25.
- Add the same assertion for a non-Oz harness to ensure the existing third-party path is unchanged. This covers Behavior 3 and 26.
- Add a follow-up test around `FollowupDispatched` or `try_submit_pending_cloud_followup` that asserts a queued-query block is inserted instead of an optimistic follow-up view. This covers Behavior 2, 6, 19, 20, and 25.
- Add a history-model append test for an ambient cloud conversation that starts with a queued-query block, appends a visible exchange containing a renderable user query, and asserts the pending block is removed while an AI block remains. This covers Behavior 9-14 and 24.
- Add a negative history/session test that attaches a session, emits progress/setup-command/generic output, or appends an exchange without a renderable user query, and asserts the queued-query block remains visible. This covers Behavior 10.
- Remove or rewrite `app/src/ai/blocklist/block/view_impl/cloud_mode_setup_tests.rs (1-37)`. The old assertions should no longer hold; if a lightweight replacement is useful, assert that normal AI block query/header rendering is not suppressed for cloud exchanges.
- Keep existing setup-command tests or add a regression assertion that `maybe_insert_setup_command_blocks` still inserts setup text and command blocks while a queued-query block exists and does not remove that queued item. This covers Behavior 7-8, 10, and 23.
- Keep existing follow-up route tests in `app/src/terminal/view_test.rs (552-576)` and `app/src/terminal/view/shared_session/view_impl_test.rs (638-805)` passing; extend them only if useful to assert queued-query insertion after `submit_cloud_followup`.
Targeted validation commands:
- `cargo test -p warp cloud_mode_setup_tests`
- `cargo test -p warp pending_cloud_followup`
- `cargo test -p warp append_followup`
- `cargo check -p warp --features handoff_cloud_cloud`
Before opening or updating a PR, follow repository rules for formatting and clippy. Do not use `cargo fmt --all` or file-specific `cargo fmt`.
Manual validation:
- With CloudModeSetupV2 enabled, submit an initial Oz cloud prompt and verify the queued-query item appears immediately, remains visible after session attach and while setup-command rich content runs, and is replaced only when the real transcript user query appears.
- Submit a follow-up from an eligible cloud tombstone or owned follow-up input and verify the same queued-query item appears and is replaced only by the real follow-up transcript query.
- Repeat the same initial run with a third-party harness to confirm its queued-query behavior is unchanged.
- Test failure/auth/cancel during startup and confirm the queued item follows the existing third-party lifecycle and no bespoke `Failed` optimistic-query block remains.
## Risks and mitigations
### Removing the queued item too early
`SessionReady`, setup-command output, progress events, or an exchange without a renderable user query can arrive before the real user-query transcript is rendered. Removing the queued item there would create a visible gap. Mitigate by removing only when a visible exchange contains the actual user query from the shared-session transcript.
### Removing unrelated `/queue` prompts
`pending_user_query_view_id` is shared by normal queued prompts and cloud lifecycle prompts. Mitigate by only using the new `AppendedExchange` removal inside ambient cloud-session handling, and only after a visible cloud exchange with a renderable user query is about to render.
### Prompt display mismatch for `/plan` and `/orchestrate`
The spawn request stores the stripped prompt and separate mode. Mitigate by keeping the existing `display_user_query_with_mode` reconstruction already used by third-party cloud runs.
### Setup-command regressions
Setup-command insertion is adjacent to the query UI in the same event flow. Mitigate by not touching `maybe_insert_setup_command_blocks` or startup-command flags, and by adding a regression check that setup blocks still render alongside the queued item.
### Stale optimistic-query code paths
Leaving optimistic tracking in place could continue hiding real transcript queries. Mitigate by deleting the model fields/methods and the AI block hiding helper in the same change that switches insertion to queued-query UI.
## Parallelization
Parallel implementation is not beneficial for this change. The work is tightly coupled across one event handler, one model cleanup, one AI-block render gate, and a small set of targeted tests; splitting it across agents would increase merge conflicts more than it would reduce wall-clock time.
