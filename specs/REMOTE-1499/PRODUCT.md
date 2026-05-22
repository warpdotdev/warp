# Empty-Prompt Local-to-Cloud Handoff (REMOTE-1499) — Product Spec
This is the canonical product spec for the empty-prompt local-to-cloud handoff feature. It describes the full end-to-end product behavior across every stage of implementation. Per-stage sub-tech-specs (`STAGE-1.md`, `STAGE-2.md`) describe what each individual stage delivers; this document is the single source of truth for the feature shape and is intentionally not modified by subsequent stages.
Sibling specs (cross-repo):
- `../../../warp-server-4/specs/REMOTE-1499/PRODUCT.md` (server-side counterpart, mirrors this doc).
- `../../../warp-server-4/specs/REMOTE-1499/TECH.md`, `../../../oz-agent-worker/specs/REMOTE-1499/TECH.md`, `../../../session-sharing-protocol/specs/REMOTE-1499/TECH.md`, `../../../session-sharing-server/specs/REMOTE-1499/TECH.md`: cross-repo tech specs.
## Problem
Today both the `&` handoff and the "Hand off to cloud" chip require a non-empty prompt to start a cloud run. The chip only enters compose mode, and `&` + Enter on an empty buffer is a no-op. `/handoff` with no argument behaves the same way as `&` + Enter on empty: nothing. Users who want to continue an in-progress local agent run in the cloud, or rehydrate workspace changes into a fresh cloud agent without typing a fake prompt, have no friction-free path today.
The chip's prior contract (REMOTE-1558 §36-38, `specs/REMOTE-1558/PRODUCT.md`) intentionally banned auto-start. Empty-Prompt Handoff inverts that contract.
## Goals
1. Three functionally equivalent entry points all result in an **immediate** local→cloud handoff with no compose step:
   - Click the "Hand off to cloud" chip on the agent input footer.
   - Type `&` + Enter on an empty buffer.
   - Type `/handoff` with no argument (also `&` + Enter on a non-empty buffer with `&` alone in the buffer).
   `/handoff` with no argument does NOT open a separate handoff compose pane — it dispatches the same launch synthesis as the chip and `&`+Enter. All three entry points funnel through the same `start_local_to_cloud_handoff` path in `app/src/workspace/view.rs:13652-13663` and produce the same wire payload.
2. Empty-prompt handoff against an in-progress local agent source: the client substitutes `continue in the cloud` on the wire (local→cloud only) so the cloud agent picks up the conversation context coherently. The substitution is silent — the user never sees this string.
3. Empty-prompt handoff with snapshotted workspace changes: the existing snapshot-rehydration `<system-message>` prompt remains the only user-role message visible to the LLM. The user-visible label on the queued-prompt indicator surfaces what's happening contextually.
4. The cloud pane's "setting up…" Cloud Mode Setup V2 UI transitions out properly after environment setup, even when the cloud agent does not fire a first LLM exchange. The viewer learns the setup phase has ended via a new shared-session-protocol marker rather than depending on a first `AppendedExchange` event.
5. Gated behind `FeatureFlag::EmptyPromptHandoff` (default off, introduced in Stage 2a).
## Non-goals
- Cloud→cloud empty `Continue` submission (deferred follow-up; out of scope here — see "Deferred follow-ups" below).
- Removing the legacy `AppendedExchange`-driven setup-phase teardown fallback paths (deferred follow-up).
## User-facing behavior
### Entry points
On a Warp client with `FeatureFlag::EmptyPromptHandoff` enabled (and `OzHandoff && HandoffLocalCloud` enabled), three entry points all launch an empty-prompt local→cloud handoff:
- **Chip click.** Clicking the "Hand off to cloud" chip in the agent input footer (`app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs:2547-2574`) dispatches `WorkspaceAction::OpenLocalToCloudHandoffPane { launch: None, environment_id: None, entry_point: HandoffEntryPoint::FooterChip }` directly. The chip skips `&` compose mode entirely — clicking once is a complete commit.
- **`&` + Enter on an empty buffer.** Removing the empty-prompt early-return at `app/src/terminal/input.rs:4060-4067` lets `maybe_launch_cloud_handoff_request` build a `PendingCloudLaunch { prompt: "".to_owned(), attachments }` and proceed. Entry point recorded as `HandoffEntryPoint::Ampersand`.
- **`/handoff` with no argument.** `app/src/terminal/input/slash_commands/mod.rs:924-940` dispatches the same `OpenLocalToCloudHandoffPane { launch: None, ... }` as the chip when the user types `/handoff` with no following text. Entry point recorded as `HandoffEntryPoint::SlashCommand`. There is **no separate handoff compose pane** for this path — the dispatch is immediate.
All three converge in `start_local_to_cloud_handoff` (`app/src/workspace/view.rs:13652-13663`), which synthesizes an empty `PendingCloudLaunch` and proceeds through the existing handoff machinery. With the feature flag off, the three entry points retain their pre-feature behavior (no-op / open compose).
### Wire-level substitution
At submit time, `build_handoff_spawn_request` in `app/src/terminal/view/ambient_agent/model.rs:686-743` decides the wire-level prompt:
- If the user-submitted prompt is empty AND the source conversation is **in progress** (`pending_handoff.source_conversation_in_progress == true`, captured once at handoff initiation from `BlocklistAIHistoryModel::active_conversation(...).status()`): substitute `prompt: Some("continue in the cloud".to_owned())` on the wire. The cloud agent's first LLM turn receives this as the user query.
- If the user-submitted prompt is empty AND the source conversation is idle (no active streaming exchange): send `prompt: None` on the wire. The cloud agent either ingests rehydrated workspace state from the snapshot or runs against an empty initial turn.
- If the user-submitted prompt is non-empty: the original prompt flows through unchanged (pre-existing behavior).
The substitution is decoupled from the user-visible queued-prompt indicator label — see below.
### Queued-prompt indicator
The "queued-prompt indicator" is the small block that appears in the cloud agent's pane during the Cloud Mode Setup V2 warmup phase. Pre-feature, that block showed the literal user prompt as a preview. For empty-prompt handoff there is no literal user prompt to show; instead, `empty_prompt_handoff_indicator` in `app/src/terminal/view/ambient_agent/model.rs:565-601` selects a context-aware label for that same block:
- **In-progress source** (continuing a streaming local conversation): label `"Continuing previous task in the cloud"`.
- **Idle source + non-empty snapshot** (`derive_touched_workspace` produced a non-empty rehydration token): label `"Applying workspace changes…"`.
- **Idle source + empty snapshot** (truly empty case): no empty-prompt indicator is rendered — the standard Cloud Mode Setup V2 setup indicator covers the warmup phase on its own.
The label string is **DECOUPLED** from the wire-level substitution string. They are independently tunable: design may refine either copy without coupling the change to the other side. The indicator labels are surface-only; the LLM never sees them.
### Cloud Mode Setup V2 teardown
Pre-feature, the cloud pane transitions out of the "setting up…" UI via the first `BlocklistAIHistoryEvent::AppendedExchange` event — i.e. the first LLM turn firing. For empty-prompt handoff with no first turn (the skip-initial-turn path), no `AppendedExchange` fires. To prevent the pane from being stuck in "setting up…" forever, the worker emits a new `AmbientSetupPhaseEnded` shared-session-protocol marker once the environment setup phase is complete. The viewer receives the marker and runs the same teardown the `AppendedExchange` path used to drive (flip the executing-startup-commands flag off, finish the setup command group, hide the group). The marker is also emitted on the normal (non-skip) cloud agent path so every cloud agent run signals "setup phase complete" via the same canonical signal.
The legacy `AppendedExchange`-driven teardowns at `app/src/terminal/view.rs:5496-5507` and `app/src/terminal/view/ambient_agent/block/setup_command_text.rs:119-136` are retained as a transition-compat fallback for old sharers. New viewers safely run both paths because both teardowns are idempotent. Removal of the fallbacks is tracked in "Deferred follow-ups".
## Telemetry
- `CloudAgentTelemetryEvent::HandoffInitiated` is extended with two new fields:
  - `empty_prompt: bool` — true when the user's submitted prompt was empty.
  - `injection_path: HandoffInjectionPath { None | Continue | SnapshotRehydrationOnly }` — which substitution path was taken (mirrors the indicator variant logic).
- New `CloudAgentTelemetryEvent::HandoffSnapshotPrepared { had_snapshot: bool }` fires after `derive_touched_workspace` settles. Analytics can join this against `HandoffInitiated.injection_path` to learn whether `SnapshotRehydrationOnly` paths actually carried snapshot content.
Both events live under `app/src/ai/ambient_agents/telemetry.rs`. Their schemas are documented in the wire spec but no PII is added.
## Feature flag
`FeatureFlag::EmptyPromptHandoff` in `crates/warp_features/src/lib.rs`, default off. Introduced in Stage 2a alongside the client-side behavior changes. Server-side relaxations (Stage 1) are additive and unconditional — they widen what's accepted on the wire but cannot be reached until the client flag turns on, because every existing caller continues to send non-empty prompts pre-Stage-2.
## Cross-repo dependencies
- Server-side relaxations (warp-server-4) widen the validators on `POST /agent/runs`, the multi-agent runtime first-turn interceptor, and `ProcessFollowupForTask`. They land in Stage 1 on the shared branch `harry/empty-prompt-handoff-wire-contract`. See `../../../warp-server-4/specs/REMOTE-1499/`.
- Worker-derived `skip_initial_turn` (warp-server-4 + oz-agent-worker) computes the "should the cloud agent skip its initial LLM turn?" boolean fresh per dispatch from `execution.Input.Prompt + execution.Input.InitialSnapshotToken`. Wire shape: a top-level `SkipInitialTurn bool` on `TaskAssignmentMessage`. The CLI flag `--skip-initial-turn` is the worker→CLI contract.
- `AmbientSetupPhaseEnded` shared-session-protocol marker (session-sharing-protocol + session-sharing-server). Stage 2c.
## Open design questions
- Final copy for the queued-prompt indicator labels (`"Continuing previous task in the cloud"`, `"Applying workspace changes…"`). Defer to design.
- Final copy for the `&` ghost-text wording. Defer to design.
- Final wire value for the in-progress continue substitution (`"continue in the cloud"`). Design may refine.
- Final wire value for the snapshot-rehydration substituted prompt server-side (`"Apply the workspace changes from my previous session."`). Design may refine.
- Title fallback when the source conversation title is itself empty — fall back to plain `Cloud agent run`.
## Deferred follow-ups
Not in scope for this feature but tracked here so we don't lose them:
- **Cloud→cloud empty `Continue` submission.** Gated on `HandoffCloudCloud && EmptyPromptHandoff`. Would permit empty submission via `try_submit_pending_cloud_followup` (`app/src/workspace/view.rs:20401`) and plumb `Option<String>` through the `submit_cloud_followup` callsite. Open questions: (a) whether to mirror Stage 2's `continue in the cloud` substitution for empty follow-ups against an in-progress source, (b) whether the cloud→cloud path needs its own `AmbientSetupPhaseEnded` analog (probably not — follow-ups don't have a setup phase), (c) whether Stage 1's `ProcessFollowupForTask` relaxation already covers the empty-on-the-wire case end-to-end.
- **Drop the legacy `AppendedExchange`-driven teardowns** at `app/src/terminal/view.rs:5496-5507` and `app/src/terminal/view/ambient_agent/block/setup_command_text.rs:119-136` once enough time has passed that new viewers no longer need to support old sharers.
- **Revert the testing-only Cargo.toml local-path swaps** in `Cargo.toml:248` (warp-4) and `../session-sharing-server/server/Cargo.toml:36-40` to real `git = ..., rev = <merged SHA>` after `session-sharing-protocol` PR merges.
