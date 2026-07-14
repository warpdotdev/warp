# Spec: Collapse per-reasoning-level entries in the inline model picker (APP-4854)

Linear: [APP-4854](https://linear.app/warpdotdev/issue/APP-4854/warp-client-model-picker-collapse-per-reasoning-level-model-entries)
Target repo: `warpdotdev/warp`
Code references are pinned to commit `27af3ab750bdc9a7c65b707401a3db9cf8702bb1`.

## PRODUCT

### Summary
The Warp client has two model pickers. The **dropdown** picker (`ProfileModelSelector`) already collapses a model's reasoning variants into a single base-model entry and exposes the reasoning levels in a selectable right-side sidecar. The **inline, search-based** picker (opened via `/model`, `InlineModelSelectorView`) does not: it lists one row per reasoning variant (e.g. separate `gpt-5.6-terra (low)`, `gpt-5.6-terra (medium)`, `gpt-5.6-terra (high)`, `gpt-5.6-terra (xhigh)` rows) and its right panel is a read-only details view. This spec brings the inline picker to parity: one row per base model, with the reasoning level chosen from a selectable right-side sidecar.

### Key design choices
1. **Reuse the dropdown's proven collapse logic, don't reinvent it.** Group reasoning variants by `LLMInfo::base_model_name()` (auto models under a single `"auto"` key, non-reasoning models by `id`) exactly as `refresh_model_menu` does in the dropdown, factoring the grouping into a pure, unit-testable helper so both the data source and tests can call it.
2. **Keep the change scoped to the model-selector surface; do not add interactive-sidecar state to the shared `InlineMenuView`.** `InlineMenuView` backs 11 menu types and its details pane is intentionally passive. Introducing sidecar selection/focus state there would be high blast-radius. Instead, keep the collapsing in `ModelSelectorDataSource` and own the interactive reasoning sidecar's state in `InlineModelSelectorView` (mirroring how the dropdown owns its `model_spec_sidecar`), with model-selector-only key routing in `input.rs`. Other inline menus stay byte-for-byte unchanged.
3. **The selected reasoning level always resolves to a concrete variant `LLMId`.** The picker's accept contract (`AcceptModel { id }` → `InlineModelSelectorEvent::SelectedModel { id, .. }`) is unchanged; a collapsed row simply resolves to the currently-targeted variant's id. This preserves the existing selection, "set as default" (`cmd`/`ctrl+shift+enter`), and preference-update plumbing.
4. **No new feature flag.** This is parity with already-shipped dropdown behavior on the same model data, so it ships ungated. (If staged rollout is later desired, gate the inline collapse+sidecar behind a new `FeatureFlag` per the repo's feature-flag conventions — noted as an option, not a requirement.)

### Behavior (numbered, testable invariants)
1. **One row per base model (default / happy path).** With the inline `/model` picker open and no search text, a model family that has multiple reasoning variants appears as exactly one row labeled with its base name (e.g. a single `gpt-5.6-terra`), not one row per reasoning level.
2. **Reasoning sidecar on the collapsed row.** When a collapsed reasoning row is the active (selected/hovered) row, the right side shows a "Reasoning level" sidecar listing that family's available levels (e.g. low, medium, high, xhigh) in the server-provided order, with the currently-active variant visibly indicated (checkmark/selected styling).
3. **Selecting a level selects that variant.** Choosing a reasoning level — by mouse click or keyboard — sets the active model to that specific variant's `LLMId`. After selection the picker behaves exactly as selecting a model does today (emits `SelectedModel`, updates preferences, closes the menu).
4. **Keyboard navigation.** `up`/`down` move between model rows. From a collapsed reasoning row, `right` (and/or `tab`) moves focus into the reasoning sidecar; `up`/`down` then cycle reasoning levels; `enter` accepts the highlighted level; `left` (and `escape` once) returns focus to the model list without closing the menu.
5. **Accepting a collapsed row without entering the sidecar.** Pressing `enter` on a collapsed reasoning row (without stepping into the sidecar) selects the currently-targeted level — the active variant when that base model is already selected, otherwise a deterministic default (server default level for the family, else the first listed level).
6. **"Set as default" is preserved.** `cmd+enter` (macOS) / `ctrl+shift+enter` (other) on a reasoning selection selects the variant *and* saves it as the profile default, identical to today's `set_as_default` path.
7. **Fuzzy search parity.** Typing a base model name (e.g. `terra`) matches the single collapsed row rather than N per-level rows; the matched collapsed row still exposes its reasoning sidecar. Typing a substring that only matches the base name still surfaces the family.
8. **Non-reasoning and auto models are unaffected.** Models without reasoning levels still render one row each with the existing read-only details panel and no reasoning sidecar. `auto` models collapse consistently with the dropdown (a single `auto` entry). Custom model routers and custom-endpoint models render as they do today.
9. **Cloud/full-terminal-use tabs unaffected in structure.** The collapse + sidecar behavior applies identically in the "Base" tab and the "Full Terminal Use" tab; cloud-pane suppression of custom-endpoint models (`include_model_in_picker`) is unchanged.
10. **No regression to other inline menus.** Slash commands, prompts, skills, conversations, history, repos, plans, and profile inline menus render and behave exactly as before (their details panes and key handling are untouched).

## TECH

### Context (how it works today)
- Inline picker data source: `app/src/terminal/input/models/data_source.rs @ 27af3ab`.
  - `ModelSelectorDataSource::run_query` (`data_source.rs:212`) maps each `LLMInfo` choice to one `ModelSearchItem` — no grouping — for both the empty-query path (`data_source.rs:254`) and the fuzzy-match path (`data_source.rs:261`), which matches on `llm.display_name` (`data_source.rs:265`), i.e. the per-variant name including the reasoning suffix.
  - `ModelSearchItem` (`data_source.rs:288`) already carries `reasoning_level: Option<String>` (`data_source.rs:308`, set from `llm.reasoning_level()` at `data_source.rs:357`) and `spec`.
  - `ModelSearchItem::render_details` (`data_source.rs:524`) renders a **read-only** right panel; when `reasoning_level.is_some()` it uses the `REASONING_LEVEL_TITLE`/`REASONING_LEVEL_DESCRIPTION` header (`data_source.rs:555`) but shows spec bars, not a selectable list.
  - Accept contract: `SearchItem::accept_result` returns `AcceptModel { id }` (`data_source.rs:722`).
- Inline picker view: `app/src/terminal/input/models/view.rs @ 27af3ab`.
  - `InlineModelSelectorView` (`view.rs:98`) wraps `InlineMenuView<AcceptModel, InlineModelSelectorTab>` and a `SearchMixer`. It subscribes to `InlineMenuEvent::AcceptedItem` and emits `InlineModelSelectorEvent::SelectedModel { id, selected_tab, set_as_default }` (`view.rs:238`).
  - It exposes `select_up`/`select_down`/`accept_selected_item` (`view.rs:519`), and holds the `ModelSelectorDataSource` handle (`view.rs:117`).
- Shared inline menu framework: `app/src/terminal/input/inline_menu/view.rs @ 27af3ab`.
  - The right panel is produced by `SearchItem::render_details` and driven by `details_display_idx()` (hover or selection) at `view.rs:968`; the split layout is built in `render` at `view.rs:1022`. It is passive — there is no per-detail selection/focus state. `InlineMenuAction::details_render_config` (`view.rs:287`) gates whether/how wide the details pane renders. This framework is shared by all `InlineMenuType`s (`inline_menu/mod.rs:38`).
- Keyboard routing (model-selector-specific) lives in `app/src/terminal/input.rs @ 27af3ab`:
  - `editor_up` → `InputSuggestionsMode::ModelSelector` → `inline_model_selector_view.select_up` (`input.rs:9024`).
  - `editor_down` → `select_down` (`input.rs:9390`).
  - `input_enter` → `inline_model_selector_view.accept_selected_item(false, ..)` (`input.rs:13173`).
  - There is **no** left/right routing to the model selector today (arrows move the editor cursor), so sidecar focus keys are net-new and scoped to `InputSuggestionsMode::ModelSelector`.
- Reference implementation (dropdown): `app/src/terminal/profile_model_selector.rs @ 27af3ab`.
  - Collapse grouping in `refresh_model_menu` (`profile_model_selector.rs:1030`): key = `"auto"` when `is_auto`, else `base_model_name()` when `has_reasoning_level()`, else `id`.
  - Sidecar item construction in `refresh_model_spec_sidecar_for_model` (`profile_model_selector.rs:1193`): filter `all_model_choices` by `base_model_name() == base_name && has_reasoning_level()`, label each by `reasoning_level()`, action `SelectModel(id)`, checkmark on the active id.
  - Sidecar accept in `handle_sidecar_selection` (`profile_model_selector.rs:1262`) → `update_preferred_agent_mode_llm`.
- Shared model helpers (reuse these): `LLMInfo::base_model_name()` (`app/src/ai/llms.rs:418`), `has_reasoning_level()` (`llms.rs:423`), `reasoning_level()` (`llms.rs:428`), `is_auto(llm)` (`app/src/ai/execution_profiles/model_menu_items.rs:19`), `has_reasoning_variants(llm, all)` (`model_menu_items.rs:25`), and `LLMInfo::new_for_test` (`llms.rs:433`) for fixtures.

### Proposed changes
1. **Collapse in the data source (`data_source.rs`).**
   - Add a pure grouping helper (e.g. `collapse_reasoning_variants(choices: &[&LLMInfo]) -> Vec<ModelGroup>`) that mirrors the dropdown's keying (`"auto"` / `base_model_name()` / `id`) and returns, per group, a representative `LLMInfo` plus its ordered reasoning variants (`Vec<{ reasoning_level: String, id: LLMId }>`). Keep it free of `AppContext` so it is unit-testable with `LLMInfo::new_for_test` fixtures.
   - In `run_query`, run collapsing after `order_model_choices`. Emit one `ModelSearchItem` per group. Choose the representative as the active variant when the family is currently selected, else a deterministic default (server default level, else first).
   - Extend `ModelSearchItem` to carry the group's reasoning variants (levels + ids) and a "target level" so it can render the sidecar and resolve `accept_result()` to the concrete variant id.
   - **Fuzzy search:** for collapsed reasoning groups, match against `base_model_name()` (the collapsed label) instead of the per-variant `display_name`; leave non-collapsed items matching on `display_name`. Preserve the existing weak-match score cutoff (`data_source.rs:270`).
2. **Selectable reasoning sidecar.** Render the "Reasoning level" sidecar for a collapsed reasoning item as a list of selectable rows (one per level, active one checked), reusing `REASONING_LEVEL_TITLE`/`REASONING_LEVEL_DESCRIPTION` and `render_model_spec_header`. Each row is a clickable element (same pattern as the existing "Manage" button in `render_details`, `data_source.rs:569`) that dispatches selection/accept of that variant's `LLMId`. Recommended: own the sidecar's keyboard focus + highlighted-level state in `InlineModelSelectorView` (analogous to the dropdown's `model_spec_sidecar`) and thread the targeted level into rendering, rather than adding selection state to the shared `InlineMenuView`. If threading state through `SearchItem::render_details` proves too awkward, the fallback is for `InlineModelSelectorView` to render its own sidecar element next to the menu — either way, do not add interactive-detail state to `InlineMenuView`.
3. **Keyboard routing (`input.rs`).** Add model-selector-scoped `right`/`tab` (enter sidecar), `left`/`escape` (leave sidecar), and make `up`/`down` + `enter` operate on the sidecar when it has focus (`input.rs` around the `InputSuggestionsMode::ModelSelector` arms at `9024`/`9390`/`13173`). Expose the needed methods on `InlineModelSelectorView` (e.g. `focus_sidecar`/`blur_sidecar`/`sidecar_select_up`/`sidecar_select_down`/`accept_sidecar`), keeping all sidecar state inside that view.
4. **Accept resolution.** `ModelSearchItem::accept_result` returns `AcceptModel { id }` for the currently-targeted variant so the existing `SelectedModel`/preferences/`set_as_default` flow (`view.rs:238`) is unchanged.

### Affected files
- `app/src/terminal/input/models/data_source.rs` (collapse helper, `run_query`, `ModelSearchItem`, sidecar rendering)
- `app/src/terminal/input/models/view.rs` (`InlineModelSelectorView` sidecar focus/target state + methods)
- `app/src/terminal/input.rs` (model-selector-scoped left/right/tab/enter routing)
- `app/src/terminal/input/models/data_source_tests.rs` (new; wired via `#[cfg(test)] #[path = "data_source_tests.rs"] mod tests;`)
- Possibly `app/src/terminal/input/models/model_spec_scores.rs` if a shared selectable-row renderer is factored out.

### Risks / blast radius
- **Shared `InlineMenuView`.** The mitigation is to keep sidecar interactivity out of it (design choice #2). If any change to `inline_menu/view.rs` becomes unavoidable, every `InlineMenuType` must be regression-checked.
- **Keyboard capture.** New left/right/tab handling must be strictly gated to `InputSuggestionsMode::ModelSelector` so editor cursor movement and other menus are unaffected.
- **Selection resolution.** Empty/edge families (a family with a single reasoning level, or a stale active id not present in choices) must resolve deterministically without panicking.

## Validation & verification criteria (must ALL pass before merge)
1. **Collapse (unit).** A new test in `data_source_tests.rs` builds `LLMInfo` fixtures for one base model with multiple reasoning levels plus at least one non-reasoning model and one `auto` model, calls the pure collapse helper, and asserts: exactly one group per `base_model_name()` for reasoning families, one `auto` group, and one group per non-reasoning `id`. Checked by `cargo nextest run -p <app crate> collapse`.
2. **Sidecar contents (unit).** A test asserts the collapsed reasoning group exposes its levels in server order with the correct per-level `LLMId`s and that the active variant is flagged selected. Checked by `cargo nextest`.
3. **Accept resolves to concrete variant (unit).** A test asserts `ModelSearchItem::accept_result()` / the sidecar accept path yields `AcceptModel { id }` for the targeted level (active variant when the family is selected; deterministic default otherwise). Checked by `cargo nextest`.
4. **Fuzzy search parity (unit).** A test drives the collapsed-search path with a query equal to a base model name and asserts a single result for that family (not one per level), and that a base-name substring still matches. Checked by `cargo nextest`.
5. **Non-reasoning / auto unaffected (unit).** A test asserts non-reasoning models produce one item each with no reasoning sidecar and auto collapses to a single entry. Checked by `cargo nextest`.
6. **No collateral damage to other inline menus.** `./script/presubmit` passes, and a manual/visual check confirms slash commands, prompts, and skills inline menus still render their details panes and navigate normally (no left/right regressions). Because the shared `InlineMenuView` is intended to be untouched, confirm via `git diff --stat` that `app/src/terminal/input/inline_menu/` is unchanged (or, if changed, explicitly regression-check every menu type).
7. **Presubmit.** `./script/format` produces no diff and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` is clean; `./script/presubmit` passes end-to-end.
8. **Mandatory UI verification video.** A screen recording of the running Warp client (`cargo run` / `./script/run`), captured with the computer-use tool, showing, in one continuous flow:
   a. Opening the inline `/model` picker and seeing exactly one entry per base model (no separate per-reasoning-level rows for a family such as `gpt-5.6-terra`).
   b. Focusing that collapsed row and opening the reasoning sidecar.
   c. Selecting **low**, then **medium**, then **high**, then **xhigh** from the sidecar and observing the active model update to each variant in turn.
   d. Confirming a non-reasoning model still shows its normal details panel with no reasoning sidecar.
   The video is attached to both the PR body and the Linear ticket (APP-4854). This criterion is required and cannot be substituted by unit tests.
9. **Reproduction of the reported behavior is fixed.** The original report ("the same model with different reasoning levels are... shown separately" in the inline picker) no longer reproduces: the video in criterion 8 shows the collapsed single-entry-per-model layout with a selectable reasoning sidecar, matching the dropdown picker.
