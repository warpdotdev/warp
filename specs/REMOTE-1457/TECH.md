# TECH.md
Companion to `specs/REMOTE-1457/PRODUCT.md`.
## Context
Add a Harness dropdown to `AgentManagementView`. Behavior lives in `PRODUCT.md`; this plan covers the filter plumbing, where the dropdown plugs in, and how harness resolution reuses the helpers introduced by REMOTE-1455.
Relevant code:
- `app/src/ai/agent_conversations_model.rs (60-150)` — existing `StatusFilter` / `SourceFilter` / `ArtifactFilter` / `EnvironmentFilter` enums and the `AgentManagementFilters` aggregate (incl. `reset_all_but_owner` / `is_filtering`). New `HarnessFilter` lands here.
- `app/src/ai/agent_conversations_model.rs (332-728)` — `ConversationOrTask` impl (`source`, `environment_id`, `matches_status`, `matches_artifact`, `matches_owner_and_creator`).
- `app/src/ai/agent_conversations_model.rs (1379-1443)` — `get_tasks_and_conversations`: where the per-item `*_filter` closures are chained. The harness filter plugs in here.
- `app/src/ai/agent_conversations_model.rs (1585-1647)` — `build_task_list_filter`: converts `AgentManagementFilters` into the server-side `TaskListFilter`. Harness is *not* added to `TaskListFilter` in this change — client-side filtering only (see "Scope" below).
- `app/src/ai/agent_management/view.rs (154-198)` — `AgentManagementView` struct: new `harness_dropdown` field goes here.
- `app/src/ai/agent_management/view.rs (265-450)` — dropdown construction + `sync_with_loaded_filters` + `update_filter_buttons`.
- `app/src/ai/agent_management/view.rs (468-702)` — existing dropdown builders (`create_status_dropdown`, `create_artifact_dropdown`, `setup_filter_menu`): the new `create_harness_dropdown` mirrors these.
- `app/src/ai/agent_management/view.rs (847-881)` — `apply_environment_filter_from_link`: must reset the new harness dropdown too (invariant 16).
- `app/src/ai/agent_management/view.rs (1876-1892)` — `filters_wrap` in `render_task_list_header`: the new dropdown gets added to this wrap row.
- `app/src/ai/agent_management/view.rs (2142-2247)` — `AgentManagementViewAction` enum + `handle_action` (incl. `ClearFilters`): new `SetHarnessFilter` variant and handler.
- `app/src/ai/agent_management/view.rs (1741-1770)` — `render_metadata_row`: builds `metadata_parts: Vec<String>` joined by ` • ` (Source, Run time, Credits used). The card Harness segment is inserted here.
- `app/src/ai/agent_management/telemetry.rs (43-52)` — `FilterType` enum: add `Harness`.
- `app/src/ai/harness_display.rs` — `display_name`, `icon_for`, `brand_color` already exist (from REMOTE-1455). Reused unchanged.
- `app/src/ai/ambient_agents/task.rs (57-83)` — `AgentConfigSnapshot.harness: Option<HarnessConfig>`. Authoritative source for a task's harness.
- `app/src/app_state.rs (37-59)` — `PersistedAgentManagementFilters` wraps `AgentManagementFilters` via serde. Adding a new field with `#[serde(default)]` keeps old persisted state compatible.
## Proposed changes
### 1. `HarnessFilter` enum + plumbing in `AgentManagementFilters`
In `agent_conversations_model.rs`, add:
```rust path=null start=null
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum HarnessFilter {
    #[default]
    All,
    Specific(Harness),
}
```
`Harness` is `Copy` and already `Serialize + Deserialize` via `warp_cli::agent::Harness` (used elsewhere). `HarnessFilter` stays `Copy` so it threads through actions cheaply.
Extend `AgentManagementFilters`:
```rust path=null start=null
pub struct AgentManagementFilters {
    // ...existing fields...
    #[serde(default)]
    pub harness: HarnessFilter,
}
```
`#[serde(default)]` keeps `PersistedAgentManagementFilters` backwards compatible with existing on-disk state (invariant 11). Update `reset_all_but_owner` to zero `harness` and `is_filtering` to include `self.harness != HarnessFilter::default()` (invariants 9, 10).
### 2. Resolve harness on `ConversationOrTask`
Add on `ConversationOrTask`:
```rust path=null start=null
pub fn harness(&self) -> Option<Harness> {
    match self {
        ConversationOrTask::Task(task) => task.agent_config_snapshot.as_ref().and_then(|c| {
            c.harness
                .as_ref()
                .map(|h| h.harness_type)
                .or(Some(Harness::Oz))
        }),
        // Local/interactive conversations always run on Warp Agent.
        ConversationOrTask::Conversation(_) => Some(Harness::Oz),
    }
}

fn matches_harness(&self, f: &HarnessFilter) -> bool {
    match f {
        HarnessFilter::All => true,
        HarnessFilter::Specific(h) => self.harness() == Some(*h),
    }
}
```
Resolution mirrors `ConversationDetailsData::from_task` from REMOTE-1455 exactly, so the filter and the details panel agree on which harness label belongs to a row:
- snapshot present + `harness` set → `Some(harness_type)`,
- snapshot present + no `harness` field → `Some(Harness::Oz)` (runtime default for an Oz-managed task),
- snapshot not loaded yet (stub) → `None` ("don't know yet"),
- local/interactive conversation → `Some(Harness::Oz)`.
`HarnessConfig.harness_type` is already a parsed `warp_cli::agent::Harness`, so the resolver no longer needs to re-parse a raw string — unknown values are already collapsed to `Harness::Oz` by the snapshot deserializer (`harness_from_name` in `ambient_agents/task.rs`). PRODUCT invariant 6 bullet 3 follows directly from `harness() == None` not matching any `HarnessFilter::Specific(_)`.
`HarnessFilter`'s `Deserialize` impl uses clap's `Harness::from_str` to coerce persisted `"oz" | "claude" | "gemini" | "all" | <unknown>` strings, falling back to `HarnessFilter::All` for unknown values; `harness()` itself doesn't parse strings.
Wire `matches_harness` into `get_tasks_and_conversations` as another closure in the `.filter(...)` chain, alongside the existing `source_filter` / `status_filter` / `environment_filter` (invariant 8).
### 3. View: dropdown construction and wiring
In `AgentManagementView`:
- New field `harness_dropdown: ViewHandle<Dropdown<AgentManagementViewAction>>`.
- New builder `create_harness_dropdown(ctx)` modeled on `create_status_dropdown`:
    - `Self::setup_filter_menu(&mut dropdown, "Harness", ctx)` for the `Harness: <selected>` button label (invariant 2).
    - Items: `All`, then one per `Harness` variant in the order listed in PRODUCT invariant 3 (`Oz`, `Claude`, `Gemini`). Each non-`All` item uses `MenuItemFields::new(display_name(h)).with_icon(icon_for(h))` and, when `brand_color(h)` is `Some(c)`, `.with_override_icon_color(Fill::from(c))`. Warp Agent renders with the default theme-foreground tint (invariant 4) because `brand_color(Harness::Oz)` is `None`. Relies on `harness_display::display_name` / `icon_for` / `brand_color` and the `MenuItemFields` icon helpers from `app/src/menu.rs`.
- `sync_with_loaded_filters` adds a block that calls `harness_dropdown.set_selected_by_action(SetHarnessFilter(self.filters.harness), ctx)`, mirroring the existing status/source/created_on/artifact blocks.
- `ClearFilters` handler resets the harness dropdown to its zeroth item (invariant 9).
- `apply_environment_filter_from_link` resets the harness dropdown to its zeroth item (invariant 16); `reset_all_but_owner` already clears `filters.harness` via (1).
- `render_task_list_header`: append `ChildView::new(&self.harness_dropdown)` to the `filters_wrap` (invariant 1). Exact placement: after `artifact_dropdown`, before `environment_dropdown`, to keep related filters adjacent. Visible under the same conditions as the other dropdowns, no feature-flag gating.
### 4. Action, telemetry, and refresh
- Add `AgentManagementViewAction::SetHarnessFilter(HarnessFilter)`.
- Add `FilterType::Harness` to `app/src/ai/agent_management/telemetry.rs` (invariant 17). `#[serde(rename_all = "snake_case")]` is already set on that enum, so the payload string is `"harness"`; `warp_core::telemetry::enum_events` picks up the new variant automatically via the existing `EnumDiscriminants` / `EnumIter` derives.
- `handle_action` branch mirrors the status branch: send `FilterChanged { filter_type: FilterType::Harness }`, update `self.filters.harness`, call `self.on_filter_changed(ctx)` (invariant 12).
### 5. Card metadata row
Extend `render_metadata_row` in `view.rs` with an `if let Some(h) = card_data.harness()` block that pushes `format!("Harness: {}", harness_display::display_name(h))` into `metadata_parts` immediately after the Source push, where `h` is `card_data.harness()` from change (2). The `if let` mirrors the details panel: when `harness()` returns `None` (a server-side stub task whose snapshot hasn't been fetched yet), the Harness segment is omitted instead of guessing — matching invariants 20–21. Once the snapshot loads in a follow-up update, the segment appears on the next render. Use the unconditional `display_name` mapping for the three known variants; no icon, no color override, no click handler (invariant 22).
Ordering: pushed between the Source push and the Run time push so the resulting ` • `-joined string matches `Source: … • Harness: … • Run time: … • Credits used: …`. If Source is absent, a present Harness segment becomes the leading segment with no leading separator because `metadata_parts.join(" • ")` skips empty positions by construction (invariant 21).
No new view state, mouse state, or subscription is introduced for the card segment — it is pure data derived from `ConversationOrTask::harness()` at render time.
### 6. Scope: client-side only
This change deliberately does *not* add `harness` to `TaskListFilter` / `build_task_list_filter` / the server query string (`app/src/server/server_api/ai.rs (492-650)`). Invariant 12 is satisfied because `on_filter_changed` already re-issues `trigger_filter_fetch` with the remaining filters and calls `get_tasks_from_model`, and `matches_harness` enforces the harness constraint on the client over everything in the model — the server simply returns a possibly-larger superset which the client narrows. If we later want server support, the extension is a new `Option<Harness>` field on `TaskListFilter` + a `harness=` query param; listed under Follow-ups.
## Testing and validation
Behavior invariants from `PRODUCT.md` map as follows:
- Invariants 6, 8, 15 — unit tests in `agent_conversations_model` tests exercising `get_tasks_and_conversations` with fixtures that cover: (a) cloud task with `harness_type = Harness::Claude` → matches `Claude` only, (b) cloud task with `agent_config_snapshot = None` → matches **only** `All` (`harness() == None`), (c) cloud task with `agent_config_snapshot = Some { harness: None }` → matches `Warp Agent` only, (d) local conversation → matches `Warp Agent` only, (e) combinations of harness + status + owner to lock AND semantics and independence from the `Personal`/`All` toggle. Unknown `harness_type` strings are not separately tested at this layer because `HarnessConfig`'s deserializer collapses them to `Harness::Oz` before they reach `ConversationOrTask::harness()`; that mapping is exercised by the `task.rs` deserializer tests.
- Invariants 9, 10, 16 — unit tests in the same module asserting `AgentManagementFilters::is_filtering()` returns `true` when only `harness` is set, and `reset_all_but_owner()` zeroes `harness` back to `HarnessFilter::All`. Dropdown re-selection on `ClearFilters` / `apply_environment_filter_from_link` is review-only.
- Invariant 11 — a serde round-trip test on `AgentManagementFilters`: deserializing a JSON object without a `harness` key yields `HarnessFilter::All` (backwards compat with existing `PersistedAgentManagementFilters`), and serialize+deserialize preserves a `Specific(Claude)` value.
- Invariants 2, 3, 4, 5 — structural: the dropdown is built from the same ordered array of `Harness` variants and uses `harness_display::display_name` / `icon_for` / `brand_color`, which are themselves tested in `harness_display_tests.rs` from REMOTE-1455. A smoke snapshot of the dropdown items (labels only) guards against accidental reordering.
- Invariant 17 — assertion that `FilterType::Harness` serializes as `"harness"` (snake_case rename) and that the existing registered telemetry enumeration picks up the new variant via `AgentManagementTelemetryEventDiscriminants::iter()`.
- Invariants 1, 7, 13, 14, 18 — covered by manual verification, not dedicated tests: structurally, `get_tasks_from_model` is the only consumer that reads `self.filters.harness`, and the dropdown is placed inside the same `filters_wrap` as its siblings so it inherits their wrapping/focus/keybinding behavior.
- Invariants 19, 20, 21, 22 — review-only: `render_metadata_row` pushes the `Harness: <display_name>` segment from `ConversationOrTask::harness()`, whose resolution is unit-tested. Invariant 23 (per-card harness string aligns with filter selection) follows from both the card and the filter using the same `harness()` resolver, already exercised by the `matches_harness` tests.
- Manual verification — `cargo run`, open the management view, and in order: (a) open Harness dropdown → exactly four options in the documented order with brand-tinted icons on the three non-`All` rows; (b) pick `Claude Code` → list narrows; (c) toggle Owner `Personal` ↔ `All` → harness constraint still applies; (d) pick `Warp Agent` → see local conversations and cloud tasks with no `harness_type` set; (e) click `Clear all` → harness returns to `All`; (f) set to `Gemini CLI`, restart Warp → selection is restored; (g) follow a deep-link that invokes `apply_environment_filter_from_link` → harness resets to `All`.
- Pre-PR — `./script/presubmit` per repo rules. No WASM-specific paths; `warp_cli::agent::Harness` already compiles for WASM.
## Risks and mitigations
- **Server returns tasks whose harness does not match the selected filter.** Expected under invariant 12 when the server does not yet filter by harness; the client drops them in `get_tasks_and_conversations`. Result is a potentially smaller rendered page than the server intended, which is cosmetic. Mitigation: add server-side `harness` to `TaskListFilter` when available (see Follow-ups).
- **Persisted filters from older clients carry unknown fields.** Guarded by `#[serde(default)]` on `harness` and serde's default laxness on unknown fields already assumed elsewhere on this struct.
- **New `Harness` variants added in `warp_cli` would render only in three-item order.** `create_harness_dropdown` iterates the variants explicitly to control order and icon/color mapping, so adding a variant requires touching the dropdown builder. This matches PRODUCT invariant 3's "adding it is out of scope" stance — mitigation is a future PR that extends the explicit list.
## Follow-ups
- Add `harness: Option<Harness>` to `TaskListFilter` + `&harness=` query param in `build_list_agent_runs_url` once the public API supports it, and wire through `build_task_list_filter`. Reduces client-side filtering overhead but is not required for any invariant.
- If a new `Harness` variant ships, extend `create_harness_dropdown`'s ordered variant list in one place.
