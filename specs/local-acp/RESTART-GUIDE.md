# Local ACP — Complete Handoff & Restart Guide

> **Starting over in a new chat?** Read **[START-FROM-SCRATCH.md](./START-FROM-SCRATCH.md)** first — full scratch from upstream, phase order, optional git reset, and the prompt to paste into the new context. You do not need to reset the work tree; you can keep pass-1 new files as read-only reference or delete everything and rebuild.

**Purpose:** Record everything implemented in this branch, explain **why every existing source file had to be touched** (or which touches were mistakes), and define how to **restart from scratch** building a **Local-ACP-only Warp** — no cloud agent, no runtime flag to enable the feature.

This is a **feature replacement**, not an optional add-on.

Related: [TECH.md](./TECH.md), [TASKS.md](./TASKS.md), [MINIMAL-INTEGRATION.md](./MINIMAL-INTEGRATION.md) (condensed index).

---

## Product definition (v2 target)

| Remove entirely | Keep / replace with |
|-----------------|---------------------|
| Cloud ambient agent (`AmbientAgentViewModel` spawn, environments, Docker modal) | Local ACP subprocess per agent |
| Server-side Oz (`send_user_query_in_new_conversation`) | `ExecuteLocalAcpQuery` → `acp_client` |
| Cloud handoff, follow-ups, `/cloud-agent` | `/harness`, `/model`, local agent slash commands |
| Runtime `FeatureFlag::LocalAcp` toggle | **Always on** — part of the product, not a flag |
| Terminal CLI scraping for third-party agents | ACP `session/update` streaming |

**There is no flag to activate.** Users open an agent pane and pick a local harness — that is the only agent mode.

### v2 build model (replace flag gating)

The first pass used `FeatureFlag::LocalAcp` for gradual rollout inside Warp dogfood. **For your fork, delete that layer:**

| First pass (dogfood) | v2 replacement-only |
|----------------------|---------------------|
| `FeatureFlag::LocalAcp.is_enabled()` | Remove — behavior is unconditional |
| `cloud_agent_disabled()` → checks flag | Rename to always-true or **delete cloud paths** instead of guarding |
| `#[cfg(feature = "local_acp")]` on modules | Keep as **compile-time** gate only (or merge into default build permanently) |
| `features.rs` registers `LocalAcp` | Remove registration; cloud agent flags can stay off or be deleted from build |
| `LOCAL_FLAGS` / `DOGFOOD_FLAGS` entry | Remove |

Keep `local_acp` in `app/Cargo.toml` **default features** so the binary always includes ACP. Routing helpers in `app/src/ai/acp/mod.rs` should assume local ACP is the agent backend — no runtime branch.

---

## Why existing files must be touched at all

Warp's agent mode is not a plugin system. There is no extension point to register a new "agent backend" without hooking the places that today hard-code:

1. **Submit** — `Input::submit_ai_query` calls `BlocklistAIController::send_user_query_*` (Oz server).
2. **Model lifecycle** — `TerminalView::new` creates `AmbientAgentViewModel` for cloud panes.
3. **Harness UI** — `HarnessSelector` / `ModelSelector` hold `ModelHandle<AmbientAgentViewModel>`.
4. **Driver selection** — `harness_kind()` returns `ClaudeHarness` / `CodexHarness` (terminal scraping).
5. **Blocklist streaming** — server SSE streams; local needs a parallel local stream id.
6. **Harness enum** — `warp_cli::agent::Harness` is exhaustive-matched in ~20 files; new agents require enum variants.

**You cannot implement Local ACP in new files alone.** New files hold all *logic*; existing files hold the *wiring* because Warp never extracted an `AgentBackend` trait at those boundaries.

The design goal for v2: **maximize logic in new modules, minimize wiring to the smallest set of choke points** (listed below as REQUIRED).

---

## Complete file inventory

### A. New files only (no upstream risk — copy wholesale)

```
crates/acp_client/                         # Transport: subprocess, JSON-RPC, AcpSession
app/src/ai/acp/                            # All integration logic (15 files)
app/src/ai/agent_sdk/driver/harness/acp.rs  # AcpHarness driver
app/src/ai/blocklist/local_acp_stream.rs    # Blocklist streaming
app/src/terminal/view/ambient_agent/harness_selection.rs  # UI backend enum
```

These contain: registry, path search, submit model, permissions, session store, telemetry, slash forwarding, harness picker model, tests.

---

### B. REQUIRED modifications — functional wiring (cannot avoid)

Each entry: **what changed → why necessary → why no alternative**.

#### Build & dependency graph

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `Cargo.toml` | Workspace dep `acp_client` | Rust must resolve the new crate | Cargo has no auto-discovery |
| `Cargo.lock` | Lock `acp_client` | Reproducible builds | Standard Cargo |
| `app/Cargo.toml` | Feature `local_acp = ["agent_harness", "dep:acp_client"]` in **default** | Links ACP into the app binary | Optional dep must be declared; v2 keeps in default permanently |
| `app/src/ai/mod.rs` | `pub(crate) mod acp` | Makes module visible to `input.rs`, `view.rs`, driver | Rust module tree is explicit |
| `app/src/lib.rs` | Register 3 singletons at app init | `LocalAcpSubmitModel`, `SessionStore`, `PermissionModel` must exist before first submit | Warp singletons are registered only in `initialize_app` |

#### Shared harness type (enum extension)

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `crates/warp_cli/src/agent.rs` | Add `Harness::Cursor`, `Harness::Devin` + parsing/display | Harness is the cross-crate ID for agent identity | Cannot add agents without extending this enum; used in settings persistence, UI, driver |
| `crates/warp_cli/src/agent_tests.rs` | Test new variants | Same | Tests follow enum |
| `app/src/ai/harness_display.rs` | Display names, icons, colors for Cursor/Devin | Harness picker renders via `harness_display` | Central display registry — duplicating in ACP-only module would fork UI |

#### Harness catalog (replaces server fetch for local mode)

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `app/src/ai/harness_availability.rs` | `local_acp_harnesses()`, `harnesses_for_selector()`, `models_for_picker()`, PATH refresh, `has_any_enabled_harness()` fix | Picker needs harness list + models **without GraphQL**; GUI PATH fix | `HarnessAvailabilityModel` is the singleton every picker reads — no indirection hook exists |
| `app/src/ai/local_harness_setup.rs` | ACP binary install detection per harness | Disabled state + tooltips in picker | `local_harness_setup_state()` is called from harness selector — existing API |
| `app/src/ai/local_harness_setup_tests.rs` | Tests for ACP detection paths | Same | — |

#### Agent driver (submit execution)

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `app/src/ai/agent_sdk/driver/harness/mod.rs` | `third_party_harness_for()` → `AcpHarness` when harness supported; `AcpResumeInfo` | All third-party execution goes through `harness_kind()` | Driver factory is a single `match` — new driver type must register here |
| `app/src/ai/agent_sdk/mod.rs` | `"cursor"` / `"devin"` orchestration labels | Telemetry / metadata strings | Exhaustive harness label match |

#### Blocklist (conversation UI)

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `app/src/ai/blocklist/mod.rs` | Export `LocalAcpBlocklistStream` | Submit model writes exchanges through blocklist API | Module visibility |
| `app/src/ai/blocklist/controller/response_stream.rs` | `ResponseStreamId::new_local()` | Server streams use request-id-based ids; local ACP has no server request | Stream id type is shared — one new constructor |
| `app/src/ai/blocklist/prompt/prompt_alert.rs` | Skip Warp AI quota alert when ACP active | ACP uses agent's own subscription, not Warp credits | Quota gate is centralized in `PromptAlertView::determine_state` |
| `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` | Pass `local_acp_harness_model` to ModelSelector; show selector when ACP harness has models | Footer is alternate harness UI surface in agent view | ModelSelector constructor is defined here for footer variant |
| `app/src/ai/blocklist/inline_action/orchestration_controls.rs` | `models_for_picker`, `local_acp_supports_harness_models` | Child-agent model dropdown must use local catalog for ACP Codex | Shared `populate_model_picker_for_harness` helper |

#### Terminal — the six critical hooks

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| **`app/src/terminal/view.rs`** | (1) Create `LocalAcpHarnessModel` when `!is_cloud_mode` (2) **Do not** create `AmbientAgentViewModel` on local panes (3) Handle `InputEvent::ExecuteLocalAcpQuery` → `try_submit_local_acp_query` (4) Pass harness model into `Input::new` (5) `extra_input_chrome_height()` for menu layout (6) Cloud spawn guards on executor events | `TerminalView` owns model graph and input event dispatch — **only place** per-pane models are created | No dependency injection for view models; events bubble View → Input → View |
| **`app/src/terminal/input.rs`** | (1) `ExecuteLocalAcpQuery` event (2) `LocalAcpPickerState` + harness/model selector views (3) `submit_ai_query` routes to ACP (4) Enter handler bypasses cloud `spawn_agent` (5) Block all Oz `send_user_query_*` paths (6) Handoff/follow-up guards | **Single front door** for Enter key and AI submit | `Input` owns keyboard handling; cannot intercept submit from outside without this file |
| **`app/src/terminal/input/agent.rs`** | Harness + model row above editor; `shows_local_acp_harness_row()` visibility | Agent input column layout lives here | No separate "agent chrome" plugin point |
| **`app/src/terminal/input/inline_menu/positioning.rs`** | +36px when harness row present | Completions/tooltips anchor to input box geometry | Positioner reads element sizes at layout time — must know about new row |
| **`app/src/terminal/view/ambient_agent/harness_selector.rs`** | `HarnessSelectionBackend` instead of hard-coded ambient model; `harnesses_for_selector()`; local install tooltips | Reuse existing picker UI component | Building duplicate picker would touch more files |
| **`app/src/terminal/view/ambient_agent/model_selector.rs`** | Optional `LocalAcpHarnessModel`; dual subscribe/set paths | Reuse existing model picker UI | Same |
| **`app/src/terminal/view/ambient_agent/mod.rs`** | Export `harness_selection` | Module visibility | — |

#### Slash commands

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `app/src/terminal/input/slash_commands/mod.rs` | `/harness` without cloud V2; block `/cloud-agent` handler | Slash execution dispatch lives here | — |
| `app/src/terminal/input/slash_commands/data_source/mod.rs` | Inject ACP per-agent commands; hide `/compact` when ACP active | Command menu data source | — |
| `app/src/search/slash_command_menu/static_commands/commands.rs` | `/harness` available in local agent view; **remove** `/cloud-agent`, `/move-to-cloud` from menu | Static command registry | v2: delete cloud commands entirely, not flag-gate |

#### Cloud agent removal (required for replacement product)

| File | Change | Why necessary | No alternative because |
|------|--------|---------------|------------------------|
| `app/src/terminal/view/ambient_agent/model.rs` | Guard `start_spawn_stream` — **v2: delete spawn code or entire cloud model** | Last resort if any cloud pane still exists | Network call originates here |
| `app/src/terminal/view/ambient_agent/view_impl.rs` | Block `enter_cloud_agent_view` — **v2: delete function body / cloud entry** | All cloud pane creation funnels here | — |
| `app/src/pane_group/pane/terminal_pane.rs` | Block Oz/Remote `StartAgentConversation` — **v2: remove remote launch** | Pane group dispatches child agent creation | — |

---

### C. REQUIRED modifications — Rust exhaustiveness only (compile, not product logic)

Adding `Harness::Cursor` and `Harness::Devin` **forces** updates anywhere the enum is matched exhaustively. These touches do **not** add Local ACP behavior — they prevent compile failures.

| File | Change | Could v2 avoid? |
|------|--------|-----------------|
| `app/src/pane_group/pane/local_harness_launch.rs` | Add Cursor/Devin to match arms; `unreachable!` in legacy CLI path | Only if Cursor/Devin are not in `Harness` enum (then you can't reference them in ACP registry either) |
| `app/src/terminal/cli_agent.rs` | Map Cursor → `CursorCli` | Same — enum extension ripples |
| `app/src/terminal/view/shared_session/cloud_conversation_continuation.rs` | Map Cursor/Devin → Unknown | **v2: delete entire file's cloud continuation** if no shared cloud sessions |
| `app/src/pane_group/pane/terminal_pane.rs` | Remote launch harness mapping | Same |
| `app/src/terminal/view/ambient_agent/view_impl.rs` | `harness_cli_installed` match arms | Same |

**v2 note:** In a cloud-free fork, delete cloud-only modules instead of patching matches — fewer files, cleaner tree.

---

### D. NOT REQUIRED — do not repeat in v2

These changes were **avoidable** and added regression surface. Documented so the restart does not repeat them.

| File | What changed | Why it was unnecessary | v2 action |
|------|--------------|------------------------|-----------|
| `app/src/terminal/profile_model_selector.rs` | Switched to `models_for_picker()` | Cloud/profile execution UI — **not** the local agent input harness row | **Revert / skip** — local harness UI is in `input/agent.rs` |
| `app/src/features.rs` | Register `FeatureFlag::LocalAcp` | Rollout mechanism — **not needed for replacement product** | **Remove** — always-on build |
| `crates/warp_features/src/lib.rs` | `FeatureFlag::LocalAcp` enum + flag lists | Same | **Remove flag**; optionally remove cloud flags too |

---

### E. First-pass flag / guard layer (replace in v2, do not copy literally)

The first pass added runtime checks everywhere:

```rust
// DELETE in v2 — not a flag product
if FeatureFlag::LocalAcp.is_enabled() { ... }
if cloud_agent_disabled() { return; }  // was: FeatureFlag::LocalAcp.is_enabled()
```

**v2 approach:** Instead of guarding cloud paths, **remove or stub cloud paths** in a fork:

- Do not create `AmbientAgentViewModel` at all (not even for `is_cloud_mode`)
- Remove `enter_cloud_agent_view`, cloud slash commands, handoff code paths
- `submit_ai_query` ends with ACP route or error — no Oz fallback to guard

This is cleaner than 15 `cloud_agent_disabled()` guards.

---

## Architecture (what worked — keep in v2)

```
Regular terminal tab
  → No agent models. Enter → shell.

Agent pane (Ctrl+Space)
  → LocalAcpHarnessModel (harness + model state)
  → HarnessSelector (HarnessSelectionBackend::LocalAcp)
  → ModelSelector (subscribes to LocalAcpHarnessModel)

Enter / submit
  → active_local_acp_harness(harness)
  → ExecuteLocalAcpQuery
  → try_submit_local_acp_query
  → LocalAcpSubmitModel
  → acp_client::AcpSession (subprocess)
  → LocalAcpBlocklistStream → blocklist UI
```

**Never again:**
- `AmbientAgentViewModel` on local panes
- `Status::Composing` to show harness picker
- Oz `send_user_query_*` as fallback

---

## Regressions from first pass (learn before v2)

| Symptom | Root cause | Fix |
|---------|------------|-----|
| `ls` broken on regular tabs | `AmbientAgentViewModel` on all panes; Enter → `spawn_agent` | Model creation only in agent pane context; never on plain tabs |
| Tooltips/completions wrong | Harness row added without layout offset | Change `positioning.rs` in **same commit** as harness row |
| "Install CLI" false negatives | GUI minimal PATH on macOS | `path_search.rs` + shell PATH in `harness_availability` |
| Docker environment modal on submit | Submit hit cloud spawn | Route to `ExecuteLocalAcpQuery` first; v2 remove cloud spawn entirely |
| Codex "disabled" incorrectly | Wrong flag (`LocalClaudeCodexChildHarnesses`) | Codex ACP uses `codex-acp` binary via registry |

---

## Cloud agent — complete removal checklist (v2)

Delete or stub — do not guard with flags:

| Surface | File(s) | v2 action |
|---------|---------|-----------|
| Ambient spawn API | `ambient_agent/model.rs` | Remove `start_spawn_stream` / cloud stream |
| Cloud pane entry | `view_impl.rs` `enter_cloud_agent_view` | Delete |
| Oz server queries | `input.rs` `submit_ai_query`, `submit_user_query_now` | ACP-only route |
| Cloud handoff | `input.rs` `maybe_launch_cloud_handoff_request` | Delete |
| Cloud follow-up | `SubmitCloudFollowup` handlers | Delete |
| `/cloud-agent`, `/move-to-cloud` | `commands.rs`, `slash_commands/mod.rs` | Delete from registry |
| Remote child agents | `terminal_pane.rs` `launch_remote_child` | Delete or stub |
| Zero-state cloud links | `zero_state_block.rs` (2 files) | Remove UI copy |
| **Audit gaps** | `agent_view.rs` auto-submit, onboarding Oz chips in `view.rs` | Route through ACP or remove |

---

## Supported agents

| Harness | Binary | Notes |
|---------|--------|-------|
| Claude | `claude-agent-acp` | Not plain `claude` when using ACP |
| Codex | `codex-acp` | Not plain `codex` |
| Gemini | `gemini --acp` | |
| Cursor | `cursor-acp` | Adapter from `raphaelluethy/cursor-acp` |
| Devin | `devin acp` | |

Oz is **not** part of the replacement product.

---

## v2 restart order

```
1. crates/acp_client + app/src/ai/acp/*     (all logic, tests pass)
2. Harness enum + harness_display            (compile ripple — batch update matches)
3. harness_availability + local_harness_setup (catalog without server)
4. driver/harness/mod.rs + blocklist hooks   (execution + streaming)
5. lib.rs singleton registration
6. view.rs — models + ExecuteLocalAcpQuery only
7. input.rs — submit routing only            (test submit before UI)
8. input/agent.rs + positioning.rs           (harness row + layout together)
9. harness_selector + model_selector + harness_selection
10. Remove cloud agent code paths             (not guards)
11. Slash commands + permissions + MCP polish
```

**Test after step 7** before adding UI — proves submit works without picker regressions.

---

## Testing matrix

| # | Scenario | Expected |
|---|----------|----------|
| 1 | Regular tab: shell commands | Normal — no agent models, no harness row |
| 2 | Agent pane: submit | ACP subprocess, blocklist streams |
| 3 | Agent pane: harness/model pickers | Visible only in agent view |
| 4 | Completions position | Above harness row |
| 5 | No cloud/Oz anywhere | Grep: no successful `send_user_query_in_new_conversation` from agent submit |
| 6 | No flag to enable | Feature works in default build with zero settings |

```bash
./script/bootstrap
cargo check --bin warp-oss --features gui
./script/run --dont-open
```

---

## Summary

| Category | Count | Action in v2 |
|----------|-------|--------------|
| New files (logic) | ~25 | Copy / rewrite cleanly |
| Required wiring files | ~18 | Touch surgically — documented above |
| Exhaustiveness-only files | ~5 | Batch with enum change, or delete if cloud removed |
| Avoid / revert | 3 | `profile_model_selector`, flag registration |
| Flag/guard layer | ~15 call sites | **Replace with deletion**, not guards |

**The replacement product is Local ACP only.** No runtime flag. Cloud agent code should be removed, not disabled. Existing files are touched only because Warp's agent submit, view model graph, harness UI, and driver factory are hard-coded — there is no plugin API. The v2 goal is the same wiring in **fewer files**, with **all logic in `app/src/ai/acp/`**, and **zero cloud agent paths** remaining.
