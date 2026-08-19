# TECH: Frontend-neutral orchestration option snapshots

## Context

This slice builds on the frontend-neutral orchestration edit-state module
(`app/src/ai/orchestration/`, base commit `ff91958d`; see
`specs/code-1822-edit-state/TECH.md`). At that base, the domain module owns the edit
state, transitions, validation predicates, and catalog providers, but the GUI pickers
still build their option lists inline:

- `app/src/ai/blocklist/inline_action/orchestration_controls.rs` — the
  `populate_model_picker_for_harness`, `populate_harness_picker`,
  `populate_environment_picker`, `populate_host_picker`,
  `populate_auth_secret_picker_for_harness`, and `sync_picker_selections` bodies read
  catalogs (`HarnessAvailabilityModel`, `LLMPreferences`,
  `CloudAmbientAgentEnvironment`, `ConnectedSelfHostedWorkersModel`) and translate them
  straight into `MenuItem`s, including ordering, filtering, disabled reasons, badges,
  and selection matching. The `DEFAULT_MODEL_LABEL` and `AUTH_SECRET_INHERIT_LABEL`
  display strings are defined in this GUI file.

The planned TUI orchestration card must render the exact same option lists — same rows,
order, badges, disabled reasons, load/error states, and selection — without depending on
GUI element types. That requires lifting the catalog-to-rows logic into the domain
module as plain data, and rewriting the GUI pickers to consume it so the logic cannot
drift between frontends.

## Proposed changes

### Snapshot contract (`app/src/ai/orchestration/snapshots.rs`)

Plain-data option lists both frontends render from:

- `OptionRow` — `id`, `label`, optional `harness: Option<Harness>` (each frontend maps
  it to its own icon or glyph), optional `badge`, optional `disabled_reason`.
- `OptionBadge` — `Default` / `Recent` / `Connected` secondary markers.
- `OptionSourceStatus` — `Ready` / `Loading` / `Failed { message }` /
  `Empty { message }` load state of the backing catalog.
- `OptionFooter` — trailing affordance: `CustomText { label }` (custom host slug entry)
  or `CreateNewAuthSecret` ("New API key…"; the TUI ignores it since resource creation
  is out of scope there).
- `OptionSnapshot` — `rows`, `selected_id: Option<String>`, `status`, `footer`.

Six builders, one per configuration field, each taking
`(&OrchestrationConfigState, &AppContext)`:

- `location_snapshot` — Cloud/Local rows (`LOCATION_CLOUD_ID` / `LOCATION_LOCAL_ID`);
  only the TUI renders a location page, the GUI keeps its own mode toggle.
- `harness_snapshot` — mirrors the GUI harness picker: filters Gemini and
  product-disabled local harnesses, sorts selectable before disabled, resolves stale
  `Harness::Unknown` cache entries by display name via `harness_display::display_name`,
  and carries `disabled_reason` from `LocalHarnessSetupState` tooltips.
- `model_snapshot` — three catalog branches: Oz/unset uses the Warp LLM catalog
  (auto, then custom, then other models; custom models excluded for Cloud runs), local
  Codex gets only a "Default model" row, other non-Oz harnesses get "Default model" plus
  the server-provided harness model catalog with unknown selections falling back to the
  default (empty id).
- `api_key_snapshot` — "Skip (advanced)" inherit row plus loaded managed-secret names
  (names only, never values); status mirrors `AuthSecretFetchState`; emits the
  `CreateNewAuthSecret` footer when `auth_secret_types_for_harness` is non-empty; keeps
  a `Named` selection while the catalog is loading.
- `host_snapshot` — workspace default (badged `Default`), `warp`, connected worker
  hosts (badged `Connected`, case-insensitively deduped), the recent custom slug
  (badged `Recent`), then a `CustomText` footer.
- `environment_snapshot` — "Empty environment" plus existing environments sorted by
  name.

Each builder is a thin `AppContext`-reading wrapper over a pure core
(`build_harness_snapshot`, `build_oz_model_snapshot`, `build_non_oz_model_snapshot`,
`build_api_key_snapshot`, `build_host_snapshot`, `build_environment_snapshot`) so the
mirroring logic is unit-testable without app singletons; catalog-dependent inputs
(`HarnessEntryInput`, `ModelChoiceInput`, `AuthSecretNamesInput`, setup-state callback)
are injected.

`DEFAULT_MODEL_LABEL` and `AUTH_SECRET_INHERIT_LABEL` move into this module (the GUI
file's copies are deleted); `AUTH_SECRET_INHERIT_LABEL` is re-exported `pub(crate)` for
the GUI's selected-label rendering.

`app/src/ai/orchestration/mod.rs` declares `mod snapshots` and re-exports the types and
builders. Items only named by the TUI (`location_snapshot`, `OptionRow`,
`ORCHESTRATION_ENV_NONE_LABEL`, `auth_secret_selection_required`,
`harness_is_selectable`) carry `#[cfg_attr(not(feature = "tui"), allow(unused_imports))]`
so `cargo check` is clean with and without the `tui` feature; the same pattern applies
`allow(dead_code)` to `harness_is_selectable` / `local_harness_setup_is_ready` in
`validation.rs`, which the GUI now reaches only through the harness snapshot builder.

### GUI adaptation (same diff, behavior-parity gate)

The GUI pickers are rewritten onto the snapshots in the same PR so the snapshot layer is
proven equivalent by the existing GUI tests, and the inline catalog logic is deleted
rather than duplicated (net-negative in `orchestration_controls.rs`):

- `app/src/ai/blocklist/inline_action/orchestration_controls.rs` — each `populate_*`
  body calls its snapshot builder and translates rows/status/footer into `MenuItem`s;
  `sync_picker_selections` re-derives selections from fresh snapshots. Behavior parity
  requirements the translation must preserve:
  - Oz model rows stay rich: snapshot rows are matched back through
    `available_model_menu_items` (`orchestration_controls.rs:283`) so provider and
    credential icons render exactly as in the execution-profile model menu; non-Oz rows
    render from the snapshot labels directly.
  - Harness rows map `OptionRow::harness` through `harness_display::icon_for` and
    `harness_display::brand_color` (`orchestration_controls.rs:335`), and
    `disabled_reason` becomes the disabled tooltip.
  - `OptionFooter::CreateNewAuthSecret` maps to the "New API key…" item, and
    `OptionSourceStatus::Loading`/`Failed` map to the disabled placeholder items
    (`orchestration_controls.rs:559`-`579`).
  - Host badges map `Default`→"(default)", `Connected`→"(connected)",
    `Recent`→"(recent)" suffixes, and the `CustomText` footer stays the custom-host
    entry row.
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs`,
  `app/src/ai/document/orchestration_config_block.rs`, and
  `app/src/ai/blocklist/action_model/execute/run_agents.rs` consume the adapted picker
  APIs; all pre-existing GUI tests must pass unchanged.

### TUI export surface

`app/src/tui_export.rs` extends the existing `crate::ai::orchestration` re-export block
with `api_key_snapshot`, `environment_snapshot`, `harness_snapshot`, `host_snapshot`,
`location_snapshot`, `model_snapshot`, `OptionBadge`, `OptionFooter`, `OptionRow`,
`OptionSnapshot`, and `OptionSourceStatus`. `resolve_recent_host_slug` drops out of the
block: recent-host data now reaches frontends through `host_snapshot`.

## Testing and validation

- `app/src/ai/orchestration/snapshots_tests.rs` (colocated per repo convention) covers
  the pure cores: harness filtering/sorting/disabled reasons/stale-cache selection
  matching, the three model branches and default fallback, api-key status mapping and
  footer emission, host ordering/badging/dedup, and environment sorting/selection.
- Pre-existing GUI tests (`run_agents_card_view_tests.rs`, `run_agents_tests.rs`,
  `orchestration_config_block` coverage) must pass unchanged — they are the
  behavior-parity gate for the picker rewrite.
- Commands: `cargo check -p warp`, `cargo check -p warp --features tui`,
  `cargo nextest run -p warp -E 'test(orchestration) + test(run_agents) + test(run_agents_card_view) + test(orchestration_config_block)'`,
  plus `./script/format`.
- Manual: all five pickers on the confirmation card and plan-card config block render
  the same rows, icons, badges, disabled tooltips, and loading/error placeholders as
  before, and selection syncing after harness/mode changes is unchanged.

## Follow-ups

Later PRs render these snapshots in the TUI: the host/environment selector and the TUI
orchestration card consume the builders exported through `tui_export`.
