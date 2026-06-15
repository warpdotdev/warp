# Local ACP — Tech Spec

GitHub: https://github.com/warpdotdev/warp/issues/9233

ACP references:
- Claude (prior art): https://github.com/warpdotdev/warp/commit/50346158548e641c3fb8cce53ec8a7a1c0a2d15d
- Cursor: https://github.com/raphaelluethy/cursor-acp
- Codex: https://github.com/zed-industries/codex-acp
- Gemini: https://geminicli.com/docs/cli/acp-mode/
- Devin: https://docs.devin.ai/cli/reference/commands#devin-acp

## Goal

Run third-party agents locally via [ACP](https://agentclientprotocol.com/) (stdio JSON-RPC ndjson). User picks agent + model in local agent view. Agent uses its own auth/subscription, not Warp AI credits.

## Context

Today:
- Local agent view routes prompts to server-side Oz.
- Third-party harnesses (`Claude`, `Codex`, `OpenCode`, `Gemini`) run via terminal CLI spawn + block output scraping (`app/src/ai/agent_sdk/driver/harness/`).
- Harness picker + model picker exist for cloud ambient agent (`harness_selector.rs`, `model_selector.rs`, `orchestration_controls.rs`).
- `HarnessAvailabilityModel` caches server harness + model catalogs (`app/src/ai/harness_availability.rs`).
- No ACP client code in repo.

ACP replaces terminal scraping with structured `session/update` streaming for local non-Oz agents.

## Architecture

```
Local agent view submit
  → AcpHarness (ThirdPartyHarness + HarnessRunner)
    → acpx::Connection (subprocess stdio)
      → agent-client-protocol typed requests
        → initialize → authenticate → session/new → session/prompt
  → Map session/update → blocklist exchanges
  → Map session/request_permission → Warp approval UI
```

Headless subprocess. Do **not** use `local_harness_launch.rs` hidden terminal panes for ACP.

## 1. ACP transport layer

**New:** `app/src/ai/acp/` using `acpx` and `agent-client-protocol`

| Component | Responsibility |
|-----------|----------------|
| `acpx::Connection` | Spawn subprocess; official ACP SDK over stdio |
| `agent-client-protocol` | Typed request/response protocol and notification dispatch |
| Submit path | `initialize` → `authenticate` → `session/new` \| `session/load` → `session/prompt` → `session/cancel` |
| Event types | `session/update`, `session/request_permission`, vendor extensions (`cursor/*`, etc.) |

**Tests:** mock subprocess with canned ndjson; streaming, permission round-trip, disconnect.

## 2. Agent registry

**New:** `app/src/ai/acp/registry.rs`

| Harness | Spawn | Auth | Model |
|---------|-------|------|-------|
| `Claude` | `claude-agent-acp` / `npx @anthropic/claude-agent-acp` | vendor OAuth | `ANTHROPIC_MODEL` env or session arg |
| `Codex` | `codex-acp` / `npx @zed-industries/codex-acp` | `CODEX_API_KEY` / ChatGPT sub | codex-acp flag |
| `Gemini` | `gemini --acp` | Google OAuth | `unstable_setSessionModel` |
| `Cursor` | `cursor-acp` | `cursor_login` | session / default model |
| `Devin` | `devin acp` | `devin auth login` / `WINDSURF_API_KEY` | `--model` or session |

Per entry: `binary_resolver` (`resolve_executable`), `install_docs_url`, `supports_resume`, `default_models: &[HarnessModelInfo]`.

**Extend `Harness`** (`crates/warp_cli/src/agent.rs`): add `Cursor` (+ `Devin` if in scope). Thread through `from_config_name`, `harness_display`, GraphQL mapping.

## 3. ACP harness driver

**New:** `app/src/ai/agent_sdk/driver/harness/acp.rs`; wire in `mod.rs`.

Implement `ThirdPartyHarness` + `HarnessRunner` on `AcpSession`:

- `build_runner()` → spawn process, `session/new(cwd, mcpServers)`
- `start()` → `session/prompt`, stream `session/update` → `AgentDriver` events
- `save_conversation()` → persist `sessionId` + transcript from ACP updates

**Event mapping:**
- `agent_message_chunk` → blocklist exchange streaming
- `tool_call` / `tool_result` → existing tool UI (collapsed OK for v1)
- `session/request_permission` → approval modal (`allow-once` / `allow-always` / `reject-once`)

**Model:** apply `HarnessModelConfig` via registry (env for Claude; `unstable_setSessionModel` for Gemini; etc.). Reuse `harness_model_env_vars` where applicable.

**Quota:** skip Warp AI credit gate when ACP harness active (same pattern as [5034615](https://github.com/warpdotdev/warp/commit/50346158548e641c3fb8cce53ec8a7a1c0a2d15d)).

**Flag:** `FeatureFlag::LocalAcp` (dogfood).

## 4. Local routing

| File | Change |
|------|--------|
| `app/src/terminal/input.rs` | Non-Oz harness + `LocalAcp` → ACP driver, not server Oz |
| `app/src/terminal/view/ambient_agent/model.rs` | Local path reads `harness`, `harness_model_id`, `harness_reasoning_level` |
| `app/src/ai/blocklist/controller.rs` | Local conversation metadata tagged with harness |
| `app/src/ai/agent_sdk/mod.rs` | Local branch selects `AcpHarness` when flag + harness match |

## 5. Agent + model selectors

Reuse existing UI; extend for local ACP.

| Component | File | Change |
|-----------|------|--------|
| Harness picker | `harness_selector.rs`, `orchestration_controls.rs` | Show ACP agents where `registry::is_installed()` + flag |
| Model picker | `model_selector.rs`, `populate_model_picker_for_harness` | Per harness: `HarnessAvailabilityModel::models_for` ∪ registry `default_models`; "Default model" first |
| Local input bar | `profile_model_selector.rs` | Show harness + model selectors when `LocalAcp` + non-Oz |
| Setup state | `local_harness_setup.rs` | Install checks for ACP binaries |

**Persistence:** `CloudAgentSettings::persist_harness_model_selection` (existing per-harness map).

**Visibility:** show when `available_acp_agents().len() >= 1` (Oz remains fallback).

## 6. Auth & MCP

| Concern | Approach |
|---------|----------|
| Auth | `authenticate` RPC; on failure surface install/login instructions |
| MCP | Pass config into `session/new` `mcpServers`; reuse `resolve_mcp_specs_to_json` |
| Permissions | Block tools until `session/request_permission` answered |

## 7. Slash commands (v1 minimal)

Forward `/`-prefixed input verbatim to ACP prompt. Static per-agent command list in slash menu, gated on active harness (Claude: `/clear`, `/compact`, `/resume`, etc.).

## Phases

| Phase | Scope |
|-------|-------|
| **P0** | `acp_client` + `AcpHarness` for Claude; harness picker in local agent view; model via `ANTHROPIC_MODEL` |
| **P1** | Codex + Gemini; model picker wired |
| **P2** | Cursor; `Harness::Cursor` |
| **P3** | Devin; `session/load` resume; slash commands; permission UI polish |

## Tests

| Area | Coverage |
|------|----------|
| `acp_client` | ndjson parse, RPC correlation, stream reassembly |
| `acp.rs` | Mock ACP server: prompt → chunks → conversation save |
| `local_harness_setup` | Binary detection per agent |
| `model_selector` | Model list switches on harness change |
| `orchestration_controls` | ACP harness visible; disabled when CLI missing |

## Out of scope (v1)

- Cloud ambient agent over ACP
- Oz over ACP
- Terminal block scraping for ACP agents
- Team admin harness overrides for local ACP
