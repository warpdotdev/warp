# Empty-Prompt Local-to-Cloud Handoff (REMOTE-1499) — Tech Spec
This is the canonical tech spec for the warp-4 side of the empty-prompt local-to-cloud handoff feature. It describes the full end-to-end architecture across every stage of implementation. Per-stage sub-tech-specs describe what each stage delivers: `STAGE-1.md` lives on this branch (`harry/empty-prompt-handoff-wire-contract`); `STAGE-2.md` is added on the stacked branch (`harry/empty-prompt-handoff-local`). This document is the single source of truth for the whole warp-4 architecture and is intentionally not modified by subsequent stages.
Cross-repo siblings:
- `../../../warp-server-4/specs/REMOTE-1499/` — server-side relaxations + worker-derived skip-initial-turn.
- `../../../oz-agent-worker/specs/REMOTE-1499/` — self-hosted worker side of skip-initial-turn.
- `../../../session-sharing-protocol/specs/REMOTE-1499/` — `AmbientSetupPhaseEnded` variant on `OrderedTerminalEventType`.
- `../../../session-sharing-server/specs/REMOTE-1499/` — testing-only protocol dep swap.
Stack layout in this repo:
- Stage 1 base: `harry/empty-prompt-handoff-wire-contract`. Wire-contract widening only — `SpawnAgentRequest.prompt` becomes `Option<String>`. No runtime behavior change in warp-4 except the CLI skill-only `None` emission. Documented in `STAGE-1.md`.
- Stage 2 head: `harry/empty-prompt-handoff-local`, stacked on Stage 1. Adds the local→cloud client behavior (entry points + guardrail + client-side substitution), drops the indicator enum, and wires the `AmbientSetupPhaseEnded` setup-phase teardown marker. Documented in `STAGE-2.md`.
## Architecture overview
The feature is composed of four orthogonal sub-systems on the warp-4 side, all gated behind `FeatureFlag::EmptyPromptHandoff` once Stage 2 lands:
1. **Wire-contract widening** — `SpawnAgentRequest.prompt` is `Option<String>` so the client can omit the field on the wire (Stage 1).
2. **Entry-point unification + client-side substitution** — three entry points (chip / `&` / `/handoff`) gated by the source-content guardrail funnel through `start_local_to_cloud_handoff`; `build_handoff_spawn_request` substitutes `"Continue"` against in-progress sources and `"Apply the workspace changes from my previous session."` against idle sources with a non-empty snapshot token. The view renders the wire prompt verbatim (display = wire) (Stage 2).
3. **Worker-derived skip-initial-turn** — the AgentDriver reads the `--skip-initial-turn` CLI flag (set by the worker on a per-execution basis) rather than destructuring a stored client-side bool (Stage 2).
4. **`AmbientSetupPhaseEnded` setup-phase teardown** — the AgentDriver emits a shared-session-protocol marker after env setup completes; the viewer event loop tears down the Cloud Mode Setup V2 UI on receipt (Stage 2).
## 1. Wire-contract widening (Stage 1)
`SpawnAgentRequest` in `app/src/server/server_api/ai.rs:205-207`:
```rust path=/Users/harryalbert/warp-4/app/src/server/server_api/ai.rs start=205
#[serde(skip_serializing_if = "Option::is_none")]
pub prompt: Option<String>,
```
`Option<T>` serializes transparently under serde: `Some("hello")` emits `"prompt": "hello"`, and `skip_serializing_if = "Option::is_none"` causes `None` to omit the field entirely. The struct derives only `Serialize`, not `Deserialize`, so wire compatibility only needs to hold client→server.
All twelve `SpawnAgentRequest { … }` literal construction sites wrap their prompt value in `Some(...)`. The two interactive submit paths in `app/src/terminal/view/ambient_agent/model.rs:666-717` and `:1187` wrap the result of `extract_user_query_mode(prompt)`. `app/src/pane_group/pane/terminal_pane.rs:2137` wraps the orchestration child's `request.prompt`. Fixtures in `spawn_tests.rs`, `model_tests.rs`, `view_tests.rs`, `mcp_config_tests.rs`, and `ai_tests.rs` follow the same shape.
The CLI path at `app/src/ai/agent_sdk/ambient.rs:267-313` is the only non-test site that emits `prompt: None` at runtime: skill-only and saved-prompt-only invocations propagate `Option<String>` through the resolution branch. The extracted `(prompt, mode)` pair is computed by a `match` at `ambient.rs:474-480` that runs `extract_user_query_mode` only on the `Some` branch and defaults `mode` to `UserQueryMode::Normal` when the prompt is `None`.
Two reader sites use `.as_deref()` so the `Some` case dereferences to `&str` and the `None` case short-circuits cleanly:
- `app/src/terminal/view/ambient_agent/block/entry.rs:160` — entry-block title fallback chain: `request.prompt.as_deref().and_then(Self::meaningful_title)`.
- `app/src/terminal/view/ambient_agent/view_impl.rs:154-189` — Cloud Mode Setup V2 queued-prompt insertion: `request.prompt.as_deref().map(|prompt| display_user_query_with_mode(request.mode, prompt))`. The existing `if !prompt.is_empty()` guard suppresses the block on `None`.
No other code in warp-4 pattern-matches or destructures `SpawnAgentRequest.prompt`.
## 2. Entry-point unification + client-side substitution
### 2.1 Feature flag
`FeatureFlag::EmptyPromptHandoff` is added to `crates/warp_features/src/lib.rs`. Default off. Not added to `DOGFOOD_FLAGS` initially.
### 2.2 Source-content guardrail
`crate::ai::blocklist::handoff::source_conversation_has_content(terminal_view_id, ctx) -> bool` returns true when `BlocklistAIHistoryModel::active_conversation(terminal_view_id)` returns `Some(_)` AND the conversation has at least one exchange (`!conversation.is_empty()`). The three entry points call this helper before deciding to dispatch the immediate-handoff path.
### 2.3 Three entry points
When `EmptyPromptHandoff` is on and the guardrail passes, all three entry points converge in `start_local_to_cloud_handoff` (`app/src/workspace/view.rs:13652-13663`), which synthesizes an empty `PendingCloudLaunch { prompt: "".to_owned(), attachments: vec![] }`.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2547-2574` `OpenHandoffPane` action: when `EmptyPromptHandoff` is on AND the guardrail passes, dispatches `WorkspaceAction::OpenLocalToCloudHandoffPane { launch: None, environment_id: None, entry_point: HandoffEntryPoint::FooterChip }` directly. The chip skips `&` compose mode. **Fallback (guardrail fails or flag off):** emits the legacy `OpenHandoffPane` event so the source input enters `&` compose mode.
- `app/src/terminal/input.rs:4060-4067`: the empty-prompt early-return in `maybe_launch_cloud_handoff_request` keys on `!(EmptyPromptHandoff.is_enabled() && source_conversation_has_content(...))`; when both conditions hold the early-return is bypassed and `PendingCloudLaunch { prompt: "".to_owned(), attachments }` is built. **Fallback:** swallows the Enter (no-op) so the compose draft is preserved.
- `app/src/terminal/input/slash_commands/mod.rs:924-940` `/handoff` with no argument: when `EmptyPromptHandoff` is on AND the guardrail passes, dispatches the same `OpenLocalToCloudHandoffPane { launch: None, ... }` as the chip. **Fallback when `EmptyPromptHandoff` is on but the guardrail fails:** no-op (no compose-mode activation). **Fallback when `EmptyPromptHandoff` is off:** legacy compose-mode activation via `activate_cloud_handoff_compose(HandoffEntryPoint::SlashCommand, ctx)`.
### 2.4 Wire-level substitution (display = wire)
`app/src/terminal/view/ambient_agent/model.rs:686-743` `build_handoff_spawn_request`:
- When the submitted prompt is empty AND `pending_handoff.source_conversation_in_progress` is true: substitute `prompt: Some("Continue")` on the wire.
- Else when the submitted prompt is empty AND `initial_snapshot_token` is `Some(non-empty)`: substitute `prompt: Some("Apply the workspace changes from my previous session.")` on the wire. The snapshot token still rides on the wire alongside.
- Else when the submitted prompt is empty: send `prompt: None` on the wire. The worker derives `--skip-initial-turn` from the execution input.
- Else (non-empty submitted prompt): pass it through unchanged.
`source_conversation_in_progress` is captured once at handoff initiation by reading `BlocklistAIHistoryModel::active_conversation(...).status()` and stored on the `PendingHandoff` struct so it can't drift mid-flow. `initial_snapshot_token` is passed in by `submit_handoff` after the async snapshot upload settles; `queue_handoff_auto_submit` always passes `None` for the token (the queue path's `self.request` is the initial UI-display request, not the final spawn request — the snapshot rehydration substitution only fires once `submit_handoff` builds the final request with the real token).
### 2.5 view_impl rendering (no indicator enum)
`app/src/terminal/view/ambient_agent/view_impl.rs:154-189` inserts a queued-prompt block whose text is the wire prompt rebuilt by `display_user_query_with_mode(request.mode, prompt)` from `request.prompt.as_deref()`. The existing `if !prompt.is_empty()` guard suppresses the block when the wire prompt is `None`. There is no `EmptyPromptHandoffIndicator` enum — display tracks the wire one-to-one.
### 2.6 Telemetry extensions
`app/src/ai/ambient_agents/telemetry.rs`:
- `CloudAgentTelemetryEvent::HandoffInitiated` gains `empty_prompt: bool` and `injection_path: HandoffInjectionPath { None | Continue | SnapshotRehydration }`.
- New `CloudAgentTelemetryEvent::HandoffSnapshotPrepared { had_snapshot: bool }` fires after `derive_touched_workspace` settles.
The `injection_path` variant is selected at handoff initiation based on the same logic the wire substitution uses: non-empty prompt → `None`; empty + in-progress source → `Continue`; empty + idle source → `SnapshotRehydration`. `HandoffSnapshotPrepared` reports whether the actual snapshot derivation produced content, so analytics can detect cases where the `SnapshotRehydration` intent fell back to `prompt: None` on the wire (idle source + empty snapshot).
## 3. Worker-derived skip-initial-turn (client-side)
The decision "should the cloud agent skip its initial LLM turn?" is computed fresh per execution by `common.ShouldSkipInitialTurn(task, execution)` in warp-server-4. With the client-side snapshot-rehydration substitution always carrying a non-empty prompt when a snapshot is present, the helper simplifies to `execution.Input.Prompt.is_empty()` — the `InitialSnapshotToken` check is no longer needed. The result reaches the sandboxed CLI as the `--skip-initial-turn` flag; the worker is the only producer of that flag and the AgentDriver is the only consumer. The flag is the entire worker→driver contract.
On the warp-4 side, the relevant client-side architecture is:
### 3.1 `SpawnAgentRequest` does not carry the flag
`app/src/server/server_api/ai.rs` — `SpawnAgentRequest` does not include a `skip_initial_turn` field. The client never derives or transmits this signal; the wire shape between client and server is silent on it.
### 3.2 `build_handoff_spawn_request` does not derive the flag
`app/src/terminal/view/ambient_agent/model.rs:686-743` — the function decides only the wire-level prompt (per Section 2.4). It does not compute a `skip_initial_turn` value. `spawn_agent` at `model.rs:1187` likewise emits no `skip_initial_turn` value.
### 3.3 `AgentRunPrompt::ServerSide` shape
`app/src/ai/agent_sdk/driver.rs:405-410` — the `ServerSide` variant carries only `skill: Option<ParsedSkill>` and `attachments_dir: Option<String>`. The skip-initial-turn signal is intentionally not part of the prompt variant because the prompt variant must round-trip through `prepare_harness` (which is harness-agnostic) without ferrying a flag that's meaningful only on the Oz harness.
### 3.4 `AgentDriverOptions` and `AgentDriver` field
`app/src/ai/agent_sdk/driver.rs` — `AgentDriverOptions` and `AgentDriver` each carry a `skip_initial_turn: bool` field. The value is sourced from `RunAgentArgs::skip_initial_turn` (which the clap parser populates from `--skip-initial-turn`) by the `build_driver_options_and_task` closure in `mod.rs:855` and threaded into `AgentDriver::new`. The `new_for_test` constructor initializes the field to `false`.
### 3.5 Skip-branch gate in `execute_run`
`app/src/ai/agent_sdk/driver.rs:2410-2411` — the gate is
```rust path=null start=null
let is_skip_initial_turn =
    self.skip_initial_turn && matches!(&task_prompt, AgentRunPrompt::ServerSide { .. });
```
The `ServerSide` match gates the flag onto the Oz harness path — third-party harnesses always resolve the server-side prompt through `prepare_harness` and ignore the flag by construction. The skip-branch body (enter agent view, emit `AmbientSetupPhaseEnded`, schedule deferred `Success` via `IdleTimeoutSender::complete_with_optional_idle`) is described in Section 4.
### 3.6 CLI flag
`crates/warp_cli/src/agent.rs:368-372` — `--skip-initial-turn` is a `requires = "task_id"` boolean flag on `oz agent run`. The CLI parser test at `crates/warp_cli/src/lib_tests.rs:233-252` pins its parsing and constraint.
### Considered alternatives
- **Storing the decision on the task config snapshot at dispatch time.** Rejected: a single stored decision drifts across executions. A cloud→cloud follow-up that submits a non-empty prompt against the same task would inherit the stamped flag and incorrectly skip the LLM turn. Computing fresh per execution makes the decision reactive to the current execution input and trivially supports future content sources without changes outside `ShouldSkipInitialTurn`.
- **Deriving the flag client-side and shipping it on `SpawnAgentRequest`.** Rejected for the same reason: the client only sees the first execution, so it has no input to base subsequent executions on. Centralizing on the server keeps the worker, the wire shape, and the driver simple and keeps the policy in one place.
## 4. `AmbientSetupPhaseEnded` setup-phase teardown
Every cloud agent run signals "environment setup phase complete" via the `OrderedTerminalEventType::AmbientSetupPhaseEnded` shared-session-protocol marker. The sharer emits the marker once setup commands have finished; the viewer's event loop receives it and tears down the Cloud Mode Setup V2 UI. The marker is path-agnostic — it fires on both the skip-initial-turn path (no first LLM turn) and the normal `ServerSide` path (a first LLM turn follows). This makes the setup-phase teardown independent of whether a first `AppendedExchange` event will ever fire.
### 4.1 Testing-only Cargo.toml swap
`Cargo.toml:248` — the `session-sharing-protocol` dep is set to `path = "../session-sharing-protocol"` while the protocol PR is in flight. Reverted to `git = ..., rev = <merged SHA>` after the protocol PR merges; the locally-running session-sharing-server must pick up the same `rev` bump before warp-4 lands, or the relay will type-decode `OrderedTerminalEventType` against an older protocol rev that lacks the new variant and silently drop the marker.
### 4.2 `TerminalModel` helper
`app/src/terminal/model/terminal_model.rs` — `send_ambient_setup_phase_ended_for_shared_session` is modeled on the adjacent `send_agent_conversation_replay_started_for_shared_session`. The helper is a no-op for non-sharer terminals; sharer terminals emit a typed `OrderedTerminalEventType::AmbientSetupPhaseEnded` event through the existing shared-session event channel.
### 4.3 `AgentDriver::execute_run` structure
`app/src/ai/agent_sdk/driver.rs` — `execute_run` is structured so the skip branch builds `IdleTimeoutSender` first, runs the skip block (`enter_agent_view` + emit marker via `send_ambient_setup_phase_ended_for_shared_session` + `complete_with_optional_idle`) before the history subscription, sets up the subscription, then conditionally dispatches the non-skip prompt. Scheduling the timer before the subscription means a later `AppendedExchange` from a session-sharing-protocol follow-up correctly invalidates the timer via `IdleTimeoutSender`'s internal generation counter.
### 4.4 `IdleTimeoutSender::complete_with_optional_idle`
`IdleTimeoutSender::complete_with_optional_idle(idle_on_complete, value)` defers via `end_run_after` when `idle_on_complete` is `Some(d)` and falls back to `end_run_now` when `None`. The `UpdatedConversationStatus` and harness-exit branches in `execute_run` use the same helper so all completion paths honor the optional idle window uniformly.
### 4.5 Viewer event_loop.rs arm
`app/src/terminal/shared_session/viewer/event_loop.rs` — the `OrderedTerminalEventType::AmbientSetupPhaseEnded` arm flips `BlockList::set_is_executing_oz_environment_startup_commands(false)`, then calls `AmbientAgentViewModel::tear_down_active_setup_command_group` which runs `finish_setup_command_group` + `set_setup_command_group_visibility(false)`. "No active group" is a no-op for idempotency. The arm is path-agnostic — it handles both the skip-initial-turn path and the normal cloud agent path.
### 4.6 Non-skip ServerSide marker emission
`app/src/ai/agent_sdk/driver.rs:2760-2792` `AgentDriver::execute_run` non-skip `AgentRunPrompt::ServerSide` arm: after the existing `terminal.enter_agent_view(None, restored_conversation_id, AgentViewEntryOrigin::Cli, ctx)` call and before the `terminal.ai_controller().update(...)` block that fires `AIAgentInput::StartFromAmbientRunPrompt`, the arm invokes `terminal.model.lock().send_ambient_setup_phase_ended_for_shared_session()`. This mirrors the skip-path emission. The `AgentRunPrompt::Local` arm intentionally does not call the helper — local runs don't have a setup phase.
### 4.7 Legacy fallback teardowns
Two `BlocklistAIHistoryEvent::AppendedExchange`-driven teardowns at `app/src/terminal/view.rs:5496-5507` and `app/src/terminal/view/ambient_agent/block/setup_command_text.rs:119-136` remain as a compatibility fallback for viewers that connect to sharers running pre-feature builds. Both teardowns are idempotent with the `AmbientSetupPhaseEnded` arm, so a new sharer + new viewer pair triggering both is harmless. Removal is tracked in PRODUCT.md "Deferred follow-ups".
### Considered alternatives
- **Reusing `AppendedExchange` as the teardown signal everywhere.** Rejected: the skip-initial-turn path never fires `AppendedExchange`, so any teardown that depends on it would leave the cloud pane stuck in the "setting up…" UI. A dedicated marker decouples setup-phase teardown from first-LLM-turn semantics.
- **Letting the AgentDriver send `Success` directly to the oneshot on the skip path (no `IdleTimeoutSender` involvement).** Rejected: the driver tears down ~80ms after sending `Success`, which is too fast for a follow-up session-sharing-protocol exchange to arrive. Routing the skip path through `IdleTimeoutSender::complete_with_optional_idle` honors `idle_on_complete` uniformly across completion paths.
## Testing strategy
Per-stage details live in `STAGE-1.md` and `STAGE-2.md`. High-level:
- Stage 1 covers serialization round-trip tests on `SpawnAgentRequest { prompt: None }` and updated borrow-site fixtures.
- Stage 2 covers behavioral tests on empty-prompt auto-submit and submit_handoff in `app/src/terminal/view/ambient_agent/model_tests.rs` (three substitution outcomes: `"Continue"`, snapshot-rehydration, `None`); the CLI parser test in `crates/warp_cli/src/lib_tests.rs:233-252`; viewer-side tests in `app/src/terminal/shared_session/viewer/event_loop_tests.rs`; sandbox-side unit tests on `TerminalModel::send_ambient_setup_phase_ended_for_shared_session`; and direct `IdleTimeoutSender::complete_with_optional_idle` tests in `app/src/ai/agent_sdk/driver_tests.rs`.
## Validation
- `cargo fmt --all --check`.
- `cargo check -p warp --tests`.
Nextest and full clippy are intentionally not part of the per-PR validation for this work — the changes touch isolated client-side wiring and the targeted `cargo check` plus per-stage unit tests are sufficient.
## Wire shape coordination summary
- `SpawnAgentRequest` (`POST /agent/run`): `prompt: Option<String>` (Stage 1); does not carry `skip_initial_turn` (Stage 2).
- `TaskAssignmentMessage` (server → self-hosted worker): top-level `SkipInitialTurn bool` (JSON tag `skip_initial_turn`, `omitempty`).
- `--skip-initial-turn` CLI flag (worker → CLI): the sole worker→driver contract for the skip-initial-turn decision.
- `OrderedTerminalEventType::AmbientSetupPhaseEnded` (sharer → viewer via session-sharing-protocol).
