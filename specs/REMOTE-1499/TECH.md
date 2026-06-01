# Empty-Prompt Local-to-Cloud Handoff (REMOTE-1499) — Tech Spec
Canonical tech spec for the warp side of REMOTE-1499. This document describes the end-to-end architecture; code-level minutia (specific line references, exhaustive call-site lists, considered alternatives, test enumerations) lives in the per-stage sub-tech-specs `STAGE-1.md` and `STAGE-2.md`.
Cross-repo siblings:
- `warp-server/specs/REMOTE-1499/` — server-side validator relaxations + worker-derived `ShouldSkipInitialTurn`.
- `oz-agent-worker/specs/REMOTE-1499/` — self-hosted worker side of `--skip-initial-turn`.
- `session-sharing-protocol/specs/REMOTE-1499/` — `CloudModeSetupPhaseEnded` variant on `OrderedTerminalEventType`.
- `session-sharing-server/specs/REMOTE-1499/` — testing-only protocol dep swap.
Stack layout:
- `harry/empty-prompt-handoff-wire-contract` (Stage 1) — wire-contract widening. See `STAGE-1.md`.
- `harry/empty-prompt-handoff-local` (Stage 2, stacked on Stage 1) — client behavior and shared-session protocol wiring. See `STAGE-2.md`.
## Architecture
Four orthogonal sub-systems on the warp side. The wire-contract widening lands in Stage 1; the other three land together in Stage 2 and ship unconditionally (no feature flag).
### Wire-contract widening
`SpawnAgentRequest.prompt` in `app/src/server/server_api/ai.rs` becomes `Option<String>` with `skip_serializing_if = "Option::is_none"` so the client can omit the field for empty submissions. The struct derives only `Serialize`, so wire compatibility only has to hold client→server. The two reader sites (entry-block title fallback in `app/src/terminal/view/ambient_agent/block/entry.rs`, queued-prompt insertion in `app/src/terminal/view/ambient_agent/view_impl.rs`) go through `.as_deref()` and short-circuit cleanly on `None`. The only runtime sites that emit `prompt: None` are the `oz agent run` CLI skill-only and conversation-only paths.
### Entry-point unification + client-side substitution
Three entry points — the "Hand off to cloud" footer chip, `&` + Enter on an empty buffer, and `/handoff` with no argument — converge in `start_local_to_cloud_handoff` (`app/src/workspace/view.rs`) when the source-content guardrail passes. The guardrail (`source_conversation_has_content` in `app/src/ai/blocklist/handoff/mod.rs`) requires the active source conversation to exist and have at least one exchange. Per-entry-point fallbacks when the guardrail fails are specified in PRODUCT.md and detailed in STAGE-2.md.
At submit time, `build_handoff_spawn_request` (`app/src/terminal/view/ambient_agent/model.rs`) chooses the wire prompt against the captured `source_conversation_active` bool and the post-upload `InitialSnapshotToken`:
- empty + in-progress source + snapshot → `"Continue. Apply the workspace changes from my previous session."`
- empty + in-progress source, no snapshot → `"Continue"`
- empty + idle source + snapshot → `"Apply the workspace changes from my previous session."`
- empty + idle source, no snapshot → `prompt: None`
- non-empty → unchanged
Display tracks the wire one-to-one — the queued-prompt block renders `request.prompt.as_deref()` via `display_user_query_with_mode`, and the existing `if !prompt.is_empty()` guard suppresses the block on `None`. There is no `EmptyPromptHandoffIndicator` enum.
Telemetry (`app/src/ai/ambient_agents/telemetry.rs`): `HandoffInitiated` gains `empty_prompt: bool` and `injection_path: HandoffInjectionPath { None | Continue | SnapshotRehydration }` (computed from the same `source_conversation_active` bool that drives the wire substitution); a new `HandoffSnapshotPrepared { derived_workspace_had_content: bool }` event fires after `derive_touched_workspace` settles.
### Worker-derived skip-initial-turn
The "should the cloud agent skip its initial LLM turn?" decision is computed fresh per execution by warp-server's `common.ShouldSkipInitialTurn(task, execution)` and reaches the AgentDriver as the `--skip-initial-turn` CLI flag — the only worker→driver contract for this signal. `ShouldSkipInitialTurn` is routed through the shared `ShouldSkipFirstTurn` predicate that also gates the runtime's first-turn validator, so the two cannot drift.
Client-side the wire and harness machinery are deliberately silent: `SpawnAgentRequest` does not carry the flag, `build_handoff_spawn_request` does not derive it, and `AgentRunPrompt::ServerSide` does not embed it (the variant must round-trip through harness-agnostic `prepare_harness`). `AgentDriverOptions`/`AgentDriver` carry a `skip_initial_turn: bool` populated from clap-parsed `RunAgentArgs::skip_initial_turn`; the gate in `AgentDriver::execute_run` matches it against `AgentRunPrompt::ServerSide { .. }` and emits a loud `[DEBUG]` warning on misconfigured `Local` pairings. The CLI flag's `requires_all = ["task_id", "idle_on_complete"]` pins the worker→driver invariant at the parser layer.
Rejected: stamping the decision onto the task config at dispatch time (drifts across executions); deriving client-side (client sees only the first execution).
### `CloudModeSetupPhaseEnded` setup-phase teardown
Every cloud agent run signals "environment setup phase complete" via a new `OrderedTerminalEventType::CloudModeSetupPhaseEnded` shared-session-protocol marker. The sharer emits it once setup commands have finished; the viewer's event loop (`app/src/terminal/shared_session/viewer/event_loop.rs`) tears down the Cloud Mode Setup V2 UI on receipt — flipping `BlockList::is_executing_oz_environment_startup_commands` off (which gates `is_cloud_agent_pre_first_exchange`, the input-vs-loading-footer toggle) and finishing/hiding the active setup command group. The marker fires on both the skip-initial-turn path (no first LLM turn) and the normal `ServerSide` path. A dedicated marker is necessary because the skip-initial-turn path never fires `AppendedExchange`, the pre-feature implicit signal — without it the input box would stay hidden behind the loading footer forever.
`AgentDriver::execute_run` emits the marker via `TerminalModel::send_cloud_mode_setup_phase_ended_for_shared_session` (a no-op on non-sharer terminals). The skip branch builds `IdleTimeoutSender` first, runs the skip block (enter agent view + emit marker + `complete_with_optional_idle`), then sets up the history subscription; scheduling the timer before the subscription lets a later `AppendedExchange` from a session-sharing-protocol follow-up correctly invalidate the timer. `IdleTimeoutSender::complete_with_optional_idle` is shared across all completion paths so they uniformly honor the optional idle window. Legacy `AppendedExchange`-driven teardowns in `app/src/terminal/view.rs` and `app/src/terminal/view/ambient_agent/block/setup_command_text.rs` remain in place as an idempotent fallback for viewers connecting to pre-feature sharers; removal is a deferred follow-up.
Rejected: reusing `AppendedExchange` (never fires on the skip-initial-turn path, so the input would stay hidden); letting the driver send `Success` directly without `IdleTimeoutSender` (driver tears down ~80ms after `Success`, too fast for a session-sharing-protocol follow-up).
## Validation
- `cargo fmt --all --check`.
- `cargo check -p warp --tests`.
Nextest and full clippy are intentionally not part of the per-PR validation. Per-stage unit tests are detailed in `STAGE-1.md` (serialization round-trip on `SpawnAgentRequest { prompt: None }` plus borrow-site fixtures) and `STAGE-2.md` (substitution outcomes in `build_handoff_spawn_request`, CLI parser tests, viewer event-loop arm, `IdleTimeoutSender::complete_with_optional_idle`).
## Wire shape coordination summary
- `SpawnAgentRequest` (`POST /agent/run`): `prompt: Option<String>` (Stage 1); does not carry `skip_initial_turn` (Stage 2).
- `TaskAssignmentMessage` (server → self-hosted worker): top-level `SkipInitialTurn bool` (JSON tag `skip_initial_turn`, `omitempty`).
- `--skip-initial-turn` CLI flag (worker → CLI): the sole worker→driver contract for the skip-initial-turn decision.
- `OrderedTerminalEventType::CloudModeSetupPhaseEnded` (sharer → viewer via session-sharing-protocol).
