# Harness-Specific Model Selection — Tech Spec

Linear: [QUALITY-643](https://linear.app/warpdotdev/issue/QUALITY-643)

Companion product spec: `specs/QUALITY-643/PRODUCT.md`

## Context

The orchestration config UI (plan card and run_agents confirmation card) lets users pick a harness and model for child agents. Both cards share picker logic in `orchestration_controls.rs`. Two problems exist today:

1. **Harness picker** is hardcoded to `[Oz, Claude, Codex]` — it doesn't read from the server's `availableHarnesses` list, doesn't include Gemini, and doesn't respect admin enabled/disabled state.
2. **Model picker** always shows Warp's internal LLM catalog filtered by provider (Anthropic for Claude, OpenAI for Codex). Those IDs (e.g. `claude-4-6-opus-high`) are not recognized by third-party harness CLIs. The server maintains separate harness-specific model catalogs that the desktop client already fetches and caches in `HarnessAvailabilityModel`, but the orchestration UI doesn't use them.

Additionally, model_id is not delivered to local child harness processes: Claude Code doesn't receive `ANTHROPIC_MODEL`. Codex model delivery uses `~/.codex/config.toml`, which is only safe in cloud/remote environments where the filesystem is isolated — local children must not touch it (see `local_harness_launch.rs:143` comment).

### Relevant files

**Shared picker logic (model/harness dropdowns)**
- `app/src/ai/blocklist/inline_action/orchestration_controls.rs` — `populate_harness_picker()` (line 380), `populate_model_picker_for_harness()` (line 314), `is_model_in_filtered_choices()` (line 353), `first_filtered_model_id()` (line 368), `sync_picker_selections()` (line 508), `matches_harness_filter()` (line 299)

**UI card views that consume the shared pickers**
- `app/src/ai/document/orchestration_config_block.rs` — plan card; subscribes to `LLMPreferencesEvent` for model refresh
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs` — confirmation card; `HarnessChanged` handler (line 752) resets model on harness change

**Harness availability and model data (already fetched and cached)**
- `app/src/ai/harness_availability.rs` — `HarnessAvailabilityModel` singleton with:
  - `available_harnesses()` → `&[HarnessAvailability]` (harness, display_name, enabled, available_models)
  - `models_for(harness)` → `Option<&[HarnessModelInfo]>` (id, display_name)
  - `is_harness_enabled(harness)` → `bool`
  - Emits `HarnessAvailabilityEvent::Changed`

**Harness display metadata**
- `app/src/ai/harness_display.rs` — `display_name()`, `icon_for()`, `brand_color()` per `Harness` variant

**Model ID delivery to harness processes**
- `app/src/ai/agent_sdk/driver/harness/mod.rs` — `harness_model_env_vars()` (line 373): sets `ANTHROPIC_MODEL` for Claude, no-op for Codex
- `app/src/pane_group/pane/local_harness_launch.rs` — `prepare_local_harness_child_launch()` (line 77): builds child env_vars but does not include model_id
- `app/src/pane_group/pane/terminal_pane.rs` — `launch_local_harness_child()` (line 1302): passes `model_id` to `apply_child_model_id_override` which only sets Oz LLM preference
- `app/src/ai/agent_sdk/driver/harness/codex.rs` — `prepare_codex_config_toml()` (line 573): writes `~/.codex/config.toml` but does not write a `model` key; top-level key name is `"model"` (confirmed by test at `codex_tests.rs:201`)

## Proposed changes

### 1. Populate harness picker from `HarnessAvailabilityModel`

**File**: `orchestration_controls.rs` — `populate_harness_picker()`

Replace the hardcoded `[Harness::Oz, Harness::Claude, Harness::Codex]` iteration (line 392) with a read from `HarnessAvailabilityModel::as_ref(ctx).available_harnesses()`.

For each `HarnessAvailability` entry:
- Use `harness_display::icon_for()` and `harness_display::brand_color()` for icons (these already cover all variants including Gemini).
- Use `harness_display::display_name()` for the label (the server's `display_name` field could also be used, but the client-side names already match and are guaranteed non-empty before the server responds).
- If `!entry.enabled`: render as disabled (non-selectable, greyed text). The `MenuItemFields` API supports `.with_disabled(true)` or equivalent — check the existing `MenuItem` disabled patterns in the codebase.
- Sort enabled entries before disabled entries.

Also subscribe both card views to `HarnessAvailabilityEvent::Changed` to repopulate the harness picker when the server list updates.

Addresses PRODUCT.md behaviors 1–5.

### 2. Switch model picker to harness-specific models with "Default model" entry

**File**: `orchestration_controls.rs` — `populate_model_picker_for_harness()`

Add an `is_local: bool` parameter (or pass the current `RunAgentsExecutionMode`) so the picker can be execution-mode-aware. Replace the current provider-filtered `LLMPreferences` logic with harness-aware branching:

```
let harness = Harness::parse_orchestration_harness(harness_type);
match harness {
    Some(Harness::Oz) | None => {
        // Current behavior: LLMPreferences filtered by provider
    }
    Some(Harness::Codex) if is_local => {
        // Local Codex: only "Default model" entry (no model delivery possible)
    }
    Some(harness) => {
        // 1. Always add a "Default model" entry first (value: empty string)
        // 2. Read HarnessAvailabilityModel::as_ref(ctx).models_for(harness)
        // 3. If Some(models): append each HarnessModelInfo as a menu item
        //    with display_name as label and id as the model_changed action value
        // 4. If None: only "Default model" is shown (loading/empty state)
    }
}
```

The "Default model" entry should use label `"Default model"` and emit `A::model_changed(String::new())` (empty string). This matches the web UI which adds this entry with value `""`.

When execution mode toggles between Local and Cloud, the `HarnessChanged` / `ExecutionModeToggled` handlers must repopulate the model picker since Codex's available models depend on the mode.

Apply the same Oz-vs-non-Oz branching to:
- `is_model_in_filtered_choices()` — for non-Oz, check model_id against `HarnessAvailabilityModel::models_for()` OR accept empty string (the "Default model" entry). For local Codex, only empty string is valid.
- `first_filtered_model_id()` — for non-Oz, return `Some(String::new())` (the "Default model" entry) as the default.
- `sync_picker_selections()` — for non-Oz, find display_name from `HarnessAvailabilityModel::models_for()` instead of `LLMPreferences`. Map empty model_id to the "Default model" label.

Addresses behaviors 6–10, 11–14, 15–16.

### 3. Subscribe both card views to `HarnessAvailabilityEvent::Changed`

**Files**: `orchestration_config_block.rs`, `run_agents_card_view.rs`

Both views already subscribe to `LLMPreferencesEvent::UpdatedAvailableLLMs`. Add an analogous subscription to `HarnessAvailabilityModel`:
- Repopulate the **harness picker** when the harness list changes (behaviors 1–5).
- Repopulate the **model picker** when harness models arrive, but only when the current harness is non-Oz (behavior 15).

Addresses behaviors 1–5, 15.

### 4. Propagate model_id to local child harness processes (Claude Code only)

**File**: `local_harness_launch.rs`

Add `model_id: Option<String>` parameter to `prepare_local_harness_child_launch()`. After building `env_vars` from `task_env_vars()`:
- For Claude: merge `harness_model_env_vars(harness, model_id.as_deref())` into env_vars. This sets `ANTHROPIC_MODEL` when model_id is non-empty.
- For Codex: no model delivery for local children. The UI ensures model_id is empty for local Codex (behavior 8), so no action is needed here. The existing code already skips `prepare_codex_environment_config()` for local children (`local_harness_launch.rs:143`) and this spec preserves that constraint.

**File**: `terminal_pane.rs`

Update the call to `prepare_local_harness_child_launch()` in `launch_local_harness_child()` (line 1325) to pass the `model_id` value.

Addresses behaviors 17, 20.

### 5. Write Codex model to config.toml (cloud/remote path only)

**File**: `codex.rs`

Add `model_id: Option<&str>` parameter to `prepare_codex_environment_config()` and `prepare_codex_config_toml()`.

In `prepare_codex_config_toml()`, after `set_codex_openai_base_url()`:
- If `model_id` is `Some(id)` where `id` is non-empty and not `"default"`: `doc["model"] = toml_edit::value(id)`.
- Otherwise: `doc.remove("model")` to clear any pre-existing key.

Add constant `CODEX_MODEL_KEY: &str = "model"`.

The existing test at `codex_tests.rs:201` verifies a pre-existing `model` key is preserved — update it to verify the new write/remove behavior.

Update the caller of `prepare_codex_environment_config()` in `codex.rs` `build_runner()` to pass `model_id`. No changes needed in `local_harness_launch.rs` — local Codex children don't call this function and don't receive model overrides.

Addresses behavior 18.

### 6. No changes needed for remote launch path

The remote launch path already passes `model_id` to the server via `StartAgentExecutionMode::Remote { model_id }` in `run_agents_to_start_agent_mode()`. With the UI now storing harness-native IDs (or empty for "Default model"), the server receives the correct value without translation.

Addresses behaviors 17 (remote), 18 (remote).

## Testing and validation

### Unit tests

**orchestration_controls tests** — new tests:
- Harness picker populated from `HarnessAvailabilityModel`; disabled harnesses shown but not selectable. (Behaviors 1–4)
- `populate_model_picker_for_harness` with harness="claude": "Default model" entry at top, then harness-specific models from `HarnessAvailabilityModel`. (Behavior 7)
- `populate_model_picker_for_harness` with harness="codex", cloud mode: "Default model" entry at top, then Codex models. (Behavior 8)
- `populate_model_picker_for_harness` with harness="codex", local mode: only "Default model" entry. (Behavior 8)
- `populate_model_picker_for_harness` with harness="oz": Warp LLM catalog (existing behavior). (Behavior 6)
- `is_model_in_filtered_choices` returns false for Warp IDs when harness is non-Oz, true for empty string ("Default model"). (Behavior 12)
- `first_filtered_model_id` returns empty string for non-Oz harness. (Behavior 11)
- Harness change from Claude (model="opus") to Oz: model resets to first Warp LLM. (Behavior 12)

**local_harness_launch tests** — new/updated tests:
- `prepare_local_harness_child_launch` merges `ANTHROPIC_MODEL` into env_vars when harness is Claude and model_id is provided. (Behavior 17)
- `prepare_local_harness_child_launch` does NOT set `ANTHROPIC_MODEL` when model_id is None or empty. (Behavior 20)

**codex config.toml tests** — update existing tests in `codex_tests.rs`:
- `prepare_codex_config_toml` writes `model = "gpt-5.4"` when model_id is `Some("gpt-5.4")`. (Behavior 18)
- `prepare_codex_config_toml` removes existing `model` key when model_id is `Some("default")`. (Behavior 18)
- `prepare_codex_config_toml` removes existing `model` key when model_id is `None`. (Behavior 20)
- Pre-existing non-model keys (openai_base_url, projects, mcp_servers) are preserved in all cases.

**orchestration_config_tests** — existing tests must continue to pass. (Behavior 21)

### Presubmit

Run `cargo fmt`, `cargo clippy`, and `./script/presubmit` before PR.

### Manual validation

- Open orchestration config on a plan card → harness picker shows Oz, Claude Code, Codex (and Gemini if server returns it, possibly disabled).
- Select Claude Code → model picker shows "Default model" at top, then `best`, `opus`, `sonnet`, etc.
- Select Codex (Cloud mode) → model picker shows "Default model" at top, then `default`, `GPT-5.5`, `GPT-5.4`, etc.
- Select Codex (Local mode) → model picker shows only "Default model".
- Toggle Local → Cloud with Codex selected → model picker repopulates with full Codex catalog.
- Select Oz → model picker returns to Warp LLM catalog.
- Change harness from Claude (with "opus" selected) to Oz → model resets.
- Launch local agents with Claude Code + "opus" → verify `ANTHROPIC_MODEL=opus` in child env.
- Launch local agents with Claude Code + "Default model" → verify no `ANTHROPIC_MODEL` in child env.
- Launch cloud agents with Codex + "gpt-5.4" → verify `model = "gpt-5.4"` in `~/.codex/config.toml` inside the cloud env.
- Launch cloud agents with Codex + "default" → verify no `model` key in `~/.codex/config.toml`.

## Parallelization

Not recommended. The changes are tightly coupled — the harness picker (change 1) and model picker (change 2) share state in `orchestration_controls.rs`, the card view subscriptions (change 3) depend on both pickers, and the launch-side changes (4–5) depend on the correct model_id format flowing from the picker. Total scope is ~7 files with moderate changes each, well suited for sequential execution by a single agent.
