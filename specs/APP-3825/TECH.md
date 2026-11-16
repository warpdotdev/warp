# APP-3825: Tech Spec — Vertical Tabs Pane Drag Parity

## Problem

The pane-drag pipeline already supports moving a pane onto the horizontal tab strip and then reusing the existing in-tab relayout flow inside the destination tab. Vertical tabs do not participate in that pipeline today, even though the downstream workspace and pane-group logic is already generic once it receives a `TabBarHoverIndex`.

The missing piece is the vertical tabs panel itself:

- it renders tab groups as draggable items for tab reordering,
- but it does not expose any drop targets for pane-header drags,
- it does not render insertion indicators from `Workspace.hovered_tab_index`,
- and its hover-only action button belt can overlap the tab-group surface during a drag.

As a result, pane-header drags over vertical tabs fall through to `PaneDragDropLocation::Other` instead of producing the existing `OverTab` / `BeforeTab` flow.

## Relevant code

- `app/src/workspace/view/vertical_tabs.rs (710-929)` — vertical tabs panel rendering; `render_vertical_tabs_panel`, `render_groups`, `render_tab_group`
- `app/src/workspace/view/vertical_tabs.rs (968-1083)` — each vertical tab group is currently only a `Draggable` + `SavePosition` for reordering
- `app/src/workspace/view.rs (15291-15328)` — horizontal tab bar wrapper adds a workspace-level `DropTarget` with `TabBarDropTargetData { AfterTabIndex(..) }`
- `app/src/tab.rs (1366-1382)` — each horizontal tab is wrapped in `DropTarget::new(..., TabBarDropTargetData { TabIndex(..) })`
- `app/src/pane_group/pane/view/header/mod.rs (321-352)` — pane-header drag hover classification currently derives `TabBarHoverIndex` from `TabBarLocation` + geometry
- `app/src/pane_group/pane/view/header/mod.rs (939-1088)` — `render_pane_header_draggable`; pane drags only accept `PaneDropTargetData` and `TabBarDropTargetData`
- `app/src/pane_group/mod.rs (544-561)` — workspace-facing events: `DroppedOnTabBar`, `SwitchTabFocusAndMovePane`, `UpdateHoveredTabIndex`, `ClearHoveredTabIndex`
- `app/src/pane_group/mod.rs (1098-1153)` — `handle_pane_view_event`; `OverTab` hides/moves panes into the target tab, `BeforeTab` preserves “new tab” behavior
- `app/src/workspace/view.rs (11630-11889)` — workspace handling for cross-tab drag/drop and existing special cases like code-pane merge behavior
- `app/src/workspace/mod.rs (1447-1461)` — `TabBarDropTargetData` and `TabBarLocation`
- `app/src/workspace/view.rs (14883-14891)` — existing horizontal insertion indicator renderer

## Current state

### Horizontal tabs already provide the full drag channel

The horizontal tab strip exposes `TabBarDropTargetData` in two places:

- each concrete tab via `TabBarLocation::TabIndex(i)`, and
- the strip container via `TabBarLocation::AfterTabIndex(tab_count)`.

Pane-header drags and editor-tab drags both recognize that data. In the pane-header path, `PaneHeader::calculate_tab_focus_hover_index` interprets the dragged rect relative to the target tab’s bounds and converts it into:

- `TabBarHoverIndex::OverTab(i)` when the user is targeting an existing tab
- `TabBarHoverIndex::BeforeTab(i)` when the user is inserting between tabs

From there, the rest of the flow is already shared:

- `PaneGroup::handle_pane_view_event` emits `SwitchTabFocusAndMovePane` for `OverTab` and hides the dragged pane for `BeforeTab`
- `Workspace` stores `hovered_tab_index`, switches active tabs when needed, and on drop either
  - adds a new tab from the moved pane for `BeforeTab`, or
  - reuses the target tab’s existing placement rules for `OverTab`

### Vertical tabs only support tab reordering today

`render_tab_group` in `vertical_tabs.rs` currently builds each tab group as:

- a hoverable group surface,
- an optional overlay belt with kebab/close actions,
- a `Draggable` used for vertical tab reordering,
- and a `SavePosition(tab_position_id(tab_index))` used by the reorder math in `Workspace::calculate_updated_tab_index_vertical`.

There is no `DropTarget` around the group and no dedicated insertion targets between groups or after the final group.

Because of that, dragging a pane header over the vertical tabs panel never produces a workspace-tab target. The drag is not accepted by the panel, so the pane-header code falls back to `PaneDragDropLocation::Other`.

### `hovered_tab_index` is only rendered in the horizontal tab strip

`Workspace.hovered_tab_index` already drives two pieces of feedback in the horizontal strip:

- highlighted tab styling for `OverTab`
- insertion bars for `BeforeTab`

The vertical tabs panel does not currently read or render that state at all.

### Overlay controls can interfere with tab-group hit-testing

The vertical tabs group action belt is shown from hover state and rendered as a positioned overlay on top of the group. During a pane drag, that overlay can visually and spatially compete with the tab-group surface unless we explicitly suppress it or ensure the drop target sits above it in hit-testing.

## Proposed changes

### 1. Add explicit vertical-tabs pane drop targets

Introduce a new workspace-level drop target data type for pane-header drags in vertical tabs. Keep it separate from `TabBarDropTargetData` so this ticket stays scoped to pane headers and does not implicitly expand to editor file-tab dragging.

Proposed shape in `app/src/workspace/mod.rs`:

```rust
#[derive(PartialEq, Copy, Clone, Debug)]
pub struct VerticalTabsPaneDropTargetData {
    pub tab_bar_location: TabBarLocation,
    pub tab_hover_index: TabBarHoverIndex,
}
```

This reuses the existing `TabBarLocation` / `TabBarHoverIndex` semantics instead of inventing a parallel enum.

### 2. Teach pane-header draggables to accept the new vertical target data

Update `render_pane_header_draggable` in `app/src/pane_group/pane/view/header/mod.rs` to accept three drop-target kinds:

- `PaneDropTargetData`
- `TabBarDropTargetData` (existing horizontal strip path)
- `VerticalTabsPaneDropTargetData` (new vertical tabs path)

To keep the existing horizontal logic intact, extend the pane-header drag action so it can optionally carry a precomputed `TabBarHoverIndex`:

```rust
PaneHeaderDragged {
    origin: ActionOrigin,
    drag_location: PaneDragDropLocation,
    drag_position: RectF,
    explicit_tab_hover_index: Option<TabBarHoverIndex>,
}
```

Behavior:

- horizontal tab-strip drags keep sending `None` and continue to use `calculate_tab_focus_hover_index`
- vertical tabs send `Some(...)` and bypass geometry inference

That lets vertical tabs use explicit “over tab” vs “between tabs” zones instead of trying to reinterpret a large tab-group card with the horizontal strip’s x-axis heuristics.

### 3. Wrap vertical tab groups in `OverTab` drop targets

In `render_groups` / `render_tab_group`:

- keep the current `Draggable + SavePosition` structure for tab reordering
- wrap each rendered tab-group element in `DropTarget::new(..., VerticalTabsPaneDropTargetData { ... OverTab(tab_index) ... })`

The `SavePosition(tab_position_id(tab_index))` should remain attached to the group element used for tab reordering, not to any insertion spacer. That preserves the current reorder math in `calculate_updated_tab_index_vertical`.

This gives the entire visible tab-group surface an explicit “existing tab” meaning, which matches the product spec for both compact and expanded modes.

### 4. Add explicit insertion targets between groups and after the final group

Render small, dedicated insertion targets in the vertical list rather than deriving `BeforeTab` from pointer position inside a tab-group card.

Concretely, `render_groups` should render:

- a leading insertion target for `BeforeTab(0)` for full parity with the horizontal strip
- one insertion target before each subsequent tab group
- one trailing insertion target after the final group for `BeforeTab(tab_count)`

Each insertion target is a thin block wrapped in `DropTarget::new(..., VerticalTabsPaneDropTargetData { tab_hover_index: BeforeTab(i), ... })`.

This has two advantages:

- it matches the product behavior more closely than splitting a tab-group card into vertical thirds
- it avoids rewriting the shared pane-group/workspace drag logic, since that logic already understands `BeforeTab`

The trailing target should have enough height to remain practically hittable even when the list fills the panel.

### 5. Render vertical drag feedback from `Workspace.hovered_tab_index`

Add vertical-tabs equivalents of the horizontal tab-strip feedback in `vertical_tabs.rs`:

- when `hovered_tab_index == OverTab(i)`, the corresponding tab group renders as the active drag target
- when `hovered_tab_index == BeforeTab(i)`, render an insertion indicator before that group
- when `hovered_tab_index == BeforeTab(tab_count)`, render the insertion indicator at the end of the list

Implementation-wise:

- thread `workspace.hovered_tab_index` into `render_groups` / `render_tab_group`
- add a helper like `render_vertical_tab_hover_indicator`
- include drag-target highlighting in the group background logic, separate from normal hover and active-tab styling

The current horizontal `render_tab_hover_indicator` is a narrow vertical bar. Vertical tabs should use a horizontal divider-style accent that visually reads as “insert here” in the list layout.

### 6. Suppress the vertical tab-group action belt during pane drags

While any pane is being dragged, do not render the floating kebab/close action belt in `render_tab_group`.

This is the simplest way to avoid overlay interference with drag targeting and keeps the panel visually focused on drop feedback instead of hover controls.

Use the existing `PaneGroup::any_pane_being_dragged(app)` signal from the active pane group, similar to how the workspace already uses it for tab-bar visibility and keymap context.

### 7. Leave pane-group and workspace drop behavior unchanged

Do not redesign the downstream move/drop handlers. The current event chain is already the right abstraction boundary:

- `PaneGroup::handle_pane_view_event`
- `Workspace`’s `SwitchTabFocusAndMovePane`
- `Workspace`’s `DroppedOnTabBar`

That code already preserves:

- hidden-pane staging while hovering `OverTab`
- “promote to new tab” behavior for `BeforeTab`
- existing target-tab placement rules
- special cases like code-pane merge behavior under tabbed editor view

This ticket should only change how vertical tabs produce `TabBarHoverIndex`, not what happens after that.

## End-to-end flow

1. The user starts dragging a pane header.
2. `render_pane_header_draggable` enters drag mode and accepts either pane drop targets, horizontal tab-strip targets, or the new vertical-tabs pane targets.
3. The user moves over the vertical tabs panel:
   - over a tab group → the drop target provides `OverTab(i)`
   - over an insertion zone → the drop target provides `BeforeTab(i)`
4. The pane-header view emits `DraggedOverTabBar` with that `TabBarHoverIndex`.
5. `PaneGroup::handle_pane_view_event` reuses the existing behavior:
   - `OverTab(i)` → stage the pane in the destination tab via `SwitchTabFocusAndMovePane`
   - `BeforeTab(i)` → hide the pane for move and preserve “create a new tab on drop”
6. `Workspace` updates `hovered_tab_index`, which now drives vertical-panel drag feedback.
7. If the target is `OverTab(i)` and a tab switch is needed, `Workspace` activates the target tab and adds the pane as hidden in that tab’s pane group.
8. The user moves into the workspace content area; the existing pane relayout drop targets appear and the pane can be placed using the current within-tab logic.
9. On drop:
   - `BeforeTab(i)` → existing `DroppedOnTabBar` logic creates a new tab at `i`
   - `OverTab(i)` → existing target-tab placement logic runs unchanged
10. Cancelling or leaving valid targets clears `hovered_tab_index` and reverts hidden-pane staging, as it does today.

## Risks and mitigations

### Risk: accidental scope expansion to editor file-tab dragging

If we reused `TabBarDropTargetData` directly in vertical tabs, editor file tabs would likely start targeting vertical tabs too, because `code/view.rs` already recognizes that type.

Mitigation:

- use a separate `VerticalTabsPaneDropTargetData`
- only add it to the pane-header draggable acceptance path

### Risk: overlay hit-testing blocks the drop target

The action button belt is rendered as an overlay on the group and can interfere with drag targeting.

Mitigation:

- suppress the overlay while any pane drag is active

### Risk: breaking vertical tab reordering

Adding wrappers around the group could change the bounds used by `calculate_updated_tab_index_vertical`.

Mitigation:

- keep `SavePosition(tab_position_id(..))` attached to the group element itself
- do not include insertion-target spacers in that saved position

### Risk: feedback mismatch between target state and actual drop behavior

If the vertical panel renders `OverTab` / `BeforeTab` differently from what the workspace eventually does, the interaction will feel inconsistent.

Mitigation:

- reuse `TabBarHoverIndex` end to end
- keep workspace and pane-group drop handlers unchanged

## Testing and validation

### Manual validation

Use the scenarios in `specs/APP-3825/PRODUCT.md`:

- drag a pane from one tab to another in expanded mode
- repeat in compact mode
- verify tabs without custom headers are targetable
- verify insertion between groups and after the last group creates a new tab
- verify cancel / drop-outside clears temporary state
- verify code-pane merge behavior still matches the horizontal flow
- verify vertical tab reordering still works

### Automated validation

This feature is mostly WarpUI drag/drop hit-testing, so manual validation is the primary check. Add lightweight automated coverage where it is cheap and reliable:

- if a pure helper is introduced for vertical drag-target rendering decisions, cover `OverTab` / `BeforeTab` cases in `app/src/workspace/view/vertical_tabs_tests.rs`
- add or update workspace tests only if the new helper can be exercised without full UI drag simulation

No new persistence, networking, or model migration coverage is needed.

## Follow-ups

- If we later want vertical tabs to support editor file-tab dragging as well, we can either:
  - teach `code/view.rs` to accept `VerticalTabsPaneDropTargetData`, or
  - unify the horizontal and vertical tab-target metadata behind a shared explicit-hover-index type
- If the vertical insertion indicator and the horizontal tab-strip indicator should share styling, we can extract a small shared helper after this behavior is stable
