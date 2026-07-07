# Empty-Prompt Local-to-Cloud Handoff — Stage 1 Sub-Tech-Spec (warp-4)
Sub-tech-spec for what **Stage 1** of REMOTE-1499 delivers on the warp-4 side. The full end-to-end architecture lives in `TECH.md`; the full product behavior lives in `PRODUCT.md`. This document is scoped to the contents of `harry/empty-prompt-handoff-wire-contract`.
Branch: `harry/empty-prompt-handoff-wire-contract` (warp-4 half of the paired-branch cross-repo PR shared with warp-server-4).
Sibling: `../../../warp-server-4/specs/REMOTE-1499/STAGE-1.md` (server-side relaxations).
## Scope
Stage 1 widens the wire shape of `SpawnAgentRequest.prompt` from `String` to `Option<String>` so a Stage 2 client can omit the field when the user submits an empty handoff. By itself Stage 1 introduces no interactive behavior changes: every interactive call site continues to send `Some(...)` of the same string, and the only non-test sites that emit `prompt: None` at runtime are the `oz agent run` CLI skill-only and conversation-only invocations. The skill-only path is end-to-end functional pre-Stage-2 because the warp-server-4 `prompt+skill` gate accepts the omitted field as the Go zero value. The conversation-only path requires the server-side validator relaxations that ship in the sibling warp-server-4 Stage 1 PR.
The server-side relaxations that accept additional shapes (empty `prompt` paired with `ConversationID`, `InitialSnapshotToken`, or rehydration metadata) land in the sibling warp-server-4 PR on the same branch name. Cross-repo coordination is purely through the JSON wire shape — there are no shared edit surfaces.
## Wire shape
`SpawnAgentRequest` in `app/src/server/server_api/ai.rs:206-208`:
```rust path=/Users/harryalbert/warp-4/app/src/server/server_api/ai.rs start=206
/// None for skill-only or conversation-only invocations; omitted on the wire.
#[serde(skip_serializing_if = "Option::is_none")]
pub prompt: Option<String>,
```
`Option<T>` serializes transparently under serde: `Some("hello")` emits `"prompt": "hello"`, and `skip_serializing_if = "Option::is_none"` causes `None` to omit the field entirely. The struct derives only `Serialize`, not `Deserialize`, so wire compatibility only has to hold client→server. Warp-server-4 deserializers that treat `prompt` as a string see the omitted field as the Go zero value `""`, which the existing skill-only validator already accepts — so the wire shape is compatible with both pre- and post-Stage-1 servers.
## Construction sites
All twelve `SpawnAgentRequest { … }` construction sites wrap their prompt value in `Some(...)`:
- `app/src/terminal/view/ambient_agent/model.rs:632` (`build_handoff_spawn_request`) and `:1120` (`spawn_agent`): both wrap the result of `extract_user_query_mode(prompt)`.
- `app/src/pane_group/pane/terminal_pane.rs:2137`: orchestration-spawned child runs.
- `app/src/ai/agent_sdk/ambient.rs:481-482`: `oz agent run` CLI (see CLI path below).
- Test fixtures: `spawn_tests.rs:702/770/838/901/1047`, `model_tests.rs:54`, `view_tests.rs:1323`, `mcp_config_tests.rs:272`, `ai_tests.rs:39`.
## CLI path
`app/src/ai/agent_sdk/ambient.rs:267-313` resolves the prompt as `Option<String>` directly:
- `Some(Prompt::PlainText(text)) → Some(text)`
- `Some(Prompt::SavedPrompt(id))` → resolves to `Some(prompt_text.to_string())` on hit; fatal-errors on miss
- `None → None` (skill-only or conversation-only invocations: `--skill` alone, `--conversation` alone, or `--skill` + `--conversation` with no prompt)
`ambient.rs:474-480` then computes the `(prompt, mode)` pair via a `match` that runs `extract_user_query_mode` only on the `Some` branch and defaults `mode` to `UserQueryMode::Normal` when the prompt is `None`. `UserQueryMode` is imported at the top of the file (`ambient.rs:6`). The resulting `Option<String>` flows directly into the constructed `SpawnAgentRequest` at `ambient.rs:481-482`.
These are the only non-test sites that emit `prompt: None` at runtime. The warp-server-4 `prompt+skill` gate at `agent_webhooks.go:343-347` accepts the skill-only case independently of the Stage 1 server relaxations, so the CLI's `--skill foo` flow continues to pass server validation even against an unupdated server. The `--conversation`-only flow depends on the Stage 1 server-side relaxations in the sibling warp-server-4 PR.
## Reader sites
Two reader sites use `.as_deref()` so the `Some` case dereferences to `&str` and the `None` case short-circuits cleanly:
- `app/src/terminal/view/ambient_agent/block/entry.rs:160` — entry-block title fallback chain:
  ```rust path=null start=null
  request.prompt.as_deref().and_then(Self::meaningful_title)
  ```
  A `None` prompt skips the fallback and the chain proceeds to the default title.
- `app/src/terminal/view/ambient_agent/view_impl.rs:159-164` — Cloud Mode Setup V2 queued-prompt insertion:
  ```rust path=null start=null
  request.prompt.as_deref()
      .map(|prompt| display_user_query_with_mode(request.mode, prompt))
  ```
  The existing `if !prompt.is_empty()` guard at `view_impl.rs:166` suppresses the queued-prompt block insertion when the prompt is `None`. Stage 2 reuses this short-circuit for the substituted-prompt UI variants.
No other code in warp-4 pattern-matches or destructures `SpawnAgentRequest.prompt`.
## Testing
### Unit tests
- `app/src/server/server_api/ai_tests.rs:66-89` — `spawn_agent_request_omits_prompt_when_none` constructs a `SpawnAgentRequest { prompt: None, ... }`, serializes to `serde_json::Value`, and asserts `value.get("prompt").is_none()`. This is the only test that exercises the `None` branch directly and pins the `skip_serializing_if` contract.
- `ai_tests.rs:37-64` (`spawn_agent_request_serializes_agent_uid_as_agent_identity_uid`) uses `Some("hello")` and round-trips the full struct through `serde_json::to_value`. Its assertions on the `agent_identity_uid` field name implicitly verify that `Some(String)` serializes transparently — a stray `{"Some": ...}` wrapping would break the round-trip.
- `app/src/ai/agent_sdk/mcp_config_tests.rs:272` (`serializes_mcp_servers_as_object_not_string`) uses `Some("hello")` and round-trips the struct to verify nested MCP config serialization; provides the same implicit guarantee for the prompt shape.
- `app/src/terminal/view/ambient_agent/model_tests.rs:143, 276, 339` and `app/src/terminal/view_tests.rs:920, 965` assert handoff auto-submit and cloud-mode dispatch payloads via `assert_eq!(request.prompt.as_deref(), Some("..."))`, pinning the exact prompt string under the `Option<String>` shape.
- `spawn_tests.rs` fixtures at `:702/770/838/901/1047` exercise the struct shape in spawn-task polling tests.
### Validation
- `cargo fmt --all --check`.
- `cargo check -p warp --tests`.
Nextest and full clippy are intentionally not part of the per-PR validation for this change — the diff is a mechanical type widening plus targeted reader updates, and the listed checks plus the per-stage unit tests cover the relevant surfaces.
## Risks and mitigations
- **Pre-Stage-1 servers receiving Stage-1+ client payloads.** Mitigated by `Option<T>`'s transparent serialization plus `skip_serializing_if = "Option::is_none"`: the common `Some` case emits the same JSON shape that pre-Stage-1 servers always accepted. Of the two `None`-emitting CLI paths, skill-only is already accepted by the `agent_webhooks.go` `prompt+skill` gate as the Go zero-value `prompt: ""`; conversation-only depends on the Stage 1 server-side relaxations and was already rejected by pre-Stage-1 servers regardless of whether the client sent `""` or omitted the field, so Stage 1 does not regress that path.
- **Post-Stage-1 servers receiving pre-Stage-1 client payloads.** Not a concern: the field is non-optional on the wire from a pre-Stage-1 client; the server deserializer tolerates presence or absence of the field equivalently.
- **Borrow-site regressions.** The two `&request.prompt` borrows in the repo go through `.as_deref()` chains; the `Some` case dereferences to `&str` identically to the pre-Stage-1 shape and the `None` case short-circuits cleanly. There are no `match` / `if let` destructures of `request.prompt` to migrate.
- **Stage-coupling risk.** Stage 1 alone never produces a `None` runtime value from any interactive flow — only the CLI skill-only and conversation-only paths can — so the additive server-side relaxations in the sibling warp-server-4 PR are not load-bearing for the skill-only CLI path. They are load-bearing for the conversation-only CLI path, but that path was already broken against pre-Stage-1 servers (which reject `prompt: ""` + no skill), so Stage 1 does not regress it. Reverting the server-side PR independently is safe modulo the conversation-only CLI flow.
## Follow-ups
Stage 1 is scaffolding for Stage 2. The behaviors that justify the wire-contract change — empty-prompt handoff via chip / `&` / `/handoff`, `continue in the cloud` substitution against an in-progress source, the queued-prompt indicator label variants, the worker-derived skip-initial-turn wiring, and the `CloudModeSetupPhaseEnded` setup-phase teardown — are specced under `STAGE-2.md` on `harry/empty-prompt-handoff-local`. Stage 1 introduces no `FeatureFlag::EmptyPromptHandoff` itself; that flag lands on the Stage 2 branch.
