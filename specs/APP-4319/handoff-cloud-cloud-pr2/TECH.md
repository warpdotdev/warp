# Cloud-to-cloud handoff PR 2 tech spec
## Problem statement
PR 2 should add the orchestration layer that turns an existing cloud agent run into a follow-up execution and hands the resulting fresh shared session to the hotswap path. This PR should remain mergeable while `HandoffCloudCloud` is disabled by default, and it should not add the user-visible tombstone Continue button or terminal-input submission route yet.
The intended boundary is model/API behavior: a future UI can call one ambient-model method with a follow-up prompt, the client submits `POST agent/runs/{runId}/followups`, polls the same run until a new joinable session appears, ignores the ended session, and emits `FollowupSessionReady` so the existing viewer manager attaches the new session in append mode.
## Current state
PR 1 added the disabled `HandoffCloudCloud` flag and encoded the Cargo feature dependency on `cloud_mode_setup_v2` in `app/Cargo.toml:924`. It also added `RunFollowupRequest`, `AIClient::submit_run_followup`, and `build_run_followup_url` in `app/src/server/server_api/ai.rs (211-216, 837-840, 1467-1471)`, with endpoint/serialization tests in `app/src/server/server_api/ai_test.rs (988-1000)`.
`AmbientAgentTask` now has `run_id()`, `conversation_id()`, and `active_run_execution()` accessors that project the current flattened response fields into a `RunExecution` view in `app/src/ai/ambient_agents/task.rs (247-279)`. `SessionJoinInfo::from_task` already consumes that projection in `app/src/ai/ambient_agents/spawn.rs (31-57)`.
Initial cloud startup is still a single combined helper: `spawn_task` creates a run, polls `get_ambient_agent_task`, emits state changes, and ends when the first session is joinable in `app/src/ai/ambient_agents/spawn.rs (85-176)`. There is no reusable “poll an existing run until a new execution session is ready” helper yet.
`AmbientAgentViewModel` still models startup as `Status::WaitingForSession { progress }` without distinguishing an initial run from a follow-up execution in `app/src/terminal/view/ambient_agent/model.rs (50-64)`. The existing `SessionStarted` handler infers follow-up readiness from being already in `AgentRunning`, which is too implicit for a model-driven follow-up flow in `app/src/terminal/view/ambient_agent/model.rs (627-638)`. The current `attach_followup_session` method simply emits `FollowupSessionReady` for a known session ID, which is useful test scaffolding but does not submit or poll a follow-up in `app/src/terminal/view/ambient_agent/model.rs:349`.
The hotswap receiver already exists. `create_cloud_mode_view` routes `SessionReady` to `connect_to_session` and `FollowupSessionReady` to `attach_followup_session` in `app/src/terminal/view/ambient_agent/mod.rs (69-82)`. The viewer manager’s follow-up attach path replaces the active network and joins with append-mode scrollback in `app/src/terminal/shared_session/viewer/terminal_manager.rs (338-384)`.
The UI has important side effects tied to initial dispatch. `DispatchedAgent` inserts the initial optimistic user query in `TerminalView::handle_ambient_agent_event` and drives the ambient entry-block insertion subscription in `app/src/terminal/view/ambient_agent/view_impl.rs (105-131, 445-481)`. PR 2 should avoid reusing that event for follow-ups, because doing so would blur initial-run and follow-up behavior before the UX PR.
## Goals
Add reusable follow-up orchestration that submits a prompt to an existing run and waits for a fresh active execution session.
Make the ambient view model explicitly track whether it is waiting for an initial session or a follow-up session.
Track the active or previous execution session ID so follow-up polling can ignore stale readiness from the ended session.
Emit the already-supported `FollowupSessionReady` event when the new session is ready, allowing the existing hotswap path to attach it.
Reuse existing Cloud Mode setup/loading/error state machinery for follow-up waiting and failures, but without adding a visible Continue entrypoint.
Keep the implementation behind `FeatureFlag::HandoffCloudCloud` and preserve behavior with the flag off.
## Non-goals
No tombstone Continue button, action, or copy changes.
No terminal input routing changes for submitting follow-up prompts.
No embedded follow-up prompt editor in the tombstone.
No product decision on tombstone stacking or update-in-place behavior.
No first-class server execution-array parsing unless the public API response shape already exposes it in this branch.
No rollout enablement for `HandoffCloudCloud`.
## Proposed changes
### Reusable run polling and follow-up helper
Refactor `spawn_task` in `app/src/ai/ambient_agents/spawn.rs` so run creation and run readiness monitoring are separate. Keep the public `spawn_task(request, ai_client, timeout)` behavior the same by having it call a new internal polling helper after `spawn_agent` succeeds.
Add a helper such as `poll_run_until_joinable_session(run_id, ai_client, previous_session_id, timeout)` that repeatedly calls `get_ambient_agent_task(&run_id)`, emits `StateChanged` when state changes, and returns `SessionStarted` only when the task is `InProgress` and `SessionJoinInfo::from_task` contains a parseable `session_id` that differs from `previous_session_id` when one was provided.
Add a follow-up stream/helper such as `submit_run_followup(prompt, run_id, previous_session_id, ai_client, timeout)`. It should call `AIClient::submit_run_followup(run_id, RunFollowupRequest { message: prompt })` first, then call the polling helper. API failure before acceptance should yield an error without polling. Polling errors should surface through the same error path as initial spawn.
For initial spawn, preserve the existing tolerance for a session link without a parsed session ID if any caller still needs that metadata. For follow-up readiness, require a parsed session ID because the hotswap API needs a `SessionId`.
Terminal states before a fresh session is found should not leave the follow-up wait indefinitely. Failure-like states should emit the state change and then surface the task status message as an error; successful terminal completion without a new session should complete with a clear “no follow-up session became available” error.
### Explicit ambient model startup kind
Add a small enum such as `SessionStartupKind { InitialRun, Followup }` and change `Status::WaitingForSession` to carry `{ progress, kind }`. Existing accessors like `agent_progress()` and `is_waiting_for_session()` should remain behavior-preserving.
Add fields to `AmbientAgentViewModel` for follow-up bookkeeping: the active execution `SessionId`, the last ended execution `SessionId` if available, and the currently submitted follow-up prompt. The prompt field is for PR 3’s optimistic rendering; PR 2 should store it but not insert a visible follow-up query block.
Update initial spawn to set `WaitingForSession { kind: InitialRun }`. Update `AmbientAgentEvent::SessionStarted` handling to emit `SessionReady` for `InitialRun` and `FollowupSessionReady` for `Followup`, rather than relying on whether the current status happens to be `AgentRunning`.
Add `AmbientAgentViewModel::submit_cloud_followup(prompt, ctx)`. It should require `FeatureFlag::HandoffCloudCloud`, require an existing `task_id`/run ID, capture the previous active or ended session ID, set `WaitingForSession { kind: Followup }`, start the progress timer, store the pending prompt, emit a distinct follow-up dispatch event, and spawn the follow-up helper stream.
On follow-up success, stop the timer, set `status` to `AgentRunning`, update the active execution session ID, clear the pending prompt, and emit `FollowupSessionReady { session_id }`. On failure, reuse the existing failure/auth/quota/capacity mapping logic as much as possible so follow-up setup errors render through the same state as initial setup errors.
### Execution-ended bookkeeping without visible UI
Extend the ambient session-ended path only enough for bookkeeping. `viewer::TerminalManager::ambient_session_ended` currently leaves the pane resumable and clears the active network in `app/src/terminal/shared_session/viewer/terminal_manager.rs (1490-1515)`. In PR 2 it can notify the ambient view model of the ended session ID behind `HandoffCloudCloud`, so the model records `last_ended_execution_session_id` and can reject duplicate readiness from that session.
This notification should not call `TerminalView::on_session_share_ended`, should not insert a tombstone, should not set `SharedSessionStatus::FinishedViewer`, and should not cancel the local conversation. Those UI and lifecycle decisions remain PR 3 scope.
### Event and view integration
Add a new model event such as `FollowupDispatched` instead of reusing `DispatchedAgent`. `create_cloud_mode_view` only needs an exhaustive-match update for the new event because `FollowupSessionReady` is already wired to `attach_followup_session`.
Update `TerminalView::handle_ambient_agent_event` to handle `FollowupDispatched` by notifying/re-rendering progress UI and marking the active ambient conversation as `ConversationStatus::InProgress` if one exists. It should not insert `CloudModeInitialUserQuery`, should not insert a second `AmbientAgentEntryBlock`, and should not auto-open new UI beyond the existing setup/progress rendering.
The existing loading screen in `app/src/terminal/view/ambient_agent/view_impl.rs (529-571)` can continue to derive messages from `AgentProgress` for PR 2. If copy changes are desired for follow-ups, keep them minimal and keyed off `SessionStartupKind`, but deferring user-facing copy to PR 3 is acceptable.
## Testing strategy
Add stream-level tests in `app/src/ai/ambient_agents/spawn_tests.rs` covering follow-up submission and polling. The important cases are: the helper calls `submit_run_followup` before polling; it ignores the previous session ID returned by the server; it emits `SessionStarted` for a different new session ID; it propagates API errors before polling; it surfaces terminal failure before readiness.
Preserve existing `spawn_task` tests so initial spawn behavior remains unchanged after the refactor.
Add model-level tests if there is a lightweight existing harness for `AmbientAgentViewModel`; otherwise keep model changes small and validate via stream tests plus targeted compile checks. Model assertions should cover `submit_cloud_followup` preconditions, `WaitingForSession { kind: Followup }`, and `FollowupSessionReady` emission on a fresh session.
Run targeted validation after implementation: `cargo nextest run -p warp ai::ambient_agents::spawn::tests server::server_api::ai::tests::build_run_followup_url_routes_to_run_followups server::server_api::ai::tests::serialize_run_followup_request` and `cargo check -p warp --features handoff_cloud_cloud`. If model or terminal-view tests are added, include their module filters. Do not use `cargo fmt --all` or file-specific `cargo fmt`; use the repo’s standard formatting command only when preparing a PR update.
## Rollout and compatibility
With `HandoffCloudCloud` off, no production UI should call the new follow-up method and existing initial Cloud Mode startup should behave as it does today.
With the flag on, PR 2 only exposes an internal/model-level follow-up path. The absence of a visible entrypoint makes this safe to merge before product UX lands, while unit tests can still exercise the orchestration path.
The runtime code may assume `CloudModeSetupV2` when `HandoffCloudCloud` is enabled because the Cargo feature dependency was added in PR 1.
## Risks and mitigations
The server may briefly return the ended execution’s session fields after accepting a follow-up. Mitigate by passing the previous session ID into the polling helper and requiring a different parsed session ID before emitting readiness.
Reusing `DispatchedAgent` for follow-ups would insert initial-run UI artifacts again. Mitigate with a distinct follow-up event and explicit startup kind.
Refactoring `spawn_task` could regress initial Cloud Mode startup. Mitigate by preserving the public stream contract and keeping existing spawn tests green.
A follow-up may be accepted but fail before any session becomes joinable. Mitigate by reusing the existing failed/auth/quota/capacity UI states and leaving future UI free to retry from the tombstone in PR 3.
Model bookkeeping could drift if session-ended notifications are missed. Mitigate by also falling back to the last active execution session ID when submitting a follow-up.
## Parallelization
This PR is small enough to implement sequentially, but two independent tracks could run in parallel if needed. One track can refactor and test `spawn.rs` follow-up polling with mocked `AIClient`; the other can wire `AmbientAgentViewModel` state/events and terminal-manager bookkeeping. They converge at `submit_cloud_followup` consuming the follow-up helper and emitting `FollowupSessionReady`.
## Definition of done
`spawn_task` still behaves the same for initial runs after extracting reusable polling.
A follow-up helper submits a prompt, polls the stable run, ignores stale session IDs, and returns a fresh joinable session.
`AmbientAgentViewModel::submit_cloud_followup` exists behind `HandoffCloudCloud` and drives `WaitingForSession { kind: Followup }` through success and error states.
`FollowupSessionReady` is emitted for fresh sessions and continues to attach through the existing hotswap path.
No tombstone Continue UI or terminal-input follow-up route is added in this PR.
Targeted tests and `cargo check -p warp --features handoff_cloud_cloud` pass.
