# Local ACP — Task List

Spec: `specs/local-acp/TECH.md`

## P0 — Claude end-to-end

- [x] Add `FeatureFlag::LocalAcp` in `crates/warp_features` + `app/src/features.rs` + `app/Cargo.toml`
- [x] Use `acpx` + `agent-client-protocol`: subprocess transport, typed requests, `session/update` streaming
- [x] Unit tests: local ACP setup, submit routing, model selector wiring
- [x] Create `app/src/ai/acp/registry.rs`: Claude spawn spec, auth, model env, install URL
- [x] Create `app/src/ai/agent_sdk/driver/harness/acp.rs`: `ThirdPartyHarness` + `HarnessRunner` for Claude
- [x] Wire `AcpHarness` in `driver/harness/mod.rs` + `HarnessKind` selection in `agent_sdk/mod.rs`
- [x] Route local agent submit (`input.rs`) to ACP driver when harness ≠ Oz and flag on
- [x] Map `session/update` chunks → blocklist exchange streaming
- [x] Handle `session/request_permission` → approval UI (`allow-once` default for v1)
- [x] Bypass Warp AI quota gate when ACP harness active (`prompt_alert.rs` or equivalent)
- [x] Show harness selector in local agent view (`profile_model_selector.rs` / agent input footer)
- [x] Extend `local_harness_setup.rs`: detect `claude-agent-acp` on PATH
- [x] Persist harness + model via `CloudAgentSettings::persist_harness_model_selection`
- [x] Integration test: mock ACP subprocess → prompt → streamed response → conversation metadata

## P1 — Codex + Gemini

- [x] Registry entries: Codex (`codex-acp`), Gemini (`gemini --acp`)
- [x] Gemini model: call `unstable_setSessionModel` before prompt
- [x] Codex model: codex-acp CLI flag / env per registry
- [x] Model picker: merge `HarnessAvailabilityModel::models_for` + registry `default_models`
- [x] `local_harness_setup.rs`: detect `codex-acp`, `gemini`
- [x] Tests: model list switches per harness; disabled state when binary missing

## P2 — Cursor

- [x] Add `Harness::Cursor` to `warp_cli::agent::Harness`
- [x] Thread through `harness_display`, `from_config_name`, `HarnessAvailabilityModel` mapping
- [x] Registry entry: `cursor-acp`, `cursor_login` auth
- [x] Handle Cursor ACP extensions (`cursor/ask_question`, `cursor/update_todos`, etc.) as needed for v1
- [x] Tests: Cursor harness in picker; spawn + authenticate smoke test

## P3 — Polish

- [x] Registry entry: Devin (`devin acp`)
- [x] `session/load` resume for agents that support it
- [x] Per-agent slash commands in slash menu; forward verbatim to ACP prompt
- [x] Permission UI: `allow-always` / `reject-once` options
- [x] MCP: pass `mcpServers` into `session/new` from project config
- [x] Telemetry: harness selected, model selected, ACP session lifecycle events

## Server / GraphQL (if needed for model catalog)

- [ ] Confirm `availableHarnesses` includes Cursor (or client-only discovery is sufficient for local)
- [ ] Add Cursor to `AgentHarness` GraphQL enum if server-side model list required
