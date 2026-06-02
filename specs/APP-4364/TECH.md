# Cloud-to-cloud handoff PR 3 tech spec
## Context
PR 3 adds the first user-visible cloud-to-cloud follow-up entrypoint. PR 1 added the disabled `HandoffCloudCloud` flag, typed follow-up API client, and execution-aware task accessors. PR 2 added model-level follow-up orchestration: `AmbientAgentViewModel::submit_cloud_followup` submits a prompt, waits for a fresh session, and emits `FollowupSessionReady`, while `create_cloud_mode_view` already routes that event to `viewer::TerminalManager::attach_followup_session` in `app/src/terminal/view/ambient_agent/mod.rs (69-82)`. This PR wires that orchestration into the terminal UI, shared-session end paths, task liveness model, replay filtering, and targeted tests.
The tombstone UI remains the generic `ConversationEndedTombstoneView`, but it now optionally receives an ambient `task_id`, builds a gated desktop `Continue` cloud action, and keeps the existing local/desktop actions in `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs (176-262, 486-678)`. The view enriches display data from `AmbientAgentTask` and hides local continuation for non-Oz harnesses so third-party harness runs do not offer an unsupported local fork.
Ambient session end handling is split by ownership. Generic `TerminalView::on_session_share_ended` still performs broad session cleanup, but under CloudModeSetupV2 it can insert one tombstone for non-owned ambient viewer sessions before ending the share. Dedicated `on_ambient_agent_execution_ended` and `on_ambient_agent_session_ended` route through `handle_non_running_ambient_agent_task`, mark the task execution ended, refresh the details panel, and either enable owned follow-up input directly or insert a single tracked tombstone for viewer-style sessions in `app/src/terminal/view/shared_session/view_impl.rs (681-879)`. `viewer::TerminalManager::ambient_session_ended` remains the narrow path for execution boundaries and does not mark the pane as a finished viewer.
Terminal input submission still assumes an active shared-session network. `Input::submit_viewer_ai_query` freezes the input, collects context attachments, and emits `InputEvent::SendAgentPrompt` in `app/src/terminal/input.rs (12400-12545)`. `TerminalView::handle_input_event` forwards that as `TerminalViewEvent::SendAgentPrompt` in `app/src/terminal/view.rs (19655-19666)`, and the viewer manager sends it to the current `Network` in `app/src/terminal/shared_session/viewer/terminal_manager.rs (1389-1402)`. Between cloud executions there is intentionally no current network, so PR 3 needs a separate follow-up submission route.
Cloud Mode setup-v2 already has UI to show startup progress and errors from the ambient model. This PR extends `AmbientAgentViewModel` with `pending_followup_prompt`, `should_show_followup_progress`, and `optimistically_rendered_user_queries` so follow-up setup can show progress without duplicating prompts after replay in `app/src/terminal/view/ambient_agent/model.rs (137-179, 532-578)`. `CloudModeInitialUserQuery` is now paired with `CloudModeFollowupUserQuery`, both backed by the shared `render_user_query` styling in `app/src/terminal/view/ambient_agent/block/query.rs`.
## Goals
Show a cloud “Continue” entrypoint on eligible ambient Cloud Mode tombstones behind `HandoffCloudCloud`.
Keep “Continue locally” available and unchanged when the feature flag is off.
Use the existing terminal input/editor for the follow-up prompt instead of embedding a separate tombstone editor.
Route follow-up submission to `AmbientAgentViewModel::submit_cloud_followup`, not to the ended shared-session `Network`.
Reuse the setup-v2 loading/error UI while waiting for the new execution session.
Render the submitted follow-up prompt optimistically while setup is in progress.
Preserve the same terminal pane, local conversation, stable run/task ID, and hotswap attach path.
Avoid replaying already-rendered follow-up prompts into the existing conversation transcript.
## Non-goals
No first-class attachments support for cloud follow-up prompts unless it falls out naturally from the existing input path. It is acceptable for PR 3 to submit text-only follow-ups and leave file attachment support for a follow-up.
No changes to the server follow-up API.
No rollout enablement for `HandoffCloudCloud`.
No details-panel redesign or per-execution history UI.
No change to normal non-ambient shared-session ended behavior.
## Proposed changes
### Ambient execution-ended tombstone insertion
`TerminalView::on_ambient_agent_execution_ended(ctx)` now delegates to `handle_non_running_ambient_agent_task` without calling the full generic shared-session cleanup. The method keeps the terminal view/model, input, ambient view model, pane configuration, and shareable object alive for follow-up hotswap. `on_ambient_agent_session_ended` uses the same helper for task-liveness updates that arrive outside the live shared-session event path.
The helper marks the task execution ended in `AgentConversationsModel`, refreshes the details panel, and then gates UI updates on `HandoffCloudCloud`, `CloudModeSetupV2`, absence of an existing tombstone, and absence of a pending follow-up. Owned ambient panes do not receive a tombstone; if no live shared session remains, they call `enable_owned_cloud_followup_input(task_id, ctx)` so the user can keep typing in the existing input. Non-owned viewer panes insert the tracked tombstone.
The implementation uses a single tracked tombstone per terminal view via `conversation_ended_tombstone_view_id`. `insert_conversation_ended_tombstone` is idempotent and `remove_conversation_ended_tombstone` removes the card when the user starts a cloud follow-up from it. This avoids repeated cards during task state churn and keeps the UI focused on the current resumable boundary.
### Tombstone Continue action
`ConversationEndedTombstoneView` on desktop now creates an optional `Continue` cloud button when the tombstone has an ambient `task_id` and `FeatureFlag::HandoffCloudCloud` is enabled. Rendering additionally requires AI to be enabled and desktop-only compilation. The terminal-view subscription validates that the current ambient view model still owns the clicked `task_id` before entering follow-up compose mode.
The tombstone action is `ContinueInCloud { task_id }`. It records `AgentManagementTelemetryEvent::TombstoneContinueInCloud`, emits `ConversationEndedTombstoneEvent::ContinueInCloud`, and lets `TerminalView::start_cloud_followup_from_tombstone` remove the tombstone, focus the existing input, and set `pending_cloud_followup_task_id`.
Keep “Continue locally” visible for Oz/plain conversations. When both actions are visible, the cloud `Continue` button renders first, followed by `Continue locally`. For non-Oz harnesses, local continuation is hidden because those runs cannot be forked into a local Warp conversation. With `HandoffCloudCloud` disabled, the cloud button is not created.
### Follow-up input mode and submission route
`TerminalView` owns the follow-up compose state with `pending_cloud_followup_task_id: Option<AmbientAgentTaskId>`. Tombstone clicks and owned execution end paths call `reset_after_cloud_followup_submission`, set agent input mode, update pane configuration, and focus the existing input. This keeps the tombstone as a reveal/focus entrypoint rather than an editor.
When `InputEvent::SendAgentPrompt` arrives, `try_submit_pending_cloud_followup` intercepts it before the normal `TerminalViewEvent::SendAgentPrompt` path. It validates the feature flag, ambient model, and task ID, then calls `AmbientAgentViewModel::submit_cloud_followup(prompt, ctx)`. On success it resets the input after submission and returns without emitting to the ended shared-session network. On empty prompts it keeps the compose route active enough to restore agent input. On validation failure it restores the prompt into the input, clears pending follow-up state, focuses input, and shows an error toast.
Slash commands like `/fork` and `/fork-and-compact` keep the existing local behavior from `Input::submit_viewer_ai_query`; the follow-up route handles normal non-empty agent prompts.
### Loading UI and optimistic prompt rendering
PR 2 already sets `Status::WaitingForSession { kind: Followup }` and emits `FollowupDispatched`. PR 3 reuses that state to render setup-v2 loading UI while polling between executions after a follow-up is submitted.
`CloudModeFollowupUserQuery` renders the submitted follow-up prompt using the same `render_user_query` styling as `CloudModeInitialUserQuery` in `app/src/terminal/view/ambient_agent/block/query.rs`. It is inserted on `FollowupDispatched`, not on `DispatchedAgent`, so initial-run behavior remains unchanged.
The ambient view model exposes pending follow-up state through `pending_followup_prompt`, `should_show_followup_progress`, and optimistic query tracking. `FollowupDispatched` records the prompt once, inserts `CloudModeFollowupUserQuery`, and the model tracks rendered prompts in `optimistically_rendered_user_queries` so the same prompt is not rendered again during shared-session replay. Terminal states that end setup clear pending/progress state so rejected submissions do not permanently append optimistic UI.
### Error, retry, replay, and state cleanup
API or polling errors reuse the existing `AmbientAgentViewModelEvent::Failed`, auth, quota, and capacity events so the setup screen can show the same error UI as initial Cloud Mode. If submission fails before the model accepts it, `restore_followup_prompt_after_failed_submission` restores the prompt into the input, re-enters agent input mode, and focuses the input for retry.
If a follow-up is accepted but fails before a session becomes ready, the local conversation stays with the same task/run ID and the ambient status moves to the existing error/auth/cancelled UI states. Retry continues through the same pending follow-up/input path rather than allocating a new local conversation.
When `FollowupSessionReady` fires, follow-up compose/input state and pending optimistic state are cleared, and the existing `FollowupSessionReady -> attach_followup_session` hotswap path attaches the new shared session.
The shared-session replay controller adds `should_skip_current_replayed_response` and `should_skip_replayed_response_for_existing_conversation` to avoid duplicating a response that is already represented in the local conversation when replaying the new execution.
### Telemetry
Add tombstone cloud-follow-up click telemetry near the existing tombstone telemetry in `AgentManagementTelemetryEvent`. The implemented event is `TombstoneContinueInCloud { task_id }`, serialized with the stable task ID. Submission/session-ready/failure outcomes continue to rely on the existing ambient task and follow-up lifecycle telemetry rather than adding separate PR 3-specific events.
## End-to-end flow
1. A Cloud Mode execution ends and the viewer manager receives `SessionEnded`.
2. `ambient_session_ended` records the ended session ID and calls the ambient execution-ended terminal-view method.
3. The terminal view marks task execution ended, refreshes task/details state, and either enables owned follow-up input directly or inserts one tracked tombstone while keeping the pane/input resumable.
4. For non-owned viewer sessions, the user clicks “Continue” on the tombstone.
5. The terminal removes the tombstone, focuses the existing input, and marks the next normal agent prompt as a cloud follow-up.
6. The user submits a prompt.
7. The input/view routes the prompt to `AmbientAgentViewModel::submit_cloud_followup`.
8. The view inserts one optimistic follow-up user query, records it as rendered, and shows setup-v2 loading UI.
9. The model submits the follow-up API request and polls for a fresh session.
10. `FollowupSessionReady` reaches `create_cloud_mode_view`, which calls `attach_followup_session`.
11. The viewer manager joins the new shared session in append mode, and new output streams into the same terminal pane.
## Testing and validation
Unit tests cover the shipped seams: task active/joinable helpers and conversation display status in `app/src/ai/agent_conversations_model_tests.rs`; cloud follow-up compose restoration in `app/src/terminal/view_test.rs`; tombstone insertion/removal, owned follow-up input, stale task rejection, and task end handling in `app/src/terminal/view/shared_session/view_impl_test.rs`; and ambient session-end/network behavior plus replay handling in `app/src/terminal/shared_session/viewer/event_loop_test.rs`.
Remaining useful coverage is tombstone rendering for button visibility with `HandoffCloudCloud` on/off, with and without `task_id`, AI enabled/disabled, and local continuation hidden for non-Oz harnesses.
Ambient view coverage verifies that `FollowupDispatched` inserts/renders an optimistic follow-up prompt separately from `DispatchedAgent`, records rendered prompts, and clears/re-enables follow-up state for retry.
Viewer manager/event-loop coverage verifies that ambient `SessionEnded` inserts or enables follow-up UI without setting `SharedSessionStatus::FinishedViewer`, without sending prompts through a stale network, and without duplicating replayed responses.
Manual validation:
- with `HandoffCloudCloud` disabled, complete a Cloud Mode run and verify the tombstone is unchanged;
- with the flag enabled, complete a Cloud Mode run, click Continue, submit a prompt, verify setup UI appears, and verify a fresh shared session attaches in the same pane;
- repeat once to catch stale session IDs, duplicate tombstones, and subscription leaks;
- verify “Continue locally” still forks locally from the tombstone;
- verify a normal shared-session viewer still becomes read-only/finished when its session ends.
Targeted validation for this PR is `cargo check -p warp --features handoff_cloud_cloud`, focused ambient model/spawn tests from PR 2, and the new tombstone/input/viewer-manager tests. Before opening or updating the PR, follow repo rules for formatting and clippy; do not use `cargo fmt --all` or file-specific `cargo fmt`.
## Risks and mitigations
### Prompt routed to stale network
The largest correctness risk is accidentally sending the follow-up prompt through `TerminalViewEvent::SendAgentPrompt` to a missing or ended `Network`. Mitigate by making follow-up compose mode intercept submission before the viewer-manager network path.
### Tombstone insertion regresses generic viewer teardown
Generic `on_session_share_ended` has important cleanup for ordinary shared sessions, but it is too broad for resumable ambient executions. Mitigate by routing execution boundaries through dedicated ambient methods and keeping ordinary shared-session teardown behavior in `on_session_share_ended`.
### Optimistic prompt duplication
The follow-up prompt or response could appear once as optimistic/local UI and again from replayed shared-session scrollback. Mitigate by tracking `optimistically_rendered_user_queries` in the ambient model and using shared-session replay skip state for already-present conversation responses.
### Retry state drift
Failures between API acceptance and session readiness can leave input frozen or the tombstone hidden. Mitigate by centralizing cleanup on ambient model failure/cancel/auth events and ensuring the tombstone remains available.
### Stale tombstone state
A tombstone can become stale if task state changes while the view is idle. Mitigate by tracking only one tombstone ID, validating the clicked `task_id` against the current ambient model before composing, and removing the tombstone when cloud follow-up starts.
## Definition of done
With `HandoffCloudCloud` off, tombstone and shared-session behavior are unchanged.
With the flag on, eligible ambient Cloud Mode tombstones show a cloud Continue action while preserving Continue locally for Oz/plain conversations.
Clicking Continue focuses/reveals the existing terminal input and submitting a normal prompt calls `AmbientAgentViewModel::submit_cloud_followup`.
The follow-up prompt does not go through the ended shared-session network.
Setup-v2 loading/error UI appears while the follow-up session is starting, and already-rendered replay content is not duplicated.
An optimistic follow-up user query renders during setup without reusing initial-run dispatch UI.
When the new session is ready, the existing `FollowupSessionReady` hotswap path attaches it to the same pane.
Targeted tests and `cargo check -p warp --features handoff_cloud_cloud` pass.
