# Empty-Prompt Local-to-Cloud Handoff (REMOTE-1499) — Tech Spec
This is the canonical tech spec for the warp-4 side of the empty-prompt local-to-cloud handoff feature. It describes the full end-to-end architecture across every stage of implementation. Per-stage sub-tech-specs (`STAGE-1.md`, `STAGE-2.md`) describe what each individual stage delivers; this document is the single source of truth for the whole warp-4 architecture and is intentionally not modified by subsequent stages.
Cross-repo siblings:
- `../../../warp-server-4/specs/REMOTE-1499/` — server-side relaxations + worker-derived skip-initial-turn.
- `../../../oz-agent-worker/specs/REMOTE-1499/` — self-hosted worker side of skip-initial-turn.
- `../../../session-sharing-protocol/specs/REMOTE-1499/` — `AmbientSetupPhaseEnded` variant on `OrderedTerminalEventType`.
- `../../../session-sharing-server/specs/REMOTE-1499/` — testing-only protocol dep swap.
Stack layout in this repo:
- Stage 1 base: `harry/empty-prompt-handoff-wire-contract`. Wire-contract widening only — no runtime behavior change in warp-4 except the CLI skill-only `None` emission. Documented in `STAGE-1.md`.
- Stage 2 head: `harry/empty-prompt-handoff-local`, stacked on Stage 1. Combines sub-stages 2a (entry-point + substitution + indicator), 2b (worker-derived skip-initial-turn client-side), 2c (`AmbientSetupPhaseEnded` marker + viewer teardown + `IdleTimeoutSender` helper), and 2d (non-skip ServerSide marker emission + legacy fallback comments). Documented in `STAGE-2.md`.
## Architecture overview
The feature is composed of four orthogonal sub-systems on the warp-4 side, all gated behind `FeatureFlag::EmptyPromptHandoff` once Stage 2a lands:
1. **Wire-contract widening** — `SpawnAgentRequest.prompt` becomes `Option<String>` so the client can omit the field on the wire. Stage 1.
2. **Entry-point unification + substitution + indicator** — three entry points (chip / `&` / `/handoff`) funnel through `start_local_to_cloud_handoff`; `build_handoff_spawn_request` substitutes `continue in the cloud` against in-progress sources; an `empty_prompt_handoff_indicator` enum drives the queued-prompt-indicator label. Stage 2a.
3. **Worker-derived skip-initial-turn** — the AgentDriver reads the `--skip-initial-turn` CLI flag (set by the worker on a per-execution basis) rather than destructuring a stored client-side bool. Stage 2b client-side.
4. **`AmbientSetupPhaseEnded` setup-phase teardown** — the AgentDriver emits a new shared-session-protocol marker after env setup completes; the viewer event loop tears down the Cloud Mode Setup V2 UI on receipt. Stages 2c and 2d.
## 1. Wire-contract widening (Stage 1)
`SpawnAgentRequest` in `app/src/server/server_api/ai.rs:205-207`:
```rust path=/Users/harryalbert/warp-4/app/src/server/server_api/ai.rs start=205
#[serde(skip_serializing_if = "Option::is_none")]
pub prompt: Option<String>,
```
`Option<T>` serializes transparently under serde, so `Some("hello")` continues to emit `"prompt": "hello"` byte-for-byte identical to the pre-change wire shape. `skip_serializing_if = "Option::is_none"` ensures `None` omits the field entirely rather than emitting `"prompt": null`. The struct derives only `Serialize`, not `Deserialize`, so wire compatibility only needs to hold client→server.
All twelve `SpawnAgentRequest { … }` literal construction sites are updated to wrap their existing prompt in `Some(...)`. The two interactive submit paths in `app/src/terminal/view/ambient_agent/model.rs:632` and `:1120` always wrap the result of `extract_user_query_mode(prompt)`. `app/src/pane_group/pane/terminal_pane.rs:2137` wraps the orchestration child's `request.prompt`. Fixtures in `spawn_tests.rs:702/770/838/901/1047`, `model_tests.rs:54`, `view_tests.rs:1323`, `mcp_config_tests.rs:272`, and `ai_tests.rs:39` are updated mechanically.
The CLI path at `app/src/ai/agent_sdk/ambient.rs:267-313` is the only non-test site that emits `prompt: None` at runtime: skill-only and saved-prompt-only invocations propagate `Option<String>` through the resolution branch (`Some(Prompt::PlainText(text)) → Some(text)`, `Some(Prompt::SavedPrompt(id)) →` resolves on hit / errors on miss, `None → None`). The extracted `(prompt, mode)` pair is computed by a `match` at `ambient.rs:474-480` that runs `extract_user_query_mode` only on the `Some` branch and defaults `mode` to `UserQueryMode::Normal` when the prompt is `None`. `UserQueryMode` is imported at the top of the file (`ambient.rs:6`).
Two reader sites use `.as_deref()` to preserve previous behavior in the `Some` case and short-circuit cleanly in the `None` case:
- `app/src/terminal/view/ambient_agent/block/entry.rs:160` — entry-block title fallback chain: `request.prompt.as_deref().and_then(Self::meaningful_title)`.
- `app/src/terminal/view/ambient_agent/view_impl.rs:159-164` — Cloud Mode Setup V2 queued-prompt insertion: `request.prompt.as_deref().map(|prompt| display_user_query_with_mode(request.mode, prompt))`. The existing `if !prompt.is_empty()` guard at `view_impl.rs:166` suppresses the block on `None`.
No other code in warp-4 pattern-matches or destructures `SpawnAgentRequest.prompt`.
## 2. Entry-point unification + substitution + indicator (Stage 2a)
### 2.1 Feature flag
`FeatureFlag::EmptyPromptHandoff` is added to `crates/warp_features/src/lib.rs`. Default off. Not added to `DOGFOOD_FLAGS` initially.
### 2.2 Three entry points
All three converge in `start_local_to_cloud_handoff` (`app/src/workspace/view.rs:13652-13663`), which synthesizes an empty `PendingCloudLaunch { prompt: "".to_owned(), attachments: vec![] }` when the feature flag is on.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2547-2574` `OpenHandoffPane` action: when `EmptyPromptHandoff` is on, dispatches `WorkspaceAction::OpenLocalToCloudHandoffPane { launch: None, environment_id: None, entry_point: HandoffEntryPoint::FooterChip }` directly. The chip skips `&` compose mode.
- `app/src/terminal/input.rs:4060-4067`: removes the empty-prompt early-return in `maybe_launch_cloud_handoff_request`; always builds `PendingCloudLaunch { prompt: "".to_owned(), attachments }` when `EmptyPromptHandoff` is on.
- `app/src/terminal/input/slash_commands/mod.rs:924-940` `/handoff` with no argument: dispatches the same `OpenLocalToCloudHandoffPane { launch: None, ... }` as the chip when `EmptyPromptHandoff` is on. The central launch synthesis in `start_local_to_cloud_handoff` covers FooterChip / Ampersand / SlashCommand uniformly. No separate handoff compose pane is created for the SlashCommand entry point.
### 2.3 Wire-level substitution
`app/src/terminal/view/ambient_agent/model.rs:686-743` `build_handoff_spawn_request`:
- When the submitted prompt is empty AND `pending_handoff.source_conversation_in_progress` is true, substitute `prompt: Some("continue in the cloud".to_owned())` on the wire.
- Otherwise (idle source, regardless of snapshot) send `prompt: None`.
`source_conversation_in_progress` is captured once at handoff initiation by reading `BlocklistAIHistoryModel::active_conversation(...).status()` and stored on the `PendingHandoff` struct so it can't drift mid-flow.
### 2.4 `empty_prompt_handoff_indicator` enum
`app/src/terminal/view/ambient_agent/model.rs:565-601` returns a context-aware variant decoupled from the substituted wire prompt:
```rust path=null start=null
pub enum EmptyPromptHandoffIndicator {
    Continue,                 // "Continuing previous task in the cloud"
    SnapshotRehydrationOnly,  // "Applying workspace changes…"
    None,                     // standard setup indicator covers it
}
```
The variant is selected from `(source_conversation_in_progress, has_snapshot)`:
- In-progress → `Continue`.
- Idle + non-empty snapshot → `SnapshotRehydrationOnly`.
- Idle + empty snapshot → `None`.
The label string is intentionally NOT derived from the wire substitution. The two strings (`continue in the cloud` on the wire, `Continuing previous task in the cloud` in the indicator) are independently tunable.
### 2.5 view_impl rendering
`app/src/terminal/view/ambient_agent/view_impl.rs:154-189` consumes the indicator and inserts a labeled queued-prompt block instead of the literal wire prompt. The existing block scaffolding (the "queued-prompt indicator" block) is reused so layout/animation/styling are unchanged; only the label-source switches from `display_user_query_with_mode(...)` to the indicator-derived label when an empty-prompt path is detected.
### 2.6 Telemetry extensions
`app/src/ai/ambient_agents/telemetry.rs`:
- `CloudAgentTelemetryEvent::HandoffInitiated` gains `empty_prompt: bool` and `injection_path: HandoffInjectionPath { None | Continue | SnapshotRehydrationOnly }`.
- New `CloudAgentTelemetryEvent::HandoffSnapshotPrepared { had_snapshot: bool }` fires after `derive_touched_workspace` settles.
## 3. Worker-derived skip-initial-turn (Stage 2b, client-side)
The original Stage 2a design plumbed a `skip_initial_turn: Option<bool>` client-side: derived at submit time from `(wire_prompt.is_none() && initial_snapshot_token.is_none())` in `build_handoff_spawn_request`, sent on the wire under `SpawnAgentRequest.skip_initial_turn`, stored server-side on the task config snapshot, and round-tripped back to the sandboxed CLI as the `--skip-initial-turn` flag. The stored-flag shape created a cloud→cloud follow-up bug: the server stamped `skip_initial_turn` once on the original task config snapshot, and a subsequent execution against the same task (e.g. a user follow-up with a real prompt) would replay the stamped flag even though the second execution's input was non-empty.
Stage 2b cuts over to derive the bool fresh per execution on the server (`common.ShouldSkipInitialTurn(task, execution)` in warp-server-4). The CLI flag `--skip-initial-turn` is the only worker→driver contract.
### 3.1 SpawnAgentRequest field removal
`app/src/server/server_api/ai.rs:249-260` — remove the `skip_initial_turn: Option<bool>` field from `SpawnAgentRequest`.
### 3.2 Client-side derivation removal
`app/src/terminal/view/ambient_agent/model.rs:742-747,765` — remove the local `let skip_initial_turn = (wire_prompt.is_none() && initial_snapshot_token.is_none()).then_some(true);` block and the `skip_initial_turn` field assignment in the constructed `SpawnAgentRequest`. The `spawn_agent` path at `model.rs:1264` drops its `skip_initial_turn: None` field as well.
### 3.3 `AgentRunPrompt::ServerSide` field removal
`app/src/ai/agent_sdk/driver.rs:405-410` — remove the `skip_initial_turn: bool` field from the `ServerSide` variant. The variant now only carries `skill: Option<ParsedSkill>` and `attachments_dir: Option<String>`. Two destructure sites that previously used `skip_initial_turn: _,` (in `prepare_harness` and the non-skip branch of `execute_run`) drop the explicit ignore.
### 3.4 `build_server_side_task` cleanup
`app/src/ai/agent_sdk/mod.rs:490` — remove the `skip_initial_turn: args.skip_initial_turn` field assignment when constructing `AgentRunPrompt::ServerSide`. The stale comment at `mod.rs:1216` (`task.prompt.skip_initial_turn is preserved across this update.`) is removed.
### 3.5 `AgentDriverOptions` + `AgentDriver` field
`app/src/ai/agent_sdk/driver.rs` — add `skip_initial_turn: bool` to both `AgentDriverOptions` (destructured from `RunAgentArgs::skip_initial_turn` by the `build_driver_options_and_task` closure in `mod.rs:855`) and the `AgentDriver` struct itself (populated from the option in `AgentDriver::new`). The `new_for_test` constructor at `driver.rs:711` initializes the field to `false`.
### 3.6 Skip-branch rewire in `execute_run`
`app/src/ai/agent_sdk/driver.rs:2410-2411` — replace
```rust path=null start=null
let is_skip_initial_turn = matches!(
    &task_prompt,
    AgentRunPrompt::ServerSide {
        skip_initial_turn: true,
        ..
    },
);
```
with
```rust path=null start=null
let is_skip_initial_turn =
    self.skip_initial_turn && matches!(&task_prompt, AgentRunPrompt::ServerSide { .. });
```
The skip branch body (enter agent view, emit `AmbientSetupPhaseEnded`, schedule deferred `Success` via `IdleTimeoutSender::complete_with_optional_idle`) is unchanged. The non-skip `ServerSide` arm drops its `skip_initial_turn: _,` from the destructure pattern.
### 3.7 CLI flag (kept)
`crates/warp_cli/src/agent.rs:368-372` — unchanged. The `--skip-initial-turn` flag is still the worker→CLI contract. The CLI parser test at `crates/warp_cli/src/lib_tests.rs:233-252` (`agent_run_accepts_skip_initial_turn_with_task_id`) is kept.
### 3.8 Test removals
`app/src/terminal/view/ambient_agent/model_tests.rs:614-725` — the three tests `build_handoff_spawn_request_sets_skip_initial_turn_when_no_content`, `build_handoff_spawn_request_does_not_set_skip_initial_turn_with_continue_substitution`, and `build_handoff_spawn_request_does_not_set_skip_initial_turn_with_snapshot` are deleted (they asserted on a now-removed client-side derivation). The `retry_request` fixture at `model_tests.rs:91-117` drops its `skip_initial_turn: None` field. Equivalent coverage for the server-side derivation lives in warp-server-4 and oz-agent-worker tests.
## 4. `AmbientSetupPhaseEnded` setup-phase teardown (Stages 2c + 2d)
**Bug being fixed:** with the Stage 2b skip path active, the cloud pane comes up under Cloud Mode Setup V2, the environment setup commands run, but the pane stays stuck in "setting up…" because no first `AppendedExchange` event fires to trigger the teardown.
**Two-cause root cause:** (a) the session-sharing-server was typed-decoding the `OrderedTerminalEventType` enum, so emitting a new variant from an older protocol rev silently dropped the marker. Fix: testing-only local-path swap on the server. (b) The AgentDriver skip path ignored `idle_on_complete`, sending `Success` directly to the oneshot. Fix: new `IdleTimeoutSender::complete_with_optional_idle` helper plus a restructured `execute_run`.
### 4.1 Testing-only Cargo.toml swap
`Cargo.toml:248` — swap the `session-sharing-protocol` dep to `path = "../session-sharing-protocol"`. Must be reverted to `git = ..., rev = <merged SHA>` after the protocol PR merges.
### 4.2 `TerminalModel` helper
`app/src/terminal/model/terminal_model.rs` — add `send_ambient_setup_phase_ended_for_shared_session`, modeled on the adjacent `send_agent_conversation_replay_started_for_shared_session`. The helper is a no-op for non-sharer terminals; sharer terminals emit a typed `OrderedTerminalEventType::AmbientSetupPhaseEnded` event through the existing shared-session event channel.
### 4.3 `AgentDriver::execute_run` restructure
`app/src/ai/agent_sdk/driver.rs` — `execute_run` is restructured so the skip branch builds `IdleTimeoutSender` first, then runs the skip block (`enter_agent_view` + emit marker via `send_ambient_setup_phase_ended_for_shared_session` + `complete_with_optional_idle`) before the history subscription, then sets up the subscription, then conditionally dispatches the non-skip prompt. Scheduling the timer before the subscription means a later `AppendedExchange` from a session-sharing-protocol follow-up correctly invalidates the timer via `IdleTimeoutSender`'s internal generation counter.
### 4.4 `IdleTimeoutSender::complete_with_optional_idle`
New helper: `IdleTimeoutSender::complete_with_optional_idle(idle_on_complete, value)`. Defers via `end_run_after` when `Some(d)`; falls back to `end_run_now` when `None`. Existing `UpdatedConversationStatus` and harness-exit branches in `execute_run` are refactored to use the same helper for consistency.
### 4.5 Viewer event_loop.rs arm
`app/src/terminal/shared_session/viewer/event_loop.rs` — adds an `AmbientSetupPhaseEnded` arm. Flips `BlockList::set_is_executing_oz_environment_startup_commands(false)`, then calls `AmbientAgentViewModel::tear_down_active_setup_command_group` which runs `finish_setup_command_group` + `set_setup_command_group_visibility(false)`. "No active group" is treated as a no-op for idempotency. The arm is path-agnostic — it handles both the skip-initial-turn path AND the normal cloud agent path.
### 4.6 Stage 2d: non-skip ServerSide marker emission
`app/src/ai/agent_sdk/driver.rs:2760-2792` `AgentDriver::execute_run` non-skip `AgentRunPrompt::ServerSide` arm: after the existing `terminal.enter_agent_view(None, restored_conversation_id, AgentViewEntryOrigin::Cli, ctx)` call and before the `terminal.ai_controller().update(...)` block that fires `AIAgentInput::StartFromAmbientRunPrompt`, invoke `terminal.model.lock().send_ambient_setup_phase_ended_for_shared_session()`. Mirrors the skip-path emission at `driver.rs:2418-2425`. Do NOT touch the `AgentRunPrompt::Local` arm — local runs don't have a setup phase.
### 4.7 Stage 2d: legacy fallback comments
`app/src/terminal/view.rs:5496-5507` — add a comment to the `BlocklistAIHistoryEvent::AppendedExchange`-driven `set_is_executing_oz_environment_startup_commands(false)` block noting that this is a legacy fallback teardown; the canonical signal is `AmbientSetupPhaseEnded` handled in `event_loop.rs`. Both paths are idempotent.
`app/src/terminal/view/ambient_agent/block/setup_command_text.rs:119-136` — add the same fallback comment on the `BlocklistAIHistoryEvent::AppendedExchange` subscription. Removal tracked in PRODUCT.md "Deferred follow-ups".
## Testing strategy
Per-stage details live in `STAGE-1.md` and `STAGE-2.md`. High-level:
- Stage 1: serialization round-trip tests on `SpawnAgentRequest { prompt: None }` and updated borrow-site fixtures.
- Stage 2a: behavioral tests on empty-prompt auto-submit, indicator variants, and feature-flag gating in `app/src/terminal/view/ambient_agent/model_tests.rs`.
- Stage 2b: CLI parser test in `crates/warp_cli/src/lib_tests.rs:233-252` is kept; client-side derivation tests are deleted.
- Stage 2c: viewer-side tests in `app/src/terminal/shared_session/viewer/event_loop_tests.rs` (teardown + idempotency), sandbox-side unit tests on `TerminalModel::send_ambient_setup_phase_ended_for_shared_session` (sharer-emits + non-sharer-no-op), `IdleTimeoutSender::complete_with_optional_idle` direct tests in `app/src/ai/agent_sdk/driver_tests.rs`.
- Stage 2d: no new tests; the comment-only changes are covered by existing Stage 2c viewer-side tests.
## Validation
- `cargo fmt --all --check`.
- `cargo check -p warp --tests`.
Per orchestrator standing instruction, nextest and full clippy are skipped for this refactor.
## Wire shape coordination summary
- `SpawnAgentRequest` (`POST /agent/run`): `prompt: Option<String>` (Stage 1). `skip_initial_turn` field removed (Stage 2b).
- `TaskAssignmentMessage` (server → self-hosted worker): top-level `SkipInitialTurn bool` (JSON tag `skip_initial_turn`, `omitempty`). Stage 2b.
- `--skip-initial-turn` CLI flag (worker → CLI): unchanged. Stage 2b keeps the flag as the only worker→driver contract.
- `OrderedTerminalEventType::AmbientSetupPhaseEnded` (sharer → viewer via session-sharing-protocol): new variant. Stage 2c.
