# APP-3832: Tech Spec — Vertical Tabs Hover Detail Sidecar

## Problem

APP-3832 adds a hover-only detail sidecar to the vertical tabs panel.

The product behavior is intentionally specific:

- in `View as = Panes`, hovering a supported pane row shows one pane-scoped detail card
- in `View as = Tabs`, hovering a supported representative row shows a tab-scoped sidecar with one pane section per visible pane in that tab
- the sidecar is floating, does not change focus, stays open while the pointer moves from the row into the sidecar, and becomes internally scrollable when tall
- v1 only supports terminal / agent terminal panes and code panes

Technically, the current vertical tabs implementation has the row hover primitives we need, but it has no concept of:

- a row-anchored overlay other than the settings popup / tab menus
- ephemeral hover-detail state that persists across the gap between a row and a sidecar
- a fixed-detail renderer that is independent of the existing row display settings
- tabs-mode eligibility rules that depend on the full set of visible panes in a tab

The implementation should add that behavior without disturbing the existing row renderers, click behavior, or synced settings flow.

## Relevant code

- `specs/APP-3832/PRODUCT.md` — agreed user-facing behavior for the hover detail sidecar
- `specs/APP-3828/PRODUCT.md` — current `View as = Panes / Tabs` behavior that determines row granularity
- `specs/APP-3828/TECH.md` — existing implementation pattern for vertical-tabs display granularity
- `app/src/workspace/view/vertical_tabs.rs (246-344)` — `VerticalTabsPanelState`, row mouse-state ownership, and other panel-local UI state
- `app/src/workspace/view/vertical_tabs.rs (458-557)` — `matching_tab_indices`; useful context for the current pane-vs-tab granularity flow
- `app/src/workspace/view/vertical_tabs.rs (630-1045)` — `render_groups` and `render_tab_group`, where row lists are built for both `Panes` and `Tabs`
- `app/src/workspace/view/vertical_tabs.rs (1020-1277)` — `render_pane_row_element`, the current row hover / click wrapper
- `app/src/workspace/view/vertical_tabs.rs (1320-1710)` — `TypedPane`, `PaneProps::new`, code-pane title/path derivation, and badge helpers
- `app/src/workspace/view/vertical_tabs.rs (1858-2340)` — terminal metadata rendering helpers and badge interactions that the sidecar should reuse semantically
- `app/src/workspace/view.rs (17784-17839)` — workspace action handling for vertical-tabs settings
- `app/src/workspace/view.rs (19201-19399)` — `Workspace::render`, which already hosts workspace-root overlays like the settings popup and tab menus
- `app/src/safe_triangle.rs` — generic safe-triangle logic for keeping hover sidecars stable during diagonal cursor movement
- `app/src/menu.rs (1852-1860, 2171-2367)` — current safe-triangle integration for menus and submenus
- `app/src/terminal/profile_model_selector.rs (1040-1078)` — concrete pattern for reading a sidecar rect from the previous frame and feeding it into safe-triangle state
- `app/src/workspace/view/vertical_tabs_tests.rs` — current home for pure vertical-tabs helper tests

## Current state

### Panel-local state

`VerticalTabsPanelState` already owns:

- the panel scroll and resize handles
- per-tab-group hover state
- per-pane-row hover state
- per-pane badge hover state
- settings-popup hover state and popup visibility

It does not currently track:

- which row is the current detail-sidecar source
- whether a sidecar itself is hovered
- a scroll state for a sidecar
- safe-triangle state for row-to-sidecar transitions

### Row rendering and granularity

`render_tab_group` already centralizes the pane ids that become visible rows:

- `Panes` mode renders all visible pane ids
- `Tabs` mode renders one representative pane id via `pane_ids_for_display_granularity(...)`

Each rendered row already has a stable `MouseStateHandle`, but rows are not wrapped in `SavePosition`, so there is no stable anchor id for a row-relative overlay.

### Overlay placement

The vertical tabs panel itself is rendered inside `Workspace::render_panels`, but floating overlays that must escape local panel bounds are rendered from the workspace root `Stack` in `Workspace::render`. This is how the vertical-tabs settings popup and tab context menus are currently placed.

That makes the workspace root the right place to render the hover detail sidecar as well.

### Existing row data extraction

The current row renderers already know how to derive most of the underlying metadata we need:

- terminal title / conversation title fallback logic
- working directory and git branch
- diff-stats and PR badge data
- code-pane filename / parent-path split
- code dirty state
- pane kind labels and icons

However, the row renderers are the wrong level of reuse for the sidecar itself because:

- they are intentionally clipped and density-dependent
- they are coupled to `Pane title as`, `Additional metadata`, and `Show`
- the sidecar layout is fixed and should not rearrange around those settings

The sidecar should therefore reuse the same data sources, not the same element tree.

### Safe-triangle precedent

The repo already has a safe-triangle implementation for hover menus that open sidecars or submenus. The existing pattern is:

- keep ephemeral hover state outside of synced settings / workspace snapshotting
- record the sidecar rect from the previous frame
- suppress intermediate hover changes while the cursor moves diagonally toward that rect

That is the right interaction primitive for this feature.

## Proposed changes

### 1. Add panel-local detail-overlay state

Extend `VerticalTabsPanelState` with state for the hover detail overlay:

- `detail_scroll_state: ClippedScrollStateHandle` — internal scrolling for tall sidecars
- `detail_sidecar_mouse_state: MouseStateHandle` — hover tracking for the sidecar itself
- `detail_overlay_state: Arc<Mutex<VerticalTabsDetailOverlayState>>` — ephemeral hover-detail state shared by row and sidecar callbacks

Introduce a small internal state type in `vertical_tabs.rs`:

```rust path=null start=null
struct VerticalTabsDetailOverlayState {
    active_target: Option<VerticalTabsDetailTarget>,
    safe_triangle: SafeTriangle,
}

enum VerticalTabsDetailTarget {
    Pane {
        pane_group_id: EntityId,
        pane_id: PaneId,
    },
    Tab {
        pane_group_id: EntityId,
        source_pane_id: PaneId,
    },
}
```

Use panel-local ephemeral state rather than new synced settings or persisted workspace actions. The sidecar is transient UI state, more like menu hover than workspace configuration.

### 2. Add helpers for detail-sidecar eligibility and anchoring

Add pure helpers in `vertical_tabs.rs` for:

- deciding whether a `TypedPane` is supported by the v1 sidecar
- converting a hovered rendered row into a `VerticalTabsDetailTarget`
- resolving the pane ids that a target should render as sidecar sections
- producing a stable save-position id for a rendered row

Suggested helper shape:

```rust path=null start=null
fn vtab_pane_row_position_id(pane_group_id: EntityId, pane_id: PaneId) -> String

fn supports_vertical_tabs_detail_sidecar(typed: &TypedPane<'_>) -> bool

fn detail_target_for_hovered_row(
    pane_group_id: EntityId,
    pane_id: PaneId,
    granularity: VerticalTabsDisplayGranularity,
) -> VerticalTabsDetailTarget

fn pane_ids_for_detail_target(
    pane_group: &PaneGroup,
    target: &VerticalTabsDetailTarget,
    app: &AppContext,
) -> Option<Vec<PaneId>>
```

Behavior:

- `Panes` mode:
  - supported pane row => pane-scoped target
  - unsupported pane row => no sidecar
- `Tabs` mode:
  - representative row => tab-scoped target
  - resolve all visible panes in the hovered tab
  - if any visible pane is unsupported, return `None` so the whole tab has no sidecar in v1

This keeps the mixed-tab gating rule centralized and testable.

### 3. Save row positions and attach hover callbacks at the row wrapper

Update `render_pane_row_element` so the final row element is wrapped in `SavePosition` using the new row-position helper.

Add a hover callback there rather than in individual compact / expanded row renderers. That keeps the hover-detail behavior shared across all row variants.

The row wrapper should:

- derive the appropriate `VerticalTabsDetailTarget` for that row
- update `detail_overlay_state.active_target` on supported hover-in
- ignore unsupported rows
- update safe-triangle state with the current pointer position
- use `with_skip_synthetic_hover_out()` so overlay insertion does not immediately force a synthetic close

This approach preserves the current row click / double-click behavior and avoids duplicating hover-detail wiring in multiple render paths.

### 4. Render the sidecar from `Workspace::render`, not inside the panel surface

Add a new helper in `vertical_tabs.rs`:

```rust path=null start=null
pub(super) fn render_detail_sidecar(
    state: &VerticalTabsPanelState,
    workspace: &Workspace,
    app: &AppContext,
) -> Option<(String, Box<dyn Element>)>
```

This helper should:

- read `detail_overlay_state.active_target`
- validate that the referenced pane group / pane(s) still exist
- resolve the pane ids that should become sections
- build the sidecar element if the target is still valid
- return the source row position id plus the sidecar element

Render the returned sidecar from the workspace-root `Stack` in `Workspace::render`, alongside the existing settings popup and tab menus. This avoids clipping by the vertical-tabs panel container and matches the existing overlay pattern in this codepath.

Position the sidecar with `OffsetPositioning::offset_from_save_position_element(...)` using the hovered row’s save-position id and `PositionedElementOffsetBounds::WindowByPosition`.

Also give the sidecar its own `SavePosition` id so its rect can be read on subsequent frames for the safe triangle.

### 5. Build a dedicated detail renderer on top of existing metadata sources

Add a dedicated sidecar renderer instead of trying to stretch the existing row renderers.

Introduce a small data model for the detail content:

```rust path=null start=null
enum VerticalTabsDetailSectionData {
    Terminal(TerminalDetailSectionData),
    Code(CodeDetailSectionData),
}
```

Back it with helper builders that reuse the same underlying sources as the row code:

- terminal title / conversation fallback logic from the existing terminal helpers
- `TerminalView` for working directory, branch, diff stats, PR link, and agent status
- `PaneProps::new` / `TypedPane` for pane kind labels and code-pane title/path derivation
- `TypedPane::badge(app)` / `CodePane` dirty checks for unsaved state

Do not make the sidecar renderer depend on:

- `VerticalTabsViewMode`
- `VerticalTabsPrimaryInfo`
- `VerticalTabsCompactSubtitle`
- `vertical_tabs_show_*`

Those settings shape rows; the sidecar is a fixed detail view.

### 6. Sidecar layout and scrolling

Render the sidecar as:

- fixed-width outer card
- bounded-height container
- internal scrollable content using `ClippedScrollable::vertical(...)` or `NewScrollable::vertical(...)`
- overlayed scrollbar styling consistent with existing panel/menu scrollables

In `Tabs` mode:

- render one section per resolved pane id
- insert dividers between sections

In `Panes` mode:

- render a single section with no internal divider treatment

Use the workspace window bounds to keep the sidecar on-screen. `WindowByPosition` plus a bounded max height is sufficient for v1; there is no need for a separate left/right flip behavior because the vertical tabs panel already lives on the left side of the workspace.

### 7. Keep the sidecar open across row-to-sidecar cursor movement

Reuse the existing safe-triangle pattern rather than inventing a new hover heuristic.

Implementation approach:

- the sidecar root is wrapped in `Hoverable::new(detail_sidecar_mouse_state, ...)`
- on row hover changes, update the `SafeTriangle` with the latest pointer position
- on subsequent hover events, suppress replacing or clearing the active target while the cursor is moving through the safe triangle toward the current sidecar
- like `ProfileModelSelector`, read the sidecar rect from the previous frame and feed it back into `SafeTriangle::set_target_rect(...)`

This keeps the interaction logic close to the existing menu/sidecar behavior already used elsewhere in the app.

### 8. Do not add new workspace actions unless implementation proves they are needed

The initial design should keep the hover-detail state entirely inside `VerticalTabsPanelState` via shared ephemeral state handles.

That avoids:

- new `WorkspaceAction` variants for non-persistent hover state
- extra `handle_action` branches in `Workspace::handle_action`
- confusion about whether the sidecar is part of saved workspace state

If the actual UI framework constraints force action-based updates later, that can be introduced during implementation, but it should not be the default plan.

## End-to-end flow

1. The user hovers a rendered vertical-tabs row.
2. The row wrapper in `render_pane_row_element` derives a `VerticalTabsDetailTarget` from:
   - the row’s pane id
   - the row’s pane group id
   - the current `VerticalTabsDisplayGranularity`
3. If the hovered row is unsupported, nothing happens.
4. If the row is supported, the row hover callback stores the target in `detail_overlay_state.active_target`.
5. On the next render, `Workspace::render` asks `render_detail_sidecar(...)` for an overlay.
6. That helper:
   - validates the target
   - resolves the pane ids to show
   - rejects mixed-support tabs in `Tabs` mode
   - builds the sidecar sections from the current pane data
7. The workspace root `Stack` positions the sidecar to the right of the hovered row using the row’s save-position id.
8. The user moves the pointer from the row into the sidecar.
9. The safe triangle suppresses intermediate hover changes, so the sidecar stays open.
10. If the sidecar becomes tall, its content scrolls internally.
11. If the pointer leaves both the source row and the sidecar, the overlay state is cleared and the sidecar disappears.

## Risks and mitigations

### Overlay-induced synthetic hover churn

Risk:

- inserting a floating overlay can trigger synthetic hover-out events on the source row, causing flicker

Mitigation:

- attach hover handling at the row wrapper
- use `with_skip_synthetic_hover_out()`
- keep the last active target in ephemeral overlay state instead of deriving visibility strictly from the current row hover bit

### Mixed-support tabs in `Tabs` mode

Risk:

- the product rule for mixed tabs is easy to accidentally implement as “render only supported panes”

Mitigation:

- centralize the rule in `pane_ids_for_detail_target(...)`
- make the helper return `None` for any tab that includes unsupported visible panes
- add explicit tests for mixed tabs

### Stale hover target after close / focus / reorder changes

Risk:

- the hovered source pane or tab may disappear while the sidecar is open

Mitigation:

- validate the target on every render before building the sidecar
- clear the target if the pane group or source pane no longer exists
- resolve tab sections from live `visible_pane_ids()` each render rather than caching pane lists in the target

### Duplication of terminal / code metadata logic

Risk:

- the sidecar could fork the row metadata logic and drift over time

Mitigation:

- reuse existing helper functions and model reads wherever possible
- keep new data-extraction helpers in `vertical_tabs.rs`, close to the row rendering code they parallel
- avoid re-encoding fallback rules in multiple places

### Root-overlay vs panel-overlay confusion

Risk:

- `render_vertical_tabs_panel` already contains local overlay structure for the settings popup, while `Workspace::render` also renders the popup globally

Mitigation:

- treat `Workspace::render` as the source of truth for the new sidecar overlay
- keep the sidecar rooted at the workspace stack from the start
- do not introduce another panel-local overlay path for this feature

## Testing and validation

### Unit tests

Add pure tests in `app/src/workspace/view/vertical_tabs_tests.rs` for:

- `supports_vertical_tabs_detail_sidecar(...)`
  - terminal panes supported
  - code panes supported
  - unsupported pane types rejected
- `pane_ids_for_detail_target(...)`
  - panes mode returns just the hovered pane
  - tabs mode returns all visible panes when every pane is supported
  - tabs mode returns `None` when any visible pane is unsupported
  - stale / missing source pane returns `None`

If needed, add small pure tests around row-position-id helpers or target resolution helpers, but prioritize the eligibility logic above.

### Manual validation

- hover a supported terminal row in `Panes` mode and verify a single pane-scoped sidecar appears
- hover a supported code row in `Panes` mode and verify a single code sidecar appears
- hover a representative row in `Tabs` mode for a tab with several supported panes and verify one section per visible pane
- hover a representative row in `Tabs` mode for a mixed-support tab and verify no sidecar appears
- move the cursor diagonally from the row into the sidecar and verify it does not flicker closed
- click diff-stats and PR badges inside the sidecar and verify the existing actions still fire
- hover a large multi-pane tab and verify the sidecar becomes internally scrollable rather than extending off-screen
- switch `Density`, `Pane title as`, `Additional metadata`, and `Show`, then verify the sidecar layout stays fixed
- close or mutate the hovered tab while the sidecar is open and verify the overlay disappears cleanly without stale content

## Follow-ups

- Support additional pane types once product behavior is defined for their detail sections.
- Add keyboard-accessible detail affordances in a follow-up ticket if the hover-only behavior proves valuable.
- If more hover sidecars are added elsewhere in Warp, consider extracting a reusable “row + sidecar + safe triangle” helper instead of keeping the logic local to vertical tabs.
