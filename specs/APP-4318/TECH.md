# HandoffCloudCloud master tech spec
## Context
Cloud Mode ambient conversations currently treat one ambient task/run as one shared-session execution. The first Cloud Mode pane is created as a deferred shared-session viewer in `app/src/terminal/view/ambient_agent/mod.rs:35`; the ambient model emits `SessionReady`, and the viewer manager joins the session. This branch adds the foundation needed to reuse that same terminal view/model for later sessions: `TerminalManager::attach_followup_session` can replace the active viewer `Network` and join a fresh shared session in append mode in `app/src/terminal/shared_session/viewer/terminal_manager.rs:338`.
The hotswap foundation is documented separately in `specs/REMOTE-1478/TECH.md`. The important shipped boundary is that session IDs are transport-scoped, while the visible ambient conversation, terminal view, terminal model, and task/run identity remain stable across follow-up executions. The event loop already has a follow-up load mode, and `TerminalModel::append_followup_shared_session_scrollback` appends only unknown block IDs instead of replacing the blocklist in `app/src/terminal/shared_session/viewer/event_loop.rs:29`, `app/src/terminal/model/terminal_model.rs:1481`, and `app/src/terminal/model/blocks.rs:759`.
The remaining client work is broader than a small change. It touches the public API client, ambient task/run data models, ambient spawn/follow-up state transitions, the cloud conversation tombstone, terminal input behavior, and tests. This should be implemented as a stack of mergeable PRs rather than one large PR.
The server-side public API now exposes `POST /api/v1/agent/runs/{runId}/followups`, which accepts `RunFollowupRequest { message }` and returns an empty success object when the follow-up is accepted. The server contract says clients should observe readiness by fetching `GET /api/v1/agent/runs/{runId}` until the run exposes an active shared session. The route and handler are in `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/router/handlers/public_api/agent_webhooks.go:181` and `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/router/handlers/public_api/agent_webhooks.go:608`; the OpenAPI schema is in `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/public_api/openapi.yaml:644`.
Run executions are a new server abstraction. A stable run/task can have many execution attempts, each with its own input, state, shared session, conversation ID, and compute accounting. The server model is in `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/model/types/ai_run_executions.go:37`. The current client is not execution-aware: `AmbientAgentTask` stores one `session_id`, one `session_link`, one `conversation_id`, and one `is_sandbox_running` value in `app/src/ai/ambient_agents/task.rs:213`; `AIConversation` stores a single `task_id`/`run_id` in `app/src/ai/agent/conversation.rs:123`; `spawn_task` polls a task until it sees a single session ID in `app/src/ai/ambient_agents/spawn.rs:27`.
The Cloud Mode tombstone already exists and can render artifacts plus “Continue locally” from `ConversationEndedTombstoneView` in `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs:139`. It is currently inserted by the generic shared-session end path in `TerminalView::on_session_share_ended` in `app/src/terminal/view/shared_session/view_impl.rs:685`. The new ambient hotswap path intentionally avoids that full teardown, so the follow-up implementation needs a dedicated ambient-execution-ended UI path that inserts the tombstone without making the viewer read-only or permanently finished.
The feature must be gated behind a new client feature flag, `HandoffCloudCloud`. Enabling `HandoffCloudCloud` must imply `CloudModeSetupV2` is enabled. Runtime code may assume `CloudModeSetupV2` behavior whenever `HandoffCloudCloud` is enabled, so new call sites should check only `FeatureFlag::HandoffCloudCloud` unless they are still preserving old setup-v1 behavior.
## Proposed changes
### Feature flag and rollout invariant
Add `FeatureFlag::HandoffCloudCloud` in `crates/warp_features/src/lib.rs` near `CloudModeSetupV2`. Do not add it to release/preview/dogfood lists until the full flow is ready for that audience. When it is added to any rollout list, add `CloudModeSetupV2` to the same list if it is not already there.
Add a small feature-flag test or helper assertion that encodes the dependency: any static rollout set containing `HandoffCloudCloud` must also contain `CloudModeSetupV2`. Runtime feature checks should assume that dependency rather than repeatedly checking both flags.
### Run/execution-aware client model
Introduce explicit run identity and execution-scoped types in the ambient agent model layer. A minimal shape is:
- `AmbientAgentRunId` or reuse `AmbientAgentTaskId` with clearer accessors while server/client naming finishes migrating from task to run.
- `AmbientAgentRunExecutionId` for server execution IDs when the public API starts returning them.
- `AmbientAgentRunExecutionSummary`, containing execution ID if present, state, shared session ID/link, conversation ID, started/updated timestamps, and whether the execution is active.
- `AmbientAgentRun`, or an evolved `AmbientAgentTask`, that exposes stable run fields separately from active/latest execution projections.
For the first client PR, the public API may still return flattened fields. The model should still expose execution-aware APIs that use those fields as the active/latest execution projection:
- `run_id()`
- `active_execution_session_id()`
- `latest_execution_session_id()`
- `active_execution_conversation_id()`
- `has_active_execution()`
- `is_terminal_run_state()`
- `can_submit_cloud_followup()`
Existing callers should stop reaching directly into `session_id`, `session_link`, `conversation_id`, and `is_sandbox_running` when the meaning is execution-scoped. That keeps later server responses with an `executions` array or `active_execution` object from forcing another cross-codebase rename.
`AIConversation` should keep stable conversation identity by server conversation token/conversation ID, while treating run/execution metadata as auxiliary. The current single `task_id` field can remain as the stable run ID for compatibility, but methods and comments should distinguish “run ID” from any future “execution ID”. Do not make a follow-up execution allocate a new local `AIConversationId`; following up continues the same local conversation and same server conversation/run.
`AgentConversationsModel` should merge fetched run data by stable run ID. Details panels and management lists can continue to show one row per run, with aggregate state from the run and active/latest execution session info for open/continue actions.
### Public API client
Add `RunFollowupRequest { message }` and `AIClient::submit_run_followup(run_id, message)` in `app/src/server/server_api/ai.rs`. Implement it as `POST agent/runs/{run_id}/followups` against the public API, returning `Result<(), anyhow::Error>`.
Split the current `spawn_task` stream into reusable pieces:
- `spawn_task(request, ai_client, timeout)` continues to create the initial run and monitor it.
- A new monitor helper polls an existing run until it reaches either a terminal error state or a new joinable execution session.
- A new follow-up helper calls `submit_run_followup`, then uses the monitor helper to wait for the next active execution session.
The follow-up monitor must know the previous execution session ID and ignore it. A ready follow-up session is a session ID that is present, parseable, active according to the run/execution projection, and different from the previous ended session ID.
### Ambient view model state
Extend `AmbientAgentViewModel` to track stable run state and per-execution startup state:
- stable run ID/task ID;
- local conversation ID;
- active execution session ID;
- last ended execution session ID;
- current startup kind: initial run or follow-up execution;
- the prompt currently being submitted for optimistic rendering.
Replace the implicit “if already `AgentRunning`, then a `SessionStarted` means follow-up” logic in `app/src/terminal/view/ambient_agent/model.rs:606` with explicit state. A good shape is `Status::WaitingForSession { progress, kind }`, where `kind` is `Initial` or `Followup`.
Add a public model method such as `submit_cloud_followup(prompt, ctx)`. It should:
1. require `HandoffCloudCloud`;
2. require an existing run ID;
3. record the follow-up prompt for optimistic rendering;
4. set `Status::WaitingForSession { kind: Followup }`;
5. emit a setup/loading event so the terminal shows the existing Cloud Mode setup UI;
6. call the follow-up API;
7. poll for the new session;
8. emit `FollowupSessionReady { session_id }` when ready;
9. surface errors through the same loading/error UI path as initial Cloud Mode setup.
When the follow-up session becomes ready, keep using `TerminalManager::attach_followup_session` rather than opening a new terminal view. When the follow-up execution starts, update the existing conversation status back to `ConversationStatus::InProgress`; when it finishes, update status based on the run state.
### Ambient execution-ended UI boundary
Add a dedicated ambient execution-ended path instead of using the generic shared-session teardown. The current branch’s `ambient_session_ended` in `app/src/terminal/shared_session/viewer/terminal_manager.rs:1516` should trigger a terminal-view method such as `on_ambient_agent_execution_ended`.
That terminal-view method should:
- insert the conversation-ended tombstone once per ended execution, or update the existing tombstone to represent the latest ended execution;
- keep the existing `TerminalView`, `TerminalModel`, ambient view model, and input model alive;
- keep the terminal pane eligible to attach another shared session;
- not set `SharedSessionStatus::FinishedViewer`;
- not put the editor into permanent read-only/selectable state;
- not cancel the local ambient conversation merely because the execution ended.
If multiple follow-ups are supported, the tombstone insertion policy should avoid stacking multiple large terminal-state cards in a way that buries the active input. The simplest acceptable policy for the first UI PR is to append a tombstone after each terminal execution, because each one marks a real boundary in the transcript. If that feels noisy in implementation review, the alternative is to keep one tombstone view and update it in place until the user submits the next follow-up.
### Tombstone Continue entrypoint and terminal input
Under `HandoffCloudCloud`, `ConversationEndedTombstoneView` should show a primary “Continue” action for ambient Cloud Mode tombstones that have a run ID and are eligible for cloud follow-up. Keep “Continue locally” as a secondary or fallback action.
Clicking “Continue” should reveal/focus the terminal input for a follow-up prompt. Prefer reusing the existing terminal input/editor rather than embedding a separate text editor inside the tombstone. That keeps submission, attachments, editor state, and keyboard behavior consistent with Cloud Mode setup.
The submit path should route through `AmbientAgentViewModel::submit_cloud_followup`, not through the shared-session viewer network. The previous shared session has ended, so there is no active sharer to receive `SendAgentPrompt`. The new run execution is created by the public follow-up API; once its shared session is ready, the viewer network is attached.
During setup, reuse the Cloud Mode setup-v2 loading screen and progress footer/screen instead of inventing a new progress UI. The initial-user-query rich content pattern in `CloudModeInitialUserQuery` can be generalized to render optimistic follow-up user prompts while the environment is starting.
### Conversation and blocklist continuity
Following up must preserve the same local `AIConversationId`, same server conversation ID, and same stable run ID. The new execution creates a new shared session and a new execution record, not a new user-visible conversation.
When the new session joins, append session scrollback using `SharedSessionInitialLoadMode::AppendFollowupScrollback`. The server/runtime contract should preserve `SerializedBlock.id` for prior rehydrated blocks or send continuation-only scrollback. Without one of those contracts, the client cannot reliably dedupe prior output from new output.
The existing hotswap append path should be treated as the only blocklist mutation path for follow-up execution output. Avoid separately loading conversation transcript data into the same view during follow-up setup, because that risks duplicating AI blocks and shell command blocks.
### Error handling
If `submit_run_followup` fails before the server accepts the prompt, restore the tombstone/input to an editable state and show the error in the same Cloud Mode setup area. Do not append the optimistic follow-up prompt permanently unless the server accepted it.
If the server accepts the follow-up but polling reaches a terminal failure before a new session is available, show the Cloud Mode error/cancelled/auth/capacity state and leave the tombstone/input available for retry when the server marks the run retryable.
If the user closes the pane while waiting for a follow-up session, cancel only local polling. Do not cancel the run unless the user explicitly invokes a cancel action.
## End-to-end flow
1. User starts a Cloud Mode ambient conversation.
2. The initial run is created with `POST /agent/run`; the client stores the stable run ID and local conversation ID.
3. `spawn_task` polls until the run exposes the initial active execution shared session.
4. `AmbientAgentViewModel` emits `SessionReady`; the viewer manager joins the initial session.
5. The execution reaches a terminal state and the shared session ends.
6. The viewer manager handles this as an ambient execution boundary, inserts or updates the tombstone, and keeps the pane/input resumable.
7. User clicks “Continue” on the tombstone and submits a prompt through the terminal input.
8. The ambient view model calls `POST /agent/runs/{runId}/followups`.
9. The client shows Cloud Mode setup-v2 loading UI while polling `GET /agent/runs/{runId}`.
10. When a new active execution session ID appears, `AmbientAgentViewModel` emits `FollowupSessionReady`.
11. The viewer manager calls `attach_followup_session`, replaces the network, and joins the new session in append mode.
12. New output streams into the same terminal view/model and same local conversation.
13. Steps 5-12 can repeat for additional follow-up executions.
## Increment plan
### PR 0: shared-session hotswap foundation
This is the current branch and can stand alone. It keeps reusable viewer resources across networks, adds `attach_followup_session`, adds append-mode scrollback loading, and prevents ambient `SessionEnded` from permanently poisoning the pane. The existing `specs/REMOTE-1478/TECH.md` covers this increment.
### PR 1: feature flag, API client, and run/execution-aware model scaffolding
Add `HandoffCloudCloud`, `submit_run_followup`, follow-up request/response tests, and run/execution-aware accessors on `AmbientAgentTask`/run models. Update existing call sites to use accessors for active session/conversation state. This PR should not expose UI or change runtime behavior except behind tests and the disabled flag.
Merge criteria: existing Cloud Mode spawn, task list, details panel, and shared-session viewer behavior are unchanged with the flag off.
### PR 2: ambient follow-up orchestration without the visible entrypoint
Add `AmbientAgentViewModel::submit_cloud_followup`, explicit initial-vs-follow-up waiting state, polling for a new active session, optimistic follow-up prompt state, and `FollowupSessionReady` wiring to the existing hotswap API. Add unit tests using a mocked `AIClient`.
This can be mergeable behind the disabled flag with a test-only or debug-only invocation path. No tombstone button is required yet.
Merge criteria: a model-level follow-up can accept a prompt, call the API, ignore the previous session ID, emit a new session ID, and handle API/polling errors.
### PR 3: tombstone Continue UX and terminal input submission
Add the cloud “Continue” tombstone action, reveal/focus the existing terminal input, route submission to the ambient follow-up model method, show setup-v2 loading UI, and render the optimistic follow-up prompt. Keep “Continue locally” available. Add telemetry for continue-in-cloud attempts, success, and failures.
Merge criteria: with `HandoffCloudCloud` off, the tombstone is unchanged; with it on, terminal-state Cloud Mode conversations can start a follow-up and attach the new shared session in the same pane.
### PR 4: polish, details panel, and end-to-end validation
Update conversation details/agent-management surfaces to use run/execution-aware helpers, ensure active/past sections treat a run with an active follow-up execution as active, and add integration coverage for at least two execution boundaries. This PR can also tune tombstone stacking/updating based on product review.
Merge criteria: repeated cloud follow-ups preserve one conversation/run identity, append output in order, and do not regress normal shared-session viewers or local “Continue locally”.
## Testing and validation
Unit tests:
- `AIClient::submit_run_followup` constructs `POST agent/runs/{runId}/followups` with `{ message }` and handles success/error responses.
- Run/execution accessors derive active/latest session state from current flattened API fields and from future optional execution-shaped test fixtures.
- Ambient follow-up model calls the API, transitions to `WaitingForSession { kind: Followup }`, polls until a new session ID appears, ignores the old session ID, emits `FollowupSessionReady`, and handles terminal failure states.
- `ConversationEndedTombstoneView` shows/hides Continue based on `HandoffCloudCloud`, task/run presence, AI settings, and target platform.
- Non-ambient shared-session `SessionEnded` still uses the generic finished/read-only viewer path.
Viewer/session tests:
- Follow-up attach replaces the active network and does not duplicate outbound subscriptions.
- Ambient `SessionEnded` inserts or updates a tombstone without setting `FinishedViewer`.
- Repeated follow-up sessions append scrollback without duplicating block IDs.
Integration or manual validation:
- Start a Cloud Mode conversation, wait for the execution to end, click Continue, submit a prompt, verify setup UI appears, then verify a fresh shared session attaches to the same pane.
- Repeat with a second follow-up to catch subscription leaks and stale session-ID handling.
- Verify “Continue locally” still forks locally from the same tombstone.
- Verify a normal shared-session viewer still becomes read-only and shows the ended banner when its session ends.
Before opening or updating PRs in this stack, run the repo-required formatting and clippy checks for touched Rust code, plus targeted tests for ambient model, server API client, tombstone view, and viewer terminal manager. For the user-facing UI increments, run UI verification with the `verify-ui-change-in-cloud` skill after implementation.
## Risks and mitigations
### Run/execution naming churn
Risk: continuing to use `task_id` everywhere makes execution-scoped changes confusing and easy to misuse.
Mitigation: add accessors and comments that separate stable run identity from execution-scoped session/conversation state, even if the underlying ID type remains `AmbientAgentTaskId` during migration.
### Stale session readiness
Risk: after submitting a follow-up, `GET /agent/runs/{runId}` may briefly return the ended execution’s previous session ID.
Mitigation: the follow-up monitor must track the previous session ID and require a different active execution session before emitting `FollowupSessionReady`.
### Duplicate transcript content
Risk: follow-up sessions may replay prior scrollback, and client-side transcript restoration may also try to render conversation data.
Mitigation: use only append-mode shared-session scrollback in the live pane, dedupe by block ID, and require the runtime/server to preserve block IDs or send continuation-only scrollback.
### Input routed to the wrong transport
Risk: the terminal input could accidentally send a prompt over the old shared-session `NetworkEvent::SendAgentPrompt` path.
Mitigation: while between executions, route submission through the ambient follow-up API path and keep `current_network` empty until a new session is attached.
### Feature flag dependency drift
Risk: `HandoffCloudCloud` could be enabled without setup-v2, exposing code paths that assume setup-v2 UI/state.
Mitigation: encode the rollout-list dependency in a test and document that runtime code only checks `HandoffCloudCloud`.
### Tombstone noise
Risk: repeated executions can append multiple large tombstones.
Mitigation: start with the simple append-per-execution behavior only if it reads well in product review; otherwise update the latest tombstone in place until the next follow-up is submitted.
## Parallelization
The work can split across three agents or branches after PR 1 lands:
- API/model track: follow-up client method, run/execution accessors, agent management/details model updates.
- Ambient orchestration track: view-model follow-up state machine, polling, and hotswap event wiring.
- UI track: tombstone Continue action, terminal input reveal/submission, optimistic prompt rendering, and setup-v2 loading/error states.
The tracks converge at `AmbientAgentViewModel::submit_cloud_followup` and the existing `FollowupSessionReady -> attach_followup_session` subscription in `create_cloud_mode_view`.
## Follow-ups
- Add first-class execution arrays or active/latest execution objects to the public API response once the server shape is ready, then remove flattened-field compatibility from client accessors.
- Decide whether conversation details should show per-execution runtime/credit rows or only aggregate run-level totals.
- Consider adding execution IDs to session-sharing source metadata so the client can associate a joined session with a specific execution without inferring from session ID.
- Remove the `HandoffCloudCloud` flag after cloud-to-cloud follow-ups are stable.
