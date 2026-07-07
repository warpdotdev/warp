# Custom Host Picker — Tech Spec

Linear: [QUALITY-701](https://linear.app/warpdotdev/issue/QUALITY-701)

Companion product spec: `specs/QUALITY-701/PRODUCT.md`

## Context

The orchestration UI (orchestrate confirmation card and plan-card orchestration block) hosts a row of pickers — model, harness, environment — that drive child agent dispatch. Until now there was no UI to choose the worker host: the dispatched `RunAgents` request always carried `worker_host = "warp"`, which routes to the default Warp cluster. Customers running self-hosted workers had no way to target them from the desktop client; the Oz webapp's host selector is the only existing entry point.

The picker chrome used by the other orchestration pickers is built around the standard `Dropdown` view in `app/src/view_components/dropdown.rs`, styled via `picker_styles()` in `orchestration_controls.rs`. Both card views construct their picker handles via shared helpers in `orchestration_controls.rs` and store them in `OrchestrationPickerHandles`. The worker-host slug already flows end-to-end as a field on `OrchestrationEditState` and on `RunAgentsExecutionMode::Remote`; the missing piece is the UI control that lets a user change it.

The workspace default slug is already exposed to the client as `defaultHostSlug` on `AmbientAgentSettings` and surfaced via `UserWorkspaces::default_host_slug()`. There is also persisted per-user "last selected host" state in `CloudAgentSettings.last_selected_host`. The Oz webapp's `HostSelector` (`client/packages/agents/src/components/HostSelector.tsx`) is the canonical reference for option ordering, default-host preselection, and recent-host surfacing.

### Relevant files

**New**
- `app/src/ai/blocklist/inline_action/host_picker.rs` — the `HostPicker` view itself (list mode + custom mode).
- `app/src/ai/blocklist/inline_action/host_picker_tests.rs` — unit tests for the pure helpers.

**Shared picker plumbing (modified)**
- `app/src/ai/blocklist/inline_action/orchestration_controls.rs` — `OrchestrationPickerHandles` gains a `host_picker` handle; new helpers `populate_host_picker`, `resolve_default_host_slug`, `resolve_recent_host_slug`, and `persist_host_selection`; `sync_picker_selections` is taught to drive the host picker.
- `app/src/ai/blocklist/inline_action/mod.rs` — registers the new module.

**Call sites (modified)**
- `app/src/ai/blocklist/inline_action/run_agents_card_view.rs` — confirmation card: builds the picker, opens its menu upward, subscribes to its events, re-dispatches `WorkerHostChanged`.
- `app/src/ai/document/orchestration_config_block.rs` — plan card: builds the picker, opts the inner menu into the overlay layer, subscribes to its events, dispatches `WorkerHostChanged`, persists field changes.

**Reference**
- `client/packages/agents/src/components/HostSelector.tsx` (warp-server) — the webapp's host selector.

## Proposed changes

### 1. New `HostPicker` view

A single non-generic view that internally switches between two render modes:

- **List mode** wraps an inner `Dropdown<InternalAction>` styled identically to the other orchestration pickers (`picker_styles()`). The menu is populated with the workspace default (badged "Default"), `warp`, the most-recent custom slug, and a "Custom host…" entry, in that order. Selecting any known item dispatches `InternalAction::SelectKnown(slug)`; selecting "Custom host…" dispatches `InternalAction::EnterCustomMode`.

- **Custom mode** swaps the dropdown top bar for an inline single-line `EditorView` plus a small cancel button. Enter or blur commits via `commit_custom`; Escape or cancel reverts via `cancel_custom`. The editor is wrapped in a `Flex::column` with `MainAxisAlignment::Center` so the glyphs sit at the vertical center of the picker box (otherwise the row's tight cross-axis constraint forces the editor to fill the height and the text renders flush to the top). The custom-mode container is wrapped in an outer `Container` with vertical margins equal to `DROPDOWN_PADDING`, mirroring the standard `Dropdown` view's outer wrapping so the custom box sits at the same y offset as the other pickers in the row.

The picker emits two public events:
- `HostPickerEvent::HostChanged { slug }` — sent whenever the current selection changes.
- `HostPickerEvent::Closed` — sent whenever the menu closes or the editor blurs, so the parent can refocus its own input.

Public API:
- `set_options(default_host, recent_host, ctx)` — replaces the menu rows.
- `set_selected(slug, ctx)` — sets the displayed slug; unknown slugs switch into custom mode pre-filled with the slug.
- `set_use_overlay_layer(bool, ctx)` — forwarded to the inner dropdown.
- `set_menu_position(element_anchor, child_anchor, ctx)` — forwarded to the inner dropdown.

Two subtleties worth noting in the implementation:
- The inner dropdown's `DropdownEvent::Close` is suppressed while the picker is transitioning into custom mode. If we let it through, the parent card refocuses itself, blurs the editor we just focused, and the resulting commit-on-blur immediately reverts custom mode — making "Custom host…" feel like a no-op.
- When the user types `warp` into custom mode, `commit_custom` collapses back to the standard `warp` selection rather than persisting `warp` as a custom value. This avoids the asymmetric case where `current_slug` is a casing variant of `warp` that doesn't match any menu label.

Pure helpers (`build_menu_items`, `menu_label_for`, `normalize_slug`) live at the bottom of the module and are unit-tested without spinning up a view context.

### 2. Shared orchestration helpers

`OrchestrationPickerHandles` gets a new `host_picker: Option<ViewHandle<HostPicker>>` field. `sync_picker_selections` is taught to call `picker.set_selected(...)` with the current `worker_host` whenever the edit state changes; this handles both initial population and subsequent changes from other pickers (e.g. mode toggle resetting host to `warp`).

Four new free functions in `orchestration_controls.rs`:

- `populate_host_picker(picker, initial_host, ctx)` — reads the workspace default and recent slug, calls `picker.set_options(...)`, then `picker.set_selected(initial_host)`. Empty input falls back to `warp`. Used by both card views during `ensure_pickers`.

- `resolve_default_host_slug(ctx) -> Option<String>` — returns the workspace default slug, honoring the developer-only `WARP_CLOUD_MODE_DEFAULT_HOST` env var override, otherwise reading from `UserWorkspaces::default_host_slug()`. Mirrors the single-agent ambient flow.

- `resolve_recent_host_slug(ctx) -> Option<String>` — returns the persisted last-selected custom slug, deduplicated against `warp` and the workspace default (so the menu doesn't show a duplicate row).

- `persist_host_selection(worker_host, ctx)` — writes the slug to `CloudAgentSettings.last_selected_host`. Skipped for empty values and for `warp` so those never become "recent" entries.

Both card views also pre-fill defaults when restoring a Remote config with an empty host: prefer the workspace default over the bare `warp` fallback so self-hosted teams see their default pre-selected, matching the Oz webapp.

### 3. Confirmation card wiring (`run_agents_card_view.rs`)

`ensure_pickers` constructs a `HostPicker` for the new `host_picker` slot and:
- Calls `picker.set_menu_position(TopLeft, BottomLeft)` so the open menu flips upward, matching the other dropdowns in this card (which use `set_upward_menu_position` for the same reason). Without this the menu visually collides with the Environment / Base model rows below.
- Calls `populate_host_picker` to seed options and selection.
- Subscribes to `HostPickerEvent`: `HostChanged` re-dispatches the existing `RunAgentsCardViewAction::WorkerHostChanged`; `Closed` refocuses the card.

The existing `WorkerHostChanged` handler updates `state.orch.worker_host` and calls `oc::persist_host_selection`, so any path that ends in a host change persists the slug.

### 4. Plan-card wiring (`orchestration_config_block.rs`)

`ensure_pickers` constructs a `HostPicker` for the new `host_picker` slot and:
- Calls `picker.set_use_overlay_layer(true)` so the menu paints above siblings, matching the other pickers in this view (which all opt into the overlay layer).
- Calls `populate_host_picker` to seed options and selection.
- Subscribes to `HostPickerEvent::HostChanged` to dispatch `OrchestrationConfigBlockAction::WorkerHostChanged`, which updates the edit state, calls `oc::persist_host_selection`, and `apply_field_change` (writes the new value into the plan's stored `OrchestrationConfig`).

### 5. No other call sites

The `worker_host` field already exists on `OrchestrationEditState` and on `RunAgentsExecutionMode::Remote`, so no downstream changes (dispatch, server marshalling, auto-launch matching) are needed. The previously-hardcoded `"warp"` value flows through the same code paths as any user-selected slug.

## Testing and validation

### Unit tests (`host_picker_tests.rs`)

The pure helpers are tested directly without a view context. Covers product invariants 4, 5, 11, 14, 16, 18.

- `build_menu_items` with no default and no recent → only `warp` + `Custom host…`. (Behavior 4)
- `build_menu_items` with default set → default row first, badged; then `warp`; then `Custom host…`. (Behavior 4)
- `build_menu_items` with recent set → `warp` first; then recent as plain slug; then `Custom host…`. (Behavior 4)
- `build_menu_items` dedups when recent equals default. (Behavior 5)
- `build_menu_items` dedups when recent equals `warp`. (Behavior 5)
- `build_menu_items` warp entry dispatches `SelectKnown("warp")`. (Behavior 6)
- `build_menu_items` custom entry dispatches `EnterCustomMode`. (Behavior 8)
- `menu_label_for` picks the "Default" badge when the slug matches the workspace default. (Behavior 6)
- `menu_label_for` returns plain slug for `warp`. (Behavior 6)
- `menu_label_for` returns plain slug for an unknown value (custom-mode display). (Behavior 15)
- `normalize_slug` trims whitespace and falls back to `warp` on empty input. (Behavior 14)

### Manual validation

The view-driven behaviors (custom-mode commit, blur, focus return, layer interaction with sibling pickers) are covered by manual smoke testing rather than view-level tests:

- **Behavior 1, 2, 3**: Open a plan with orchestration approved and an orchestrate confirmation card; verify the host picker is present in both surfaces, only in Cloud mode, and visually matches the model / harness / environment pickers.
- **Behavior 4, 5, 6**: With and without `defaultHostSlug` configured (toggle via SQL on the local `organization_settings` table, or via `WARP_CLOUD_MODE_DEFAULT_HOST`), verify the dropdown contents and ordering, the "Default" badge, and the badge appearing in the collapsed top bar.
- **Behavior 8, 9, 10, 11, 12**: Open custom mode, verify the editor is pre-filled and focused; type a slug, press Enter; reopen the menu and verify the slug now appears as a recent entry. Repeat with Escape and with the cancel button. Try committing an empty buffer and the literal string `warp` / `WARP`.
- **Behavior 13**: Visually compare the custom-mode box to its neighbours; the editor text should be vertically centered and the box should sit at the same y as the sibling pickers.
- **Behavior 15**: Use the developer override (`WARP_CLOUD_MODE_DEFAULT_HOST=some-unknown-slug`) and verify the picker boots into custom mode pre-filled with the slug.
- **Behavior 16, 17, 18**: Pick a custom slug, dismiss the card, then reopen another plan / confirmation card; verify the slug appears as the recent entry. With a workspace default set, verify the recent entry deduplicates against it.
- **Behavior 19**: Pick a non-`warp` slug, dispatch the agents, and verify the worker-host slug reaches the worker. End-to-end smoke against a local Oz stack with a self-hosted worker registered as `local-dev` is the canonical check; worker logs show `task_claimed worker_id:"local-dev"` when the custom slug is routed correctly.
- **Behavior 20**: Open the menu in each surface and confirm it doesn't visually overlap the Environment or Base model rows.
- **Behavior 21**: After every menu close or custom-mode commit, the input box of the parent card should regain focus.

### Presubmit

`cargo fmt`, `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`, and the host_picker nextest suite all pass clean. Run `./script/presubmit` before opening the PR.

## Parallelization

Not used. The implementation is a single new view file plus thin wiring at two call sites; the work is sequentially tightly coupled (helpers feed the view, the view feeds both call sites) and small enough that splitting across agents would add coordination overhead without saving wall-clock time.
