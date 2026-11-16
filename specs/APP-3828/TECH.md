# APP-3828: Tech Spec — Vertical Tabs `View as` Panes / Tabs

## Problem

APP-3828 adds a new `View as` control to the vertical tabs options popup. The product behavior is intentionally narrow:

- `Panes` preserves the current pane-centric rendering
- `Tabs` renders one representative row per tab
- the representative row is the existing row UI for that tab’s active pane (`Focused session`)
- the current compact / expanded density controls and pane-row display controls remain in place

Technically, the current vertical tabs implementation has no abstraction for “row granularity.” It assumes that every visible pane in a `PaneGroup` should be rendered and searched independently. The new feature therefore needs a low-risk way to:

- add a new synced setting
- wire a new popup control into existing action / settings flow
- centralize the decision of “which pane ids should this tab render/search right now?”
- keep the existing row renderers unchanged as much as possible

## Relevant code

- `specs/APP-3828/PRODUCT.md` — agreed user-facing behavior for this feature
- `app/src/workspace/tab_settings.rs (171-276)` — current synced vertical-tabs settings (`VerticalTabsViewMode`, `VerticalTabsPrimaryInfo`, `VerticalTabsCompactSubtitle`)
- `app/src/workspace/action.rs (237-241)` — existing vertical-tabs popup actions
- `app/src/workspace/action.rs (740-741)` — `should_save_app_state_on_action` coverage for the current vertical-tabs setting actions
- `app/src/workspace/view.rs (17761-17810)` — workspace-side action handlers that update `TabSettings`
- `app/src/workspace/view/vertical_tabs.rs (246-344)` — `VerticalTabsPanelState` and popup mouse-state ownership
- `app/src/workspace/view/vertical_tabs.rs (458-483)` — `matching_tab_indices`, which currently assumes every visible pane can make a tab searchable
- `app/src/workspace/view/vertical_tabs.rs (630-799)` — `render_groups`, including current search filtering behavior
- `app/src/workspace/view/vertical_tabs.rs (800-1034)` — `render_tab_group`, which currently renders one row per visible pane
- `app/src/workspace/view/vertical_tabs.rs (1569-1664)` — `PaneProps::new` and query matching helpers used by render/search
- `app/src/workspace/view/vertical_tabs.rs (2380-2664)` — `render_settings_popup`, including the current top-of-popup density segmented control and the existing secondary controls
- `app/src/workspace/view/vertical_tabs.rs (2845-3033)` — `render_compact_pane_row`; compact density reuses `PaneProps`
- `app/src/workspace/view/vertical_tabs.rs (1160-1250)` — `render_pane_row`; expanded density reuses `PaneProps`
- `app/src/pane_group/mod.rs:1981` — `PaneGroup::focused_pane_id`, the current source of truth for the pane last focused within a tab
- `app/src/pane_group/mod.rs (4566-4571)` — `PaneGroup::display_title`, which already derives tab-level display state from the focused pane
- `app/src/workspace/view/vertical_tabs_tests.rs (1-196)` — current unit-test home for vertical-tabs pure logic
- `app/src/workspace/action_tests.rs (1-36)` — current tests for vertical-tabs action persistence behavior

## Current state

### Settings and actions

Vertical-tabs display preferences already follow a consistent pattern:

- the setting enum lives in `TabSettings`
- the enum is registered with `implement_setting_for_enum!`
- the popup dispatches a `WorkspaceAction::*`
- `Workspace::handle_action` writes the new value into `TabSettings`
- `should_save_app_state_on_action` returns `false`, because persistence is handled by the settings framework rather than workspace snapshotting

This pattern currently exists for:

- `VerticalTabsViewMode` = compact vs expanded density
- `VerticalTabsPrimaryInfo`
- `VerticalTabsCompactSubtitle`

### Popup structure

`render_settings_popup` currently starts with the compact / expanded segmented control, then renders the existing pane-row controls below it. There is no concept of a top-level “what does each row represent?” setting.

### Row rendering

`render_tab_group` obtains `visible_pane_ids()` from the `PaneGroup`, builds `PaneProps` for each one, and then delegates to either:

- `render_compact_pane_row`
- `render_pane_row`

Those row renderers already contain the exact UI we want to reuse in `Tabs` mode.

### Search behavior

The current search flow is duplicated in two places:

- `matching_tab_indices` decides which tabs are included in keyboard navigation / search result bookkeeping
- `render_groups` computes `matching_ids` to decide which rows to render while searching

Both paths iterate all `visible_pane_ids()` and check each pane independently with `PaneProps::new` plus `pane_matches_query`.

### Tab-level “active pane” state

For this feature, the correct tab representative is not `active_session_id()` because tabs may be backed by non-terminal panes. The right primitive is `PaneGroup::focused_pane_id()`:

- it works for any pane type
- it already tracks the pane that would be focused when the tab becomes active again
- `PaneGroup::display_title()` already treats the focused pane as the tab-level source of truth

That makes `focused_pane_id()` the right backing state for “Focused session” in Tabs mode.

## Proposed changes

### 1. Add a new synced setting for row granularity

Add a new enum in `app/src/workspace/tab_settings.rs`:

```rust path=null start=null
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq, Copy, Clone)]
pub enum VerticalTabsDisplayGranularity {
    #[default]
    Panes,
    Tabs,
}
```

Register it in `TabSettings` with the same sync / hierarchy behavior as the existing vertical-tabs settings:

- `SupportedPlatforms::ALL`
- `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- `hierarchy: "appearance.tabs"`

Deliberately do not rename the existing `VerticalTabsViewMode` enum in this ticket. It already means compact vs expanded density in code, and renaming it would create avoidable churn across unrelated logic. The popup can relabel that control to `Density` without touching the backing enum name yet.

### 2. Add a workspace action for the new setting

Add `WorkspaceAction::SetVerticalTabsDisplayGranularity(VerticalTabsDisplayGranularity)` beside the existing vertical-tabs setting actions.

Handle it in `Workspace::handle_action` exactly like the other setting writes:

- read the enum payload
- call `settings.vertical_tabs_display_granularity.set_value(...)`
- `ctx.notify()`

Update `should_save_app_state_on_action` so this action returns `false`, and add a matching unit test in `action_tests.rs`.

### 3. Extend popup-local state for the new segmented control

Add two `MouseStateHandle`s to `VerticalTabsPanelState` for the new control:

- one for the `Panes` segment
- one for the `Tabs` segment

Keep the existing `compact_segment_mouse_state` / `expanded_segment_mouse_state` fields unchanged so the density control continues to work without refactoring unrelated code.

### 4. Restructure `render_settings_popup`

Update `render_settings_popup` so the popup composition becomes:

1. `View as` header
2. text segmented control for `Panes` / `Tabs`
3. divider
4. `Density` header
5. existing compact / expanded icon segmented control
6. existing pane-row display sections (`Pane title as`, `Additional metadata`, `Show`) in their current conditional behavior

Implementation details:

- add a new helper for a text-labeled segment control rather than forcing the existing icon helper to serve both roles
- keep the popup width fixed unless the new control proves to clip; the current 200px width is likely enough for `Panes` / `Tabs`
- leave the popup open after clicking `Panes` or `Tabs`, matching the existing “update in place” behavior of the other controls
- do not hide `Pane title as`, `Additional metadata`, or `Show` when `Tabs` is selected; the representative row still uses pane-row rendering, so those controls continue to apply in both granularities
- keep the existing conditional logic that is already based on density (`Additional metadata` only in compact, `Show` only in expanded)

The current compact / expanded segmented control should keep writing `VerticalTabsViewMode`; only the user-facing label changes to `Density`.

### 5. Centralize pane-id selection by granularity

Add a small helper in `vertical_tabs.rs` that decides which pane ids a tab should expose for rendering and search:

```rust path=null start=null
fn pane_ids_for_display_granularity(
    visible_pane_ids: &[PaneId],
    focused_pane_id: PaneId,
    granularity: VerticalTabsDisplayGranularity,
) -> Vec<PaneId>
```

Behavior:

- `Panes` returns all visible pane ids in existing order
- `Tabs` returns exactly one pane id:
  - `focused_pane_id` if it is present in `visible_pane_ids`
  - otherwise the first visible pane as a defensive fallback
  - otherwise an empty vec if the tab has no visible panes

This helper should be pure and small enough to unit test in `vertical_tabs_tests.rs`.

### 6. Use the helper in both render and search paths

Read the new setting once in each relevant call path and replace direct iteration of `visible_pane_ids()` with `pane_ids_for_display_granularity(...)`.

Affected paths:

- `matching_tab_indices`
- the search branch inside `render_groups`
- the row-building path inside `render_tab_group`

This keeps the meaning of `Tabs` mode consistent everywhere:

- a tab renders only its representative row
- search only considers that representative row
- tabs hidden by search are determined by the same representative row

This is the most important structural change in the ticket. It is also intentionally narrow: the existing row building stays pane-based, but the set of pane ids fed into it changes.

### 7. Keep `PaneProps` and row renderers unchanged

Do not introduce a new “tab row” prop type in this ticket.

Instead, continue to build a normal `PaneProps` from the chosen representative `PaneId` and reuse:

- `render_pane_row`
- `render_compact_pane_row`
- `render_pane_row_element`

This preserves:

- row click behavior (`FocusPane`)
- existing metadata and badge rules
- compact / expanded density behavior
- selection and hover styling
- future compatibility with any pane-row improvements already in flight on this branch

### 8. Use `focused_pane_id()` as the representative source of truth

In `render_tab_group` and search helpers, compute the representative pane from:

- `pane_group.visible_pane_ids()`
- `pane_group.focused_pane_id(app)`

Do not use `active_session_id()`:

- it is terminal-only
- it would fail for code / notebook / workflow tabs

Using `focused_pane_id()` also ensures the representative row updates automatically when focus changes within a split tab, because that state is already maintained by `PaneGroup::focus_pane`.

### 9. Keep header and action-belt behavior untouched

`render_tab_group` currently owns more than just the rows: it also owns the group container, optional custom-title header, hover background, and overlay action belt. This ticket should not fork that structure for Tabs mode.

Only the row list inside the body changes. Everything else in `render_tab_group` stays as-is.

That matches the product scope and lowers regression risk around rename, tab actions, and drag behavior.

## End-to-end flow

1. The user opens the vertical-tabs popup from the settings icon.
2. `render_settings_popup` reads `vertical_tabs_display_granularity` and shows `Panes` selected by default.
3. The user clicks `Tabs`.
4. `WorkspaceAction::SetVerticalTabsDisplayGranularity(Tabs)` is dispatched.
5. `Workspace::handle_action` writes the synced setting through `TabSettings` and calls `ctx.notify()`.
6. On re-render, `render_groups` and `render_tab_group` both read the new setting.
7. For each tab:
   - the code gets `visible_pane_ids()`
   - gets `focused_pane_id()`
   - runs `pane_ids_for_display_granularity(...)`
   - receives either all panes (`Panes`) or exactly one representative pane (`Tabs`)
8. That representative pane is passed through `PaneProps::new` and then through the existing compact / expanded row renderer.
9. Clicking the representative row still dispatches `FocusPane`, which activates the tab and focuses that pane.
10. If the user changes focus within a split tab, `PaneGroup::focus_pane` updates `focused_pane_id()`, and the next render shows a different representative row automatically.

## Risks and mitigations

### Ambiguity between existing `VerticalTabsViewMode` and the new product “View as”

Risk:

- the current code uses `VerticalTabsViewMode` to mean density, while the product copy now uses `View as` to mean pane-vs-tab granularity

Mitigation:

- introduce a separately named enum (`VerticalTabsDisplayGranularity`) instead of overloading `VerticalTabsViewMode`
- relabel the existing control in UI only

### Stale focused pane not present in visible panes

Risk:

- during close / restore edge cases, `focused_pane_id()` might not be present in the visible pane list momentarily

Mitigation:

- `pane_ids_for_display_granularity(...)` falls back to the first visible pane
- the helper returns an empty vec only when the tab truly has no visible panes

### Search/render drift

Risk:

- Tabs mode could render one pane but still search across all panes if the two code paths diverge

Mitigation:

- use the same granularity helper in `matching_tab_indices`, `render_groups`, and `render_tab_group`
- keep pane-id selection in one place

### Popup churn beyond scope

Risk:

- the Figma exploration also shows Tabs-only secondary controls, which could tempt additional conditional popup logic

Mitigation:

- explicitly keep this ticket to the top-level `View as` control only
- do not add a Tabs-only `Default name` or `Summary` section in this implementation
- do not hide the existing pane-row controls when `Tabs` is selected; they remain relevant because the representative row is still a pane row

## Testing and validation

### Unit tests

In `app/src/workspace/view/vertical_tabs_tests.rs`:

- add tests for `pane_ids_for_display_granularity(...)`
  - `Panes` returns all visible panes in order
  - `Tabs` returns the focused pane when present
  - `Tabs` falls back to the first visible pane when the focused pane is absent
  - empty visible list returns empty

In `app/src/workspace/action_tests.rs`:

- add a test that `SetVerticalTabsDisplayGranularity(...)` does not save workspace state

### Manual validation

- verify popup layout now shows `View as` first and `Density` above the existing compact / expanded toggle
- verify `Panes` remains default and current behavior is unchanged
- verify a multi-pane tab shows one representative row in `Tabs` mode
- verify switching focus within a split tab changes the representative row
- verify compact and expanded densities both work in `Tabs` mode
- verify existing `Pane title as`, `Additional metadata`, and `Show` preferences remain visible in the popup in `Tabs` mode and still affect the representative row as expected
- verify search in `Tabs` mode only matches the representative row, not hidden non-active panes
- verify the setting persists across relaunch

## Follow-ups

- Rename `VerticalTabsViewMode` to something density-specific in code if we want terminology to match the product UI more closely. This is not necessary for APP-3828.
- Add the future Tabs-only naming control (`Focused session` vs `Summary`) in a separate ticket once product behavior is finalized. At that point, pane-row-specific controls can become conditional on the focused-session path rather than always being shown for `Tabs`.
