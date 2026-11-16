# Tombstone metadata for cloud agent runs — Tech Spec

Pairs with `PRODUCT.md`.

## Problem
`ConversationEndedTombstoneView` reads `run_time` / `credits` / `artifacts` off the in-memory `AIConversation`. Non-Oz cloud runs don't materialize one (they produce a `CLIAgentConversation`), so all three stay empty. Cloud Oz uses values that under-report relative to the details panel: exchange-derived run time excludes the cloud lifecycle; `credits_spent()` excludes compute cost; conversation-side artifacts can lag the task copy. The artifact buttons row is also constructed eagerly in the constructor and never refreshed, so the async task fetch has nothing to push artifacts into.

Separately, both the tombstone and `ConversationDetailsPanel` unconditionally surface the `Continue locally` button on non-transcript runs. `ContinueConversationLocally` only forks Oz-harness conversations, so for non-Oz tasks the button is dead UI.

## Relevant code
- `app/src/terminal/view/shared_session/conversation_ended_tombstone_view.rs` — `TombstoneDisplayData`, `from_conversation`, `enrich_from_task` (`#[cfg(not(target_family = "wasm"))]`), `render_metadata_row`, `render_action_buttons`.
- `app/src/ai/conversation_details_panel.rs` — `ConversationDetailsData::from_task` (~line 310), `ConversationDetailsData::harness`, `ConversationDetailsPanel::continue_locally_conversation_id`. Consistency reference: sets both fields straight from the task.
- `app/src/ai/ambient_agents/task.rs` — `AmbientAgentTask::run_time()` (`Option<chrono::Duration>`), `credits_used()` (`Option<f32>`), `AgentConfigSnapshot::harness` (`Option<HarnessConfig>`), `HarnessConfig::harness_type` (`Harness`).
- `crates/warp_cli/src/agent.rs` — `Harness` enum (`Oz`, `Claude`, `Gemini`).
- `format_credits` (`app/src/ai/blocklist/view_util.rs`) and `human_readable_precise_duration` (`app/src/util/time_format.rs`) — already imported by the tombstone.

## Change
Extend `enrich_from_task` to overwrite `run_time`, `credits`, and `artifacts` from the task whenever the task has values, and refresh the artifact buttons view after the async task fetch resolves. Mirrors `ConversationDetailsData::from_task`.

```rust path=null start=null
#[cfg(not(target_family = "wasm"))]
fn enrich_from_task(&mut self, task: AmbientAgentTask) {
    // existing: title / source / skill / error ...

    if let Some(d) = task.run_time() {
        self.run_time = Some(human_readable_precise_duration(d));
    }
    if let Some(c) = task.credits_used() {
        self.credits = Some(format_credits(c));
    }
    if !task.artifacts.is_empty() {
        self.artifacts = task.artifacts;
    }
}
```

The constructor now always materializes `artifact_buttons_view` (passing whatever `display_data.artifacts` start with, possibly empty), and the spawn callback that handles the task fetch calls `update_artifacts(&me.display_data.artifacts, ctx)` after enrichment. Render gates the row on `display_data.artifacts.is_empty()`.

### Continue Locally harness gate

`TombstoneDisplayData` gains one field: `harness: Option<Harness>`, defaulting to `None`. `enrich_from_task` sets it from `task.agent_config_snapshot`: snapshot present + explicit harness → `Some(harness)`; snapshot present + no explicit harness → `Some(Oz)` (default); snapshot absent → stays `None`.

`render_action_buttons` (cfg `not(wasm)`) and `ConversationDetailsPanel::continue_locally_conversation_id` both apply the same gate: `!matches!(harness, Some(h) if h != Harness::Oz)`. `None` and `Some(Oz)` both show the button; only confirmed non-Oz hides it. No `is_task` flag is needed — plain conversations and pre-load tasks naturally fall into the `None` bucket.

`ConversationDetailsData::from_task` already populates `harness`, so no upstream wiring changes are required for the panel.

Wasm `Open in Warp` is intentionally untouched: it just opens the same conversation in the desktop client, where the same gate hides `Continue locally`.

Notes:
- All three metadata fields are unguarded: when the task has a value, it wins. When it doesn't, whatever `from_conversation` populated (or default-empty for non-Oz) survives.
- Local Oz runs have no `task_id`, so `enrich_from_task` never runs and `from_conversation` remains the only source. No regression there.
- Tombstone keeps a single aggregated `Credits used: X.X` segment; the panel's inference/compute split (`render_credits_with_split`) is panel-only.
- The `artifact_buttons_view` field is no longer `Option`. The view returns `Empty` when its button list is empty, but we still render-gate to avoid the surrounding margin container.
- The harness gate is permissive on `None`: cloud non-Oz tasks briefly show the button until the fetch resolves and switches `harness` to `Some(non-Oz)`.
- No new fetches, no schema work, no GraphQL changes.

## Risks
- **Cloud Oz numbers visibly change** (both up). Intentional; matches the details panel for the same run. No flag-gating; if surprising in dogfood, can hide behind `FeatureFlag::AgentHarness` after the fact.
- **Brief flash** as `from_conversation` values render before the async task fetch resolves and overwrites them. Same race the panel has today; acceptable.
- **`Continue locally` flashes briefly on non-Oz tombstones** before the task fetch resolves. Conscious trade-off: hiding eagerly would also hide the button on local conversations and cloud Oz runs (we don't know the harness yet), which is worse.
- **Snapshot missing harness on a non-Oz run** mis-shows `Continue locally` because `Some(snapshot) + harness == None` defaults to `Some(Oz)`. In practice the server always populates `harness` when a non-Oz run is dispatched; if that ever changes the gate becomes too permissive.
- **Wasm**: `enrich_from_task` is `cfg(not wasm)`, so cloud tombstones on web are unchanged. Pre-existing gap shared with the panel. The wasm `Open in Warp` button stays visible because it just routes the user to the desktop tombstone, which then enforces the gate.
- **Partial task data**: if `run_time()` or `credits_used()` is `None`, we leave whatever `from_conversation` set (avoids blanking Oz when the task is partially populated).

## Tests
Unit tests next to `conversation_ended_tombstone_view.rs`:
1. Oz with task: run time / credits overwritten by task values.
2. Oz with task missing those fields: `from_conversation` values preserved.
3. Non-Oz happy path: empty defaults populated from a fully-populated task.
4. Task artifacts populate empty defaults.
5. Task artifacts override conversation artifacts.
6. Empty `task.artifacts` preserve conversation artifacts.
7. Task without `agent_config_snapshot` leaves `harness == None`.
8. Snapshot without explicit harness defaults to `Some(Harness::Oz)`.
9. Snapshot with explicit harness propagates `Some(harness)` for each variant.

Details panel tests (`conversation_details_panel_tests.rs`) already cover `from_task` populating `harness` for each variant and `from_conversation_metadata` passing it through, which is the input to the new `continue_locally_conversation_id` gate.

No cross-surface test: tests 1 and 3 already assert tombstone output equals `human_readable_precise_duration(task.run_time())` and `format_credits(task.credits_used())`, which is the same data path the panel feeds into `ConversationDetailsData::from_task`. A literal panel-vs-tombstone string comparison would just rerun the same formatters and require widening visibility on private panel internals.

Manual: cloud Claude Code (`Continue locally` hidden in both surfaces once the task loads), cloud Oz (numbers should match the details panel; `Continue locally` shown), local Oz (unchanged). Run `./script/presubmit` before PR.

## Follow-ups
- Wasm parity (fetch the task and run `enrich_from_task` on web; share with the panel's wasm path).
- `Directory:` / `Branch:` for non-Oz via a future `POST /harness-support/report-context` sibling endpoint piggy-backed under `try_join!` with the existing uploads.
- Shared helper translating `AmbientAgentTask` → display strings if a third surface ever needs it.
- If/when `ContinueConversationLocally` learns to fork non-Oz runs, drop the harness gate (or invert it to whatever the new constraint is).
