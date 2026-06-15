# Local ACP — Minimal Integration Guide

> **New context / full scratch:** [START-FROM-SCRATCH.md](./START-FROM-SCRATCH.md)  
> **Canonical handoff doc:** [RESTART-GUIDE.md](./RESTART-GUIDE.md)

This file is a condensed file-tier index. The target is **Local ACP only** — not a flag-gated add-on.

Related: [TECH.md](./TECH.md) (architecture), [TASKS.md](./TASKS.md) (checklist).

---

## Design principles

1. **New code lives in new files** — transport, registry, submit, streaming, permissions, telemetry.
2. **Existing files get thin hooks only** — routing guards, model creation, UI backend selection, menu height.
3. **Never piggyback on cloud ambient-agent lifecycle** for local panes — no `AmbientAgentViewModel`, no `Status::Composing`, no `spawn_agent` on regular terminal tabs.
4. **Cloud agent removed, not gated** — In v2, delete cloud spawn/Oz paths; do not rely on `FeatureFlag::LocalAcp` or `cloud_agent_disabled()` guards.
5. **Compile-time only** — `#[cfg(feature = "local_acp")]` in default build is fine; no runtime toggle.

---

## File tiers

### Tier 0 — Add-only (safe to copy wholesale)

These files did not exist before Local ACP. Copy the entire directory/tree.

| Path | Role |
|------|------|
| `acpx` + `agent-client-protocol` | ACP transport: subprocess, typed protocol requests, session updates |
| `app/src/ai/acp/mod.rs` | Routing helpers: `should_use_acp_harness`, `active_local_acp_harness`, `should_submit_via_local_acp`, row height constant |
| `app/src/ai/acp/harness_picker.rs` | **`LocalAcpHarnessModel`** — per-pane harness + model only (no cloud spawn state) |
| `app/src/ai/acp/registry.rs` | Spawn specs per agent (Claude, Codex, Gemini, Cursor, Devin) |
| `app/src/ai/acp/path_search.rs` | Binary resolution with augmented PATH |
| `app/src/ai/acp/models.rs` | ACPX-backed model discovery from agent `config_options` + fallback defaults |
| `app/src/ai/acp/openusage.rs` | OpenUsage local API integration for provider usage context |
| `app/src/ai/acp/submit.rs` | `try_submit_local_acp_query`, permission response |
| `app/src/ai/acp/submit_model.rs` | **`LocalAcpSubmitModel`** — spawns session, drives blocklist |
| `app/src/ai/acp/session_store.rs` | Resume `sessionId` |
| `app/src/ai/acp/slash_commands.rs` | Per-agent ACP slash forwarding |
| `app/src/ai/acp/telemetry.rs` | Lifecycle telemetry |
| `app/src/ai/agent_sdk/driver/harness/acp.rs` | **`AcpHarness`** driver |
| `app/src/terminal/view/ambient_agent/harness_selection.rs` | **`HarnessSelectionBackend::Cloud \| LocalAcp`** |

Also add unit tests alongside: `registry_tests.rs`, `models_tests.rs`, `mcp_tests.rs`, etc.

### Tier 1 — Build / flag / enum (5 files, small diffs)

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | `acpx` + `agent-client-protocol` dependencies |
| `app/Cargo.toml` | `local_acp = ["agent_harness", "dep:acpx", "dep:agent-client-protocol"]` feature |
| `crates/warp_features/src/lib.rs` | `FeatureFlag::LocalAcp` in `LOCAL_FLAGS` + `DOGFOOD_FLAGS` |
| `app/src/features.rs` | Register flag |
| `crates/warp_cli/src/agent.rs` | `Harness::Cursor`, `Harness::Devin` |

### Tier 2 — Harness catalog (3 files, localized)

| File | Change |
|------|--------|
| `app/src/ai/harness_availability.rs` | `local_acp_harnesses()`, `harnesses_for_selector()` local branch, PATH refresh, **`cloud_agent_disabled()`**, fix `has_any_enabled_harness()` for local catalog |
| `app/src/ai/local_harness_setup.rs` | Detect ACP binaries; skip Codex product-disable when Local ACP on |
| `app/src/ai/harness_display.rs` | Cursor/Devin display metadata |

### Tier 3 — Driver + blocklist hooks (5 files)

| File | Change |
|------|--------|
| `app/src/ai/mod.rs` | `pub(crate) mod acp` behind `feature = "local_acp"` |
| `app/src/lib.rs` | Register `LocalAcpSubmitModel`, `LocalAcpSessionStore`, `LocalAcpPermissionModel` singletons |
| `app/src/ai/agent_sdk/driver/harness/mod.rs` | Route to `AcpHarness` when `should_use_acp_harness` |
| `app/src/ai/blocklist/mod.rs` | Re-export `LocalAcpBlocklistStream` |
| `app/src/ai/blocklist/controller/response_stream.rs` | `ResponseStreamId::new_local()` |
| `app/src/ai/blocklist/prompt/prompt_alert.rs` | Quota bypass via `bypasses_warp_ai_quota` |

### Tier 4 — Terminal integration (6 files, the critical hooks)

These are the **only** terminal files that must change for core behavior:

| File | Hook | What to add |
|------|------|-------------|
| **`app/src/terminal/view.rs`** | Model creation | `AmbientAgentViewModel` **only** when `is_cloud_mode`; `LocalAcpHarnessModel` when `!is_cloud_mode && local_acp_harness_selector_enabled()`. Handle `ExecuteLocalAcpQuery`. `extra_input_chrome_height()` for harness row. |
| **`app/src/terminal/input.rs`** | Submit routing | `LocalAcpPickerState`; `selected_harness_for_submit()`; emit `ExecuteLocalAcpQuery` in `submit_ai_query`; block Oz when `cloud_agent_disabled()`; bypass cloud `spawn_agent` Enter path for local ACP. |
| **`app/src/terminal/input/agent.rs`** | Harness row UI | Show row via `shows_local_acp_harness_row()` (not cloud composing); model selector beside harness chip. |
| **`app/src/terminal/input/inline_menu/positioning.rs`** | Tooltip/completion layout | Add `LOCAL_ACP_HARNESS_ROW_HEIGHT_PX` to max height / waterfall inset. |
| **`app/src/terminal/view/ambient_agent/harness_selector.rs`** | Picker UI | Use `HarnessSelectionBackend` instead of hard-coded `AmbientAgentViewModel`. |
| **`app/src/terminal/view/ambient_agent/model_selector.rs`** | Model picker | Subscribe to `LocalAcpHarnessModel` events. |

### Tier 5 — Cloud isolation guards (6 files, one-liners at choke points)

When `FeatureFlag::LocalAcp` is on, cloud agent must not run. Central helper:

```rust
// app/src/ai/harness_availability.rs
pub fn cloud_agent_disabled() -> bool {
    FeatureFlag::LocalAcp.is_enabled()
}
```

| File | Guard location |
|------|----------------|
| `app/src/terminal/view/ambient_agent/model.rs` | Top of `start_spawn_stream` — blocks all ambient cloud API spawns |
| `app/src/terminal/view/ambient_agent/view_impl.rs` | Top of `enter_cloud_agent_view` |
| `app/src/terminal/input.rs` | `submit_ai_query` Oz fallback; `submit_user_query_now`; `RunAgentQuery`; `maybe_launch_cloud_handoff_request`; `SubmitCloudFollowup` emit |
| `app/src/terminal/view.rs` | `SubmitCloudFollowup` handler; `StartAgentConversation` from executor |
| `app/src/pane_group/pane/terminal_pane.rs` | `dispatch_start_agent_conversation` — block `Remote` and Oz-local child |
| `app/src/terminal/input/slash_commands/mod.rs` | `/cloud-agent` handler no-op |
| `app/src/search/slash_command_menu/static_commands/commands.rs` | Hide `/cloud-agent` and `/move-to-cloud` from menu when Local ACP on |

### Tier 6 — Slash commands + footer (optional polish, 4 files)

| File | Change |
|------|--------|
| `app/src/terminal/input/slash_commands/data_source/mod.rs` | `local_acp_harness_active`, forward ACP slash commands |
| `app/src/terminal/input/slash_commands/mod.rs` | Enable `/harness` outside cloud V2 when Local ACP on |
| `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` | Model selector for local ACP harnesses |
| `app/src/ai/blocklist/inline_action/orchestration_controls.rs` | `local_acp_supports_harness_models` |

### Tier 7 — Do **not** touch (avoid regressions)

| Area | Why leave alone |
|------|-----------------|
| `app/src/terminal/view/ambient_agent/model.rs` lifecycle (except `start_spawn_stream` guard) | Cloud-only; not created on local panes |
| `local_harness_launch.rs` | Legacy CLI hidden-pane path; ACP replaces it |
| `profile_model_selector.rs` | Unrelated to ACP harness row |
| Regular shell input / PTY / blocklist rendering | Must stay untouched |
| `AmbientAgentViewModel` init defaults | Revert any Local-ACP-specific harness init — use `LocalAcpHarnessModel` instead |

---

## Integration flow (reference)

```
Agent pane Enter / submit_ai_query
  │
  ├─ should_submit_via_local_acp?
  │     └─ yes → Event::ExecuteLocalAcpQuery
  │              → try_submit_local_acp_query
  │              → LocalAcpSubmitModel
  │              → acpx::Connection
  │              → typed ACP session/update handling
  │
  ├─ cloud_agent_disabled?
  │     └─ yes → return (no Oz, no spawn)
  │
  └─ (flag off) existing Oz / cloud paths unchanged
```

### Model split by pane type

| Pane | `is_cloud_mode` | Harness state | Submit path |
|------|-----------------|---------------|-------------|
| Regular terminal tab | `false` | none | Shell only — **no models created** |
| Local agent pane | `false` | `LocalAcpHarnessModel` | Local ACP |
| Cloud agent pane | `true` | `AmbientAgentViewModel` | Cloud spawn (blocked when Local ACP flag on) |

### Harness row visibility

Show the harness chip row when **all** of:

- `FeatureFlag::LocalAcp` + `FeatureFlag::AgentHarness`
- `HarnessAvailabilityModel::should_show_harness_selector()`
- Agent view active (`agent_view_controller.is_active()`)
- `local_acp_picker_state` present

Do **not** use `Status::Composing` or `is_configuring_ambient_agent()` for local panes.

---

## Step-by-step redo checklist

Use this order to minimize merge conflicts:

### Phase A — Foundation (no UI changes)

- [ ] Add `acpx` + `agent-client-protocol` workspace deps
- [ ] Add `FeatureFlag::LocalAcp` + `app/Cargo.toml` feature
- [ ] Add `app/src/ai/acp/*` module tree
- [ ] Add `AcpHarness` + wire in `driver/harness/mod.rs`
- [ ] Extend `Harness` enum (Cursor, Devin) + `harness_display`
- [ ] Extend `harness_availability` + `local_harness_setup` + `cloud_agent_disabled()`
- [ ] Register singletons in `app/src/lib.rs`

### Phase B — Submit path (agent pane only)

- [ ] `local_acp_stream.rs` + response stream id
- [ ] `submit.rs` / `submit_model.rs` / permissions / session store
- [ ] `view.rs`: create `LocalAcpHarnessModel`, handle `ExecuteLocalAcpQuery`
- [ ] `input.rs`: `ExecuteLocalAcpQuery` event, routing in `submit_ai_query`, Enter bypass

### Phase C — Harness picker UI (decouple from cloud)

- [ ] `harness_picker.rs` (`LocalAcpHarnessModel`)
- [ ] `harness_selection.rs` backend enum
- [ ] Update `harness_selector.rs` + `model_selector.rs`
- [ ] `input.rs`: `LocalAcpPickerState`, `shows_local_acp_harness_row()`
- [ ] `input/agent.rs`: harness row visibility
- [ ] `inline_menu/positioning.rs`: row height offset

### Phase D — Cloud isolation

- [ ] `cloud_agent_disabled()` + all Tier 5 guards
- [ ] Hide cloud slash commands in `commands.rs`
- [ ] Verify: regular tab `ls` works; agent pane submit hits ACP not Oz

### Phase E — Polish

- [ ] Slash command forwarding, MCP, telemetry, permissions UI
- [ ] PATH augmentation (`path_search.rs` + shell PATH via `LocalShellState`)

---

## Cloud agent choke points (audit list)

When auditing that cloud agent cannot fire with Local ACP on, grep for these symbols and confirm each either (a) is unreachable without `AmbientAgentViewModel` on local panes, or (b) has a `cloud_agent_disabled()` guard:

| Symbol / path | Guarded? |
|---------------|----------|
| `start_spawn_stream` / `spawn_agent` | Yes — model.rs |
| `enter_cloud_agent_view` | Yes — view_impl.rs |
| `submit_ai_query` → `send_user_query_*` | Yes — input.rs |
| `submit_user_query_now` | Yes — input.rs |
| `RunAgentQuery` prompt chip | Yes — input.rs (routes to ACP or drops) |
| `maybe_launch_cloud_handoff_request` | Yes — input.rs |
| `SubmitCloudFollowup` | Yes — input.rs + view.rs |
| `EnterCloudAgentView` action/event | Yes — view_impl.rs |
| `/cloud-agent` slash | Yes — mod.rs + commands.rs |
| `StartAgentConversation` (Oz/Remote) | Yes — terminal_pane.rs + view.rs |
| Zero-state “start cloud agent” links | UI still visible; action no-ops via `enter_cloud_agent_view` guard |

Optional follow-up (not required for safety): hide zero-state cloud links when `cloud_agent_disabled()` in `zero_state_block.rs` files.

---

## PATH / install detection note

GUI apps on macOS have a minimal PATH. Local ACP binary detection must:

1. Augment PATH with common install locations (`path_search.rs`)
2. Refresh from interactive shell PATH on startup (`harness_availability::refresh_local_acp_search_path`)
3. Use `path_search::resolve_command()` in registry + `local_harness_setup`

Required binaries: `claude-agent-acp`, `codex-acp`, `gemini --acp`, `cursor-acp` (Cursor), `devin acp`.

---

## Testing matrix

| Scenario | Expected |
|----------|----------|
| Regular terminal tab, Local ACP flag on | Shell works; no harness row; no ambient model |
| Agent pane (Ctrl+Space), harness selected | Harness + model row; Enter → local ACP subprocess |
| Agent pane, no harness / Oz selected | Submit blocked (no server Oz) |
| Completions / tooltips in agent pane | Positioned above harness row (+36px) |
| `/cloud-agent`, `/move-to-cloud` | Hidden or no-op |
| Local ACP flag **off** | Identical to upstream — cloud + Oz unchanged |

Run: `./script/bootstrap && ./script/run --dont-open`  
Compile: `cargo check --bin warp-oss --features gui`

---

## Summary: minimum upstream files touched

**15 existing files** for core integration (Tiers 1–4), **6 more** for cloud isolation (Tier 5), **4 optional** for slash/footer polish (Tier 6). Everything else is new files under `crates/acp_client/` and `app/src/ai/acp/`.

The highest-risk mistakes to avoid when re-porting:

1. Creating `AmbientAgentViewModel` on all local panes → breaks shell Enter and tooltips.
2. Leaving panes in `Status::Composing` to show the picker → hijacks Enter to `spawn_agent`.
3. Forgetting harness row height in menu positioning → overlapping completions.
4. Missing `cloud_agent_disabled()` on Oz fallback → environment modal / server queries still fire.
