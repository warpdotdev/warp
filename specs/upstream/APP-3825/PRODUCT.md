# APP-3825: Vertical Tabs — Drag Panes Between Tabs

## Summary

Add the vertical-tabs equivalent of Warp's existing horizontal-tab pane-drag flow. When vertical tabs are enabled, a user should be able to drag a pane header over a different tab in the vertical tabs panel, have that tab become the active drag destination, and then drop the pane into the target tab using the same in-tab drop-target and relayout UX that already exists for horizontal tabs.

The feature should also preserve the existing "promote to a new tab" behavior: dragging between tabs should create a new tab at that position instead of targeting an existing tab.

## Problem

In horizontal tabs mode, users can move a pane from one tab to another by dragging the pane header to the top tab strip, hovering the destination tab so it becomes active, and then dropping the pane into a new location inside that tab. That workflow is currently missing in vertical tabs mode, even though the same pane-drag and in-tab relayout concepts already exist elsewhere in the product.

This creates an inconsistent drag-and-drop model between horizontal and vertical tabs. Users who opt into vertical tabs lose an existing pane-management workflow and must fall back to indirect alternatives like creating a new tab first or rearranging panes after the fact.

## Goals

- Restore parity with the existing horizontal-tab pane-drag workflow when vertical tabs are enabled.
- Let users target an existing tab from the vertical tabs panel and then place the pane within that tab using the existing pane drop overlays and relayout rules.
- Let users create a new tab by dropping between vertical tab groups, matching the current horizontal-tab "drop between tabs" behavior.
- Make the targeting behavior work regardless of whether the vertical tabs panel is in compact or expanded mode.

## Non-goals

- Auto-opening the vertical tabs panel if it is currently closed.
- Redesigning the in-tab pane drop overlays or changing how pane placement inside a tab works.
- Changing any of the existing special-case pane-drop rules inside the target tab (for example, code-pane merge behavior when tabbed editor view is preferred).
- Adding new keyboard interactions for this drag flow.
- Extending this ticket to editor file-tab dragging; this spec is only for dragging pane headers.

## Figma / design references

Figma: none provided

Design intent should match the current horizontal-tab pane-drag behavior as closely as possible.

## User experience

### Availability

This behavior applies when:

- vertical tabs are enabled, and
- the vertical tabs panel is visible, and
- the user is dragging a pane by its pane header.

If the vertical tabs panel is closed, this ticket does not introduce a new auto-open or alternate targeting path.

### Valid tab targets in the vertical tabs panel

Each visible vertical tab group represents one workspace tab and must be targetable during a pane drag.

In practice:

- In **expanded** mode, hovering any visible part of a tab group counts as hovering that tab, including its pane rows and its optional custom-title header.
- In **compact** mode, the same rule applies to the compact rendering of that tab group.
- The user does not need a custom tab title/header in order to target a tab. Tabs that render only pane rows must still be targetable.

Hover-only panel chrome must not interfere with drag targeting:

- Drag-target feedback takes precedence over hover-only action affordances like the kebab/close button belt.
- Those controls must not prevent the user from targeting the underlying tab group while a pane drag is in progress.

### Hovering an existing tab

When the user drags a pane header over a different tab group in the vertical tabs panel, Warp should treat that tab group the same way the horizontal tab strip treats a hovered destination tab today.

That means:

- The hovered tab group is shown as the active drag target.
- The workspace switches to that tab as the drag destination.
- Once that tab is active, the user can continue moving the cursor into the workspace content area and see the existing pane relayout drop targets for that tab.

This should feel like a direct vertical-tabs analogue of the current horizontal behavior, not like a separate drag mode.

### Dropping into the target tab

After the destination tab becomes active, dropping the pane inside the workspace should reuse the existing in-tab pane drop behavior unchanged.

Specifically:

- The same drop-target affordances that already appear when rearranging panes within a tab should appear for the dragged pane in the destination tab.
- The same placement outcomes should apply as they do today in horizontal tabs mode.
- Any existing special-case behavior in the destination tab remains unchanged. For example, if a destination tab's current rules would merge a dragged code pane into an existing code-pane/editor setup instead of allowing arbitrary free placement, vertical tabs should preserve that same result rather than inventing a new one.

### Dragging between tabs to create a new tab

The vertical tabs panel must also support the vertical analogue of "drop between tabs."

When the dragged pane is positioned between two visible tab groups, Warp should show an insertion indicator between those groups. Dropping there creates a new workspace tab containing the dragged pane at that position.

The same applies after the last visible tab group:

- Hovering below the final tab group shows an insertion indicator at the end of the list.
- Dropping there creates a new tab at the end.

This should match the semantics of the current horizontal-tab strip:

- **Over a tab** targets that existing tab.
- **Between tabs** creates a new tab at that position.

### Visual feedback

During a pane drag in vertical tabs mode, the panel should provide clear, mutually exclusive feedback:

- **Over an existing tab**: that tab group is highlighted as the current destination tab.
- **Between tab groups**: show an insertion indicator between groups.
- **No valid tab target**: clear any tab-target highlight or insertion indicator.

At no point should both an existing-tab highlight and a between-tabs insertion indicator be shown at the same time.

### Cancellation and reversibility

The cross-tab drag flow must remain reversible until the drop is committed.

- Moving off a target tab group removes that target state.
- Moving from one tab group to another updates the target accordingly.
- Aborting the drag or dropping outside any valid destination leaves the tab/pane layout unchanged.

### No new behavior outside the intended scope

This ticket does not change how users reorder vertical tabs by dragging the tab groups themselves. It only adds parity for dragging a pane header from one tab into another tab or into a new tab position.

## Success criteria

1. In vertical tabs mode with the panel open, dragging a pane header over a different visible tab group makes that tab group the active drag destination.
2. When a non-active tab group becomes the drag destination, Warp switches the workspace to that tab so the user can place the pane inside it.
3. After switching to the destination tab, the existing pane relayout/drop-target UX appears in the workspace and can be used to place the pane.
4. The placement outcomes inside the destination tab match the existing horizontal-tabs flow; no new placement rules are introduced.
5. Dragging between two visible tab groups shows an insertion indicator and dropping there creates a new tab containing the pane at that position.
6. Dragging below the last visible tab group shows an end-of-list insertion indicator and dropping there creates a new tab at the end.
7. The behavior works in both compact and expanded vertical tabs panel modes.
8. Tabs without a custom title/header are still targetable via their rendered tab-group body.
9. Hover-only controls in the vertical tabs panel do not block or replace drag-target feedback.
10. Cancelling the drag, or dropping outside a valid destination, leaves the tab/pane layout unchanged and clears any temporary targeting UI.
11. Existing vertical-tab reordering behavior is unchanged.
12. Existing within-tab special cases, including code-pane merge behavior where applicable, remain unchanged.

## Validation

- **Existing-tab transfer**: In vertical tabs mode, create two tabs with multiple panes. Drag a pane header from tab A over tab B in the vertical tabs panel. Verify tab B becomes active and the pane can be dropped into a new split location using the normal in-tab drop overlays.
- **Compact mode**: Repeat the same flow with the vertical tabs panel in compact mode.
- **Expanded mode**: Repeat the same flow with the panel in expanded mode.
- **No custom header**: Verify the drag works for a destination tab that has no custom tab title and therefore renders without a separate custom header row.
- **New-tab insertion**: Drag a pane between two tab groups and verify an insertion indicator appears. Drop and confirm a new tab is created at that exact position containing the dragged pane.
- **End insertion**: Drag below the last tab group and verify dropping creates a new final tab.
- **Cancel path**: Start a cross-tab pane drag, hover a target so it highlights, then cancel or drop outside any valid target. Verify no pane move is committed and temporary highlighting clears.
- **Special-case regression check**: Use a scenario where the existing horizontal-tabs flow has a special outcome inside the target tab (for example, a code-pane/editor merge case). Verify vertical tabs preserves the same behavior.
- **Tab reordering regression**: Verify that dragging vertical tab groups themselves still reorders tabs exactly as before.

## Open questions

None.
