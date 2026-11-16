# Cloud-to-cloud handoff PR 1 tech spec
## Problem statement
The master cloud-to-cloud handoff plan needs an initial mergeable PR that adds the scaffolding required by later follow-up orchestration and UI work without changing end-user behavior. PR 1 should make the client aware of the `HandoffCloudCloud` rollout boundary, add a typed client method for the server follow-up API, and start isolating task/run identity from execution/session-scoped data in the client model.
This PR intentionally does not add the tombstone Continue entrypoint, does not submit follow-ups from the UI, and does not attach a follow-up shared session to an existing terminal. Those behaviors belong in later PRs after the foundational APIs exist.
## Current state
The current branch already adds the UI/model layer needed to attach a new backing shared session to an existing shared-session viewer; that foundation is documented in `specs/REMOTE-1478/TECH.md`. The broader sequencing and intended user flow are documented in `specs/handoff-cloud-cloud/TECH.md`.
Feature flags are defined in `crates/warp_features/src/lib.rs`. `CloudModeSetupV2` already exists near the end of the enum at `crates/warp_features/src/lib.rs:827`, and the rollout arrays are defined at `crates/warp_features/src/lib.rs:852`, `crates/warp_features/src/lib.rs:910`, and `crates/warp_features/src/lib.rs:926`. There is currently no `HandoffCloudCloud` flag. The Cargo feature graph in `app/Cargo.toml` is the right place to encode that `handoff_cloud_cloud` depends on `cloud_mode_setup_v2`.
The public API client already has ambient run methods for spawn, list, and get. The `AIClient` trait includes `spawn_agent`, `list_ambient_agent_tasks`, and `get_ambient_agent_task` at `app/src/server/server_api/ai.rs:799`, and the `ServerApi` implementation posts to `agent/run`, lists `agent/runs`, and gets `agent/runs/{task_id}` at `app/src/server/server_api/ai.rs:1404`. Adjacent public API methods post to run-scoped subresources such as `agent/runs/{task_id}/attachments/prepare` at `app/src/server/server_api/ai.rs:1785`, which is the natural implementation pattern for the follow-up API.
The server follow-up API is `POST /api/v1/agent/runs/{runId}/followups` with a JSON body containing `message`. The server route and request type are in the server worktree at `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/public_api/openapi.yaml:644`, `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/router/handlers/public_api/agent_webhooks.go:181`, `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/router/handlers/public_api/agent_webhooks.go:608`, and `/Users/zachbai/.warp-dev/worktrees/warp-server/cloud-agent-task-name/public_api/types/types.gen.go:1225`.
`AmbientAgentTask` currently exposes execution-scoped fields directly: `session_id`, `session_link`, `conversation_id`, `request_usage`, and `is_sandbox_running` at `app/src/ai/ambient_agents/task.rs:217`. `is_no_longer_running` already combines sandbox liveness with run state at `app/src/ai/ambient_agents/task.rs:257`. `SessionJoinInfo::from_task` reads `task.session_link` and `task.session_id` directly at `app/src/ai/ambient_agents/spawn.rs:32`. Agent management and details code also reads these flattened fields directly for session status, links, conversation dedupe, open actions, and details-panel display, including `app/src/ai/agent_conversations_model.rs:468`, `app/src/ai/agent_conversations_model.rs:515`, `app/src/ai/agent_conversations_model.rs:528`, `app/src/ai/agent_conversations_model.rs:553`, `app/src/ai/agent_conversations_model.rs:1293`, and `app/src/ai/conversation_details_panel.rs:337`.
## Goals
Add a disabled `HandoffCloudCloud` feature flag, with the app Cargo feature depending on `cloud_mode_setup_v2`.
Add typed client support for submitting a follow-up prompt to a run via the public API.
Introduce run/execution-aware accessors on `AmbientAgentTask` that preserve current behavior while giving later PRs a place to route active/latest-execution semantics.
Move the most important existing call sites from direct flattened fields to the new accessors when doing so is behavior-preserving and low-risk.
Add targeted unit tests for the new API serialization/path helper and task accessor behavior.
## Non-goals
No visible cloud conversation tombstone changes.
No terminal input submission changes.
No cloud mode setup UI changes.
No follow-up polling or shared-session hotswap orchestration.
No attempt to parse or store a full run-executions array unless the public API already returns it to the client in the current schema. PR 1 should define seams that can absorb that shape later.
No rollout enablement in `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, or `RELEASE_FLAGS`.
## Proposed changes
### Feature flag scaffolding
Add `HandoffCloudCloud` to `FeatureFlag` in `crates/warp_features/src/lib.rs`, preserving the enum's chronological ordering. Keep the description concise and product-focused, for example gating cloud-to-cloud continuation of cloud mode conversations.
Do not add the flag to any rollout arrays in PR 1. The flag should be available for local overrides and future PRs, but disabled by default.
Add `handoff_cloud_cloud = ["cloud_mode_setup_v2"]` to `app/Cargo.toml` and map the Cargo feature to `FeatureFlag::HandoffCloudCloud` in `app/src/lib.rs`. This keeps the dependency explicit in the build feature graph without adding rollout dependency tests.
### Follow-up API client
Add a request type near `SpawnAgentRequest` in `app/src/server/server_api/ai.rs`:
`RunFollowupRequest { message: String }`, serialized as `{"message":"..."}`.
Add an `AIClient` method named `submit_run_followup` with parameters `run_id: &AmbientAgentTaskId` and `request: RunFollowupRequest`, returning `anyhow::Result<(), anyhow::Error>`. The current server behavior does not expose a client-needed response payload, so implement with `post_public_api_unit(&build_run_followup_url(run_id), &request)`.
Add a small path helper, for example `build_run_followup_url(run_id: &AmbientAgentTaskId) -> String`, mirroring `build_list_agent_runs_url` at `app/src/server/server_api/ai.rs:521`. This makes unit testing the endpoint path independent from HTTP mocking.
Do not call this new method from UI or orchestration code in PR 1.
### Run/execution-aware task accessors
Add lightweight accessors to `AmbientAgentTask` in `app/src/ai/ambient_agents/task.rs`. For PR 1 these should project the existing flattened fields as the active/latest execution:
`run_id() -> AmbientAgentTaskId` returning `task_id`.
`conversation_id() -> Option<&str>` returning `conversation_id.as_deref()`.
`active_run_execution() -> RunExecution<'_>` returning a borrowed projection of the flattened execution-scoped fields.
Add `RunExecution<'a>` with borrowed `session_id`, non-empty `session_link`, `request_usage`, and `is_sandbox_running` fields. The key is that callers stop encoding the assumption that task-level fields are intrinsically task-scoped.
Update `is_no_longer_running` to use `active_run_execution().is_sandbox_running` and preserve existing behavior.
Move `SessionJoinInfo::from_task` in `app/src/ai/ambient_agents/spawn.rs:32` to use `active_run_execution()`.
Update low-risk UI/model call sites that only need the current projection:
`ConversationOrTask::session_id` should parse `active_run_execution().session_id`.
`link_preference` should use `active_run_execution().is_sandbox_running`.
`session_or_conversation_link` should use `active_run_execution().session_link` and `conversation_id()`.
`get_session_status` should use the session/link accessors.
`get_open_action` and conversation shadowing should use `run_id()` and `conversation_id()`.
`ConversationDetailsData::from_task` should use `conversation_id()` for the details-panel conversation id and `active_run_execution().request_usage` for credits.
Avoid broad mechanical churn in call sites where fields are clearly task metadata rather than execution data, such as title, prompt, state, source, creator, artifact list, and config snapshot.
### Optional naming cleanup
Do not rename `AmbientAgentTaskId` or public UI labels in PR 1. The server and much of the client still use run/task terminology interchangeably, and a broad rename would create churn without improving mergeability. The new `run_id()` accessor is enough to clarify stable identity for later PRs.
## Testing strategy
Add public API helper tests in `app/src/server/server_api/ai_test.rs` for the follow-up endpoint path. If the request type is public enough to serialize directly, add a serialization test asserting the JSON shape is exactly `{"message":"..."}`.
Add or update ambient task tests in `app/src/ai/ambient_agents/spawn_tests.rs` or a new `app/src/ai/ambient_agents/task_tests.rs` to cover:
`SessionJoinInfo::from_task` still prefers server-provided session links.
It still falls back to constructing a join link from `session_id`.
It returns `None` when neither an active link nor parseable session id exists.
The new accessors preserve current flattened-field behavior.
Run targeted checks after implementation:
`cargo nextest run -p warp server::server_api::ai::tests::build_run_followup_url_routes_to_run_followups server::server_api::ai::tests::serialize_run_followup_request ai::ambient_agents::spawn::tests`
`cargo nextest run -p warp ai::agent_conversations_model::tests ai::conversation_details_panel::tests`
`cargo check -p warp --features handoff_cloud_cloud`
The exact package names should be verified during implementation from `Cargo.toml` before running. Do not use `cargo fmt --all` or file-specific `cargo fmt`; if formatting is needed before review, use the repo’s standard `cargo fmt` per project guidance.
## Rollout and compatibility
The flag is disabled by default, so PR 1 should not change runtime behavior.
The new API client method is unused in PR 1 and should therefore be safe to merge before server rollout, as long as it compiles against existing client code.
The accessor migration should be behavior-preserving because each accessor initially projects the same flattened fields. If a direct field use is ambiguous or risky, leave it in place and document it as follow-up rather than expanding the PR scope.
## Risks and mitigations
The largest risk is accidentally changing management view link selection or session-open behavior while replacing direct field reads. Keep changes small, prefer local accessor substitutions, and rely on existing spawn and agent management tests where available.
The follow-up endpoint response shape may differ from the assumed empty response. During implementation, verify the server contract before choosing `post_public_api_unit`; if the server returns a body, add a minimal response type and test deserialization.
Runtime implication of `HandoffCloudCloud` to `CloudModeSetupV2` is intentionally not implemented in PR 1. The Cargo feature dependency covers compiled builds, while local runtime overrides can still force unusual states for targeted testing.
The accessor names may need to change once the client consumes first-class run-execution data. Keep PR 1 names descriptive but avoid adding a large abstraction that is not backed by current API data.
## Definition of done
`HandoffCloudCloud` exists and is disabled by default.
`handoff_cloud_cloud` in `app/Cargo.toml` depends on `cloud_mode_setup_v2`.
`AIClient` and `ServerApi` expose a typed follow-up submission method for `POST agent/runs/{run_id}/followups`.
`AmbientAgentTask` exposes run/execution-aware accessors and the main session/conversation call sites use them without behavior changes.
Targeted tests cover the follow-up API path/serialization and task/session accessor behavior, and a feature-gated compile check passes.
