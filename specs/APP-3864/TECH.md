# APP-3864: Tech Spec — Vertical Tabs Detail Sidecar Stale Visibility Fix

## Problem

The vertical tabs detail sidecar can remain visible after the pointer has effectively left both the source row and the sidecar.

One reproducible path is:

1. Hover a pane row so the detail sidecar appears.
2. Move into the sidecar scrollbar gutter.
3. Move vertically within the gutter.
4. Move farther right so the pointer leaves the sidecar and vertical tabs panel.

Observed behavior:

- the sidecar stays visible
- visibility is not corrected until a later mouse event re-enters the vertical tabs area

Expected behavior:

- the sidecar should hide as soon as the pointer is no longer over the source row, over the sidecar, or in the safe-triangle transit path between them

## Relevant code

- `app/src/workspace/view/vertical_tabs.rs:377` — `render_pane_row_element`, where row hover updates `detail_overlay_state.active_target`
- `app/src/workspace/view/vertical_tabs.rs:624` — `VerticalTabsDetailHoverState` and panel-local detail-sidecar state
- `app/src/workspace/view/vertical_tabs.rs:4189` — `render_detail_sidecar`, which renders the overlay from `active_target`
- `app/src/workspace/view.rs:19774` — workspace-root rendering path that hosts the detail sidecar overlay
- `app/src/workspace/view.rs:20406` — workspace-wide `EventHandler` wrapper around the rendered workspace tree
- `app/src/safe_triangle.rs:1` — safe-triangle logic used to keep hover sidecars stable while the pointer moves toward them
- `crates/warpui_core/src/elements/hoverable.rs:405` — current per-element hover state transitions and coverage behavior
- `app/src/workspace/view/vertical_tabs_tests.rs:1` — helper test coverage for vertical-tabs sidecar logic

## Current state

The current implementation uses two local hover-driven mechanisms:

- row hover callbacks set and clear `detail_overlay_state.active_target`
- a sidecar-local `Hoverable` and `MouseStateHandle` are used to keep the sidecar open while hovered and to clear it on sidecar hover-out

This works for normal pointer movement from a row into the sidecar, especially when the safe triangle suppresses intermediate row hover changes.

It breaks down when pointer movement passes through a region that is visually part of the sidecar flow but does not produce the expected hover transitions on either the row or the sidecar root. The scrollbar gutter is one example. In these cases, `active_target` becomes stale and the overlay continues rendering even though the pointer is no longer in any region that should keep it visible.

The key problem is that visibility currently depends on element-local hover state instead of a direct geometry check against the current pointer position.

## Proposed changes

### 1. Add a pure visibility-reconciliation helper

Add a helper in `app/src/workspace/view/vertical_tabs.rs` that determines whether the sidecar should remain visible for a given pointer position.

Inputs:

- current mouse position
- source-row rect
- sidecar rect
- mutable `SafeTriangle`

Behavior:

- keep visible if the pointer is inside the source row
- keep visible if the pointer is inside the sidecar bounds
- keep visible if the pointer is moving through the safe triangle toward the sidecar
- otherwise report that the sidecar should be cleared

This helper should be pure with respect to vertical-tabs state other than the safe-triangle updates, so it can be tested directly in `vertical_tabs_tests.rs`.

### 2. Add a reconciliation method on detail hover state

Add a method on `VerticalTabsDetailHoverState` that:

- reads the current `active_target`
- resolves the saved source-row position id for that target
- reads the saved source-row rect and sidecar rect from the last frame
- calls the new helper
- clears `active_target` and the safe-triangle target rect when the helper says the sidecar is no longer valid

When clearing, also reset the sidecar mouse interaction state so stale hover state does not keep the overlay alive across subsequent renders.

### 3. Reconcile from the workspace root on mouse moves

Keep the existing row and sidecar hover callbacks, but add a workspace-root mouse-move reconciliation path in `app/src/workspace/view.rs`.

The root `EventHandler` that wraps the workspace tree is the right place for this because it can observe real mouse moves even when:

- the pointer is over a covered child region
- the pointer has left the vertical tabs panel but is still inside the workspace window
- the sidecar's own hover state is stale

Implementation shape:

- only enable the reconciliation path when vertical tabs are enabled and the panel is open
- attach a mouse-move callback at the workspace root with `fire_when_covered: true`
- invoke the new `VerticalTabsDetailHoverState` reconciliation method from that callback
- call `ctx.notify()` only when reconciliation actually clears stale state

This makes the workspace root the final source of truth for sidecar visibility without replacing the existing local hover behavior.

### 4. Preserve the current hover model and safe triangle

Do not replace the row/sidecar hover callbacks with action-driven state or a new overlay architecture.

The current design is still correct for:

- opening the sidecar from row hover
- maintaining the sidecar during normal row-to-sidecar movement
- tracking supported vs unsupported targets

The new reconciliation logic is a correctness backstop for stale-state paths, not a redesign of the feature.

### 5. Be conservative when the sidecar rect is unavailable

The sidecar rect comes from the previous frame via `SavePosition`, so there is a brief window during initial appearance where no sidecar rect exists yet.

The reconciliation logic should avoid aggressively clearing in that state. Until a valid sidecar rect exists, the system should continue to rely on the existing row-hover behavior rather than assume the sidecar has already been left.

## End-to-end flow

1. Hovering a supported vertical-tabs row sets `detail_overlay_state.active_target`.
2. The workspace renders the detail sidecar overlay anchored to that row.
3. On each real mouse move within the workspace, the workspace-root event handler invokes detail-sidecar reconciliation.
4. Reconciliation checks whether the pointer is still in the source row, inside the sidecar bounds, or in the safe-triangle transit path.
5. If one of those conditions is true, the sidecar remains visible.
6. If none of them is true, reconciliation clears `active_target`, clears the safe-triangle target rect, resets sidecar hover interaction state, and requests a redraw.
7. The next render no longer includes the sidecar overlay.

## Risks and mitigations

### Over-clearing during initial sidecar appearance

Risk:

- the sidecar rect is unavailable on the first frame, so geometry-based reconciliation could hide the sidecar too early

Mitigation:

- make reconciliation conservative until the sidecar has a valid saved rect

### Interfering with unrelated workspace hover behavior

Risk:

- adding mouse-move logic at the workspace root could create unintended coupling with other overlays

Mitigation:

- gate the logic narrowly to the vertical-tabs detail-sidecar state
- do nothing unless vertical tabs are enabled, the panel is open, and `active_target` is set

### Stale mouse state after clearing

Risk:

- even after `active_target` is cleared, cached sidecar hover state could persist and create inconsistent behavior on the next render

Mitigation:

- reset sidecar interaction state when clearing stale visibility

## Testing and validation

### Unit tests

Add tests in `app/src/workspace/view/vertical_tabs_tests.rs` for the new geometry helper:

- returns true when the pointer is inside the source row
- returns true when the pointer is inside the sidecar bounds
- returns true when the pointer is still inside the safe triangle
- returns false when the pointer is outside the row, outside the sidecar, and outside the safe triangle

These tests should be independent of the full workspace render path and should focus on the geometry and safe-triangle behavior directly.

### Build and targeted validation

- run targeted tests covering `vertical_tabs_tests`
- run a compile check for the affected crate

### Manual validation

- reproduce the scrollbar-gutter path and verify the sidecar hides immediately after the pointer leaves the valid hover region
- verify normal diagonal movement from a row into the sidecar still keeps the sidecar open
- verify the sidecar still hides normally when moving from the row to unrelated workspace content
- verify no regressions in tabs-mode vs panes-mode sidecar behavior

## Generalization

This fix is likely reusable for other hover-driven sidecar UI, but only at the interaction-primitive layer.

The generalizable pattern is:

- anchor an overlay to a source element via saved geometry
- keep it open while the pointer is over the source, over the overlay, or moving through a safe-triangle corridor between them
- reconcile visibility from a higher-level mouse-move observer instead of depending exclusively on element-local hover state

That pattern is a good fit for hover sidecars, submenu-adjacent overlays, and other floating UI that contains covered child regions such as scrollbars or nested interactive elements.

It is not a good fit for click-triggered popovers, focus-driven overlays, or panels whose lifetime is determined by selection rather than pointer geometry.

For now, this change should stay local to vertical tabs. If similar bugs appear in other sidecar-style UI, the first extraction should be a small geometry/state helper rather than a fully shared overlay component.

## Follow-ups

- if similar stale-visibility bugs appear in other hover sidecars, consider extracting a reusable geometry-based sidecar visibility helper
- if workspace-root mouse reconciliation becomes a recurring pattern, evaluate a shared overlay lifecycle utility instead of keeping this logic local to vertical tabs
