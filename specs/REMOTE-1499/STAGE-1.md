# Empty-Prompt Local-to-Cloud Handoff — Stage 1 Sub-Tech-Spec (warp-4)
Sub-tech-spec for what **Stage 1** of REMOTE-1499 delivers on the warp-4 side. The full end-to-end architecture lives in `TECH.md`; the full product behavior lives in `PRODUCT.md`. This document covers only the changes that ship on `harry/empty-prompt-handoff-wire-contract`.
Branch: `harry/empty-prompt-handoff-wire-contract` (warp-4 half of Stage 1's paired-branch cross-repo PR).
Sibling: `../../../warp-server-4/specs/REMOTE-1499/STAGE-1.md` (Stage 1 server-side relaxations).
## Scope
Stage 1 is purely a wire-contract change: widen `SpawnAgentRequest.prompt` from `String` to `Option<String>` so a later Stage 2 client can omit the field on the wire when the user submits an empty handoff. Stage 1 by itself does not change any interactive runtime behavior — every existing call site continues to send `Some(...)` of the same string it sent before, and no interactive flow produces `None` until Stage 2 turns the `EmptyPromptHandoff` client flag on. The only non-test site that emits `prompt: None` at runtime after Stage 1 is the `oz agent run` CLI skill-only / saved-prompt-only path, which already worked end-to-end pre-feature thanks to the server's pre-existing `prompt+skill` gate.
The server-side relaxations that accept the new shape land in the sibling warp-server-4 PR on the same branch name. The two sides only touch each other through the JSON wire shape and have no cross-edit conflicts.
## Pre-implementation state
- `app/src/server/server_api/ai.rs` — `SpawnAgentRequest` was a `#[derive(Serialize)]` struct whose `prompt` field was a required `String`. The struct is the JSON payload sent to `POST /agent/run` via `AIClient::spawn_agent` (`app/src/server/server_api/ai.rs:1697-1703`).
- Twelve literal `SpawnAgentRequest { … }` construction sites across production code, CLI, and tests built the struct with a `prompt: String` value. The two non-test paths are `app/src/ai/agent_sdk/ambient.rs` (the `oz agent run` CLI) and `app/src/pane_group/pane/terminal_pane.rs` (orchestration-spawned child runs). The two interactive submit paths live in `app/src/terminal/view/ambient_agent/model.rs:622-649` (`build_handoff_spawn_request`, used by handoff auto-submit) and `app/src/terminal/view/ambient_agent/model.rs:1111-1139` (`spawn_agent`, used by normal cloud-mode submits). The remaining sites are fixtures in `spawn_tests.rs`, `model_tests.rs`, `view_tests.rs`, `mcp_config_tests.rs`, and `ai_tests.rs`.
- Two readers held `&request.prompt` borrows: `app/src/terminal/view/ambient_agent/block/entry.rs:160` (entry-block title fallback) and `app/src/terminal/view/ambient_agent/view_impl.rs:159` (Cloud Mode Setup V2 queued-prompt rendering via `display_user_query_with_mode`).
- The CLI path at `app/src/ai/agent_sdk/ambient.rs` carried a long-standing TODO acknowledging that skill-only and saved-prompt-only invocations were sending `prompt: ""` purely to satisfy the required-string contract; the comment proposed making the field optional as the right fix.
- `SpawnAgentRequest` derives only `Serialize`, not `Deserialize`. Wire-format compatibility therefore only needs to hold client→server: any new shape the client emits must continue to be accepted by both old and new servers.
## Changes delivered in Stage 1
### 1. Widen the field type
`app/src/server/server_api/ai.rs:205-207`:
```rust path=/Users/harryalbert/warp-4/app/src/server/server_api/ai.rs start=205
#[serde(skip_serializing_if = "Option::is_none")]
pub prompt: Option<String>,
```
`Option<T>` serializes transparently under serde, so `Some("hello")` continues to emit `"prompt": "hello"` byte-for-byte identical to the pre-change wire shape. `skip_serializing_if = "Option::is_none"` ensures `None` omits the field entirely rather than emitting `"prompt": null`. Old warp-server-4 versions that decode `prompt` as `string` see the omitted field as the zero value `""` (the same value they already accept from the existing skill-only path), so the change is backwards-compatible against unupdated servers.
### 2. Wrap every constructor in `Some(...)`
All twelve `SpawnAgentRequest { … }` literal construction sites are updated to wrap their existing prompt value in `Some(...)`. The two interactive submit paths in `app/src/terminal/view/ambient_agent/model.rs:632` and `:1120` always wrap the result of `extract_user_query_mode(prompt)` so handoff and normal cloud-mode submits keep emitting the same JSON. `app/src/pane_group/pane/terminal_pane.rs:2137` wraps the orchestration child's `request.prompt`. Each fixture in `spawn_tests.rs:702/770/838/901/1047`, `model_tests.rs:54`, `view_tests.rs:1323`, `mcp_config_tests.rs:272`, and `ai_tests.rs:39` is updated in place.
### 3. CLI path: send `None` for skill-only and saved-prompt-only invocations
`app/src/ai/agent_sdk/ambient.rs:267-313` previously coerced "no prompt" into `String::new()` because the field was required, then unconditionally piped that empty string through `extract_user_query_mode`. The new code propagates `Option<String>` through the resolution branch:
- `Some(Prompt::PlainText(text)) → Some(text)`
- `Some(Prompt::SavedPrompt(id)) →` resolves to `Some(prompt_text.to_string())` on hit, fatal-errors on miss
- `None → None`
The pre-existing TODO comment about making the field optional is removed.
The extracted `(prompt, mode)` pair is then computed by a `match` at `ambient.rs:474-480` that runs `extract_user_query_mode` only on the `Some` branch and defaults `mode` to `UserQueryMode::Normal` when the prompt is `None`. `UserQueryMode` is imported at the top of the file (`ambient.rs:6`) per the repo's import-at-top convention. The new `None` value is plumbed straight into the constructed `SpawnAgentRequest` at `ambient.rs:481-482`.
This is the only non-test site that now emits `prompt: None` at runtime. The server already accepted the original `prompt: ""` skill-only case via the `prompt+skill` gate at `agent_webhooks.go:343-347`; omitting the field deserializes to the same Go zero value, so the CLI's `--skill foo` flow continues to pass server validation without depending on the Stage 1 server-side relaxation.
### 4. Update the two `&request.prompt` readers
- `app/src/terminal/view/ambient_agent/block/entry.rs:160`: the entry block's title fallback chain now reads
  ```rust path=null start=null
  request.prompt.as_deref().and_then(Self::meaningful_title)
  ```
  so a `None` prompt simply skips the fallback and the chain proceeds to the default title.
- `app/src/terminal/view/ambient_agent/view_impl.rs:159-164`: the Cloud Mode Setup V2 queued-prompt insertion now reads
  ```rust path=null start=null
  request.prompt.as_deref()
      .map(|prompt| display_user_query_with_mode(request.mode, prompt))
  ```
  When prompt is `None`, the resulting display string is empty and the existing `if !prompt.is_empty()` guard at `view_impl.rs:166` suppresses the queued-prompt block insertion — exactly the same behavior Stage 2 will rely on for the substituted-prompt UI variants.
No other code in warp-4 pattern-matches or destructures `SpawnAgentRequest.prompt`, so no further reader changes are needed.
## Testing
### Unit tests
- `app/src/server/server_api/ai_tests.rs:66-89` — new test `spawn_agent_request_omits_prompt_when_none` constructs a `SpawnAgentRequest { prompt: None, ... }`, serializes to `serde_json::Value`, and asserts `value.get("prompt").is_none()`. This is the only test that exercises the `None` branch directly; it pins the `skip_serializing_if` contract.
- The existing test at `ai_tests.rs:37-64` (`spawn_agent_request_serializes_agent_uid_as_agent_identity_uid`) is updated to use `Some("hello")` and continues to round-trip the full struct through `serde_json::to_value`. Its assertions on the `agent_identity_uid` rename implicitly verify that `Some(String)` serializes transparently — a failing `{"Some": ...}` wrapping would break the round-trip.
- `app/src/ai/agent_sdk/mcp_config_tests.rs:272` (`serializes_mcp_servers_as_object_not_string`) uses `Some("hello")` and round-trips the struct to verify nested MCP config serialization; same implicit guarantee for the prompt shape.
- `app/src/terminal/view/ambient_agent/model_tests.rs:143, 276, 318, 339` and `app/src/terminal/view_tests.rs:920, 965` are updated from `assert_eq!(request.prompt, "...")` to `assert_eq!(request.prompt.as_deref(), Some("..."))`, so handoff auto-submit and cloud-mode dispatch test coverage keeps asserting the exact prompt string post-refactor.
- `spawn_tests.rs` fixtures at `:702/770/838/901/1047` are updated mechanically to keep the spawn-task polling tests compiling and exercising the new struct shape.
### Validation
- `cargo fmt --all --check`.
- `cargo check -p warp --tests`.
Per the orchestrator's standing instruction for client refactors of this kind, nextest and full clippy are skipped.
## Risks and mitigations
- **Old servers continuing to receive new-client payloads.** Mitigated by `Option<T>`'s transparent serialization plus `skip_serializing_if = "Option::is_none"`: the wire shape for the common `Some` case is byte-identical to the pre-change payload, and the only path that produces `None` at runtime (the CLI skill-only / saved-prompt-only branch) was already accepted by the original `agent_webhooks.go` `prompt+skill` gate as `prompt: ""`. Old servers deserialize the omitted field into the Go zero value and proceed.
- **New servers receiving old-client payloads.** Not a concern: old clients always send `"prompt": "..."`. The server's deserialization tolerates either presence or absence of the field.
- **Borrow-site regressions.** The only two `&request.prompt` borrows in the repo are both updated to `.as_deref()` chains that preserve the previous behavior in the `Some` case and short-circuit cleanly in the `None` case. There are no `match` / `if let` destructures of `request.prompt` to migrate.
- **Stage-coupling risk.** Stage 1 alone never produces a `None` runtime value from any interactive flow — only the CLI skill-only path can — so the additive server-side relaxations in the sibling warp-server-4 PR are not load-bearing for warp-4 Stage 1 to ship. Reverting the server-side PR independently is safe.
## Follow-ups
Stage 1 is purely scaffolding for Stage 2. The behaviors that justify the wire-contract change — empty-prompt handoff via chip / `&` / `/handoff`, `continue in the cloud` substitution against an in-progress source, the queued-prompt indicator label variants, the worker-derived skip-initial-turn cutover, and the `AmbientSetupPhaseEnded` setup-phase teardown — are specced under `STAGE-2.md` on `harry/empty-prompt-handoff-local`. Stage 1 introduces no `FeatureFlag::EmptyPromptHandoff` itself; that flag lands in Stage 2a.
