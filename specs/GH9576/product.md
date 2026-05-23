# PRODUCT.md — Show current zoom level when zooming

Issue: https://github.com/warpdotdev/warp/issues/9576

## Summary

When a user changes Warp's UI zoom level from a keyboard shortcut, menu action, command-palette action, or supported Ctrl+scroll gesture, Warp should briefly show the resulting zoom level as a percentage in the workspace. The indicator should make the new state obvious without requiring the user to open Settings → Appearance → Window → Zoom.

Figma: none provided. A prior issue comment requested designs as part of the spec; this spec defines a lightweight HUD direction that can be validated with a live prototype or screenshot before implementation review.

## Problem

Warp already supports discrete UI zoom levels, but the workspace gives no direct feedback after zooming. Users can see the interface rescale, but they cannot tell whether they landed at `90%`, `100%`, `110%`, or another supported step unless they open settings. After several zoom-in or zoom-out actions, the only quick way to recover a known value is reset-to-default.

## Goals

- Show the current UI zoom level immediately after user-initiated zoom changes.
- Use the same percentage values and formatting shown by the Appearance settings dropdown.
- Keep the feedback transient, low-distraction, and scoped to the active workspace window.
- Support every existing user-facing zoom path that dispatches `IncreaseZoom`, `DecreaseZoom`, or `ResetZoom`.
- Avoid adding another persistent setting, toolbar item, menu item, or panel.
- Preserve existing zoom behavior, step values, limits, and reset semantics.

## Non-goals

- Changing the supported zoom values or introducing continuous zoom.
- Showing a permanent zoom badge in the tab bar, status area, settings button, or terminal.
- Showing the indicator for unrelated settings changes unless they are caused by a user-facing workspace zoom action.
- Adding telemetry as part of the spec-only change. If product later wants measurement, that should be specified separately.
- Removing or stabilizing the existing `UIZoom` feature flag.

## Design direction

The intended surface is a compact, non-interactive HUD rather than a general-purpose toast. It should visually resemble common zoom feedback in apps such as VS Code, Chrome, Slack, and macOS apps:

- Content: the zoom percentage only, for example `110%`.
- Placement: centered horizontally near the top of the active workspace content, below the tab/title bar area and above terminal or panel content.
- Shape: small rounded rectangle or pill with theme-aware background, border or shadow if needed for contrast, and prominent percentage text.
- Lifetime: visible for about one second after the most recent zoom action, then disappears automatically.
- Repeated zooming: update the same HUD in place and restart the timeout instead of stacking multiple indicators.
- Interaction: no close button, no link, no click action, and no keyboard focus.
- Accessibility: the indicator is visual feedback for a user-triggered scaling action and should not steal focus. If the UI framework supports non-disruptive announcements without excessive chatter, it may expose the updated percentage, but this should not block the initial implementation.

## User experience

1. When the `UIZoom` feature is enabled, invoking "Zoom In" increases the zoom level to the next supported value and shows a HUD containing the resulting percentage, such as `110%`.

2. Invoking "Zoom Out" decreases the zoom level to the previous supported value and shows the resulting percentage, such as `90%`.

3. Invoking "Reset Zoom" resets the zoom level to the default value and shows `100%`.

4. The indicator uses exactly the supported zoom values from Warp's zoom setting: `50%`, `60%`, `70%`, `80%`, `90%`, `100%`, `110%`, `125%`, `150%`, `175%`, `200%`, `225%`, `250%`, `300%`, and `350%`.

5. The indicator text is formatted as `{value}%` with no extra label by default. For example, it says `125%`, not `Zoom: 125%`.

6. The behavior applies to every user-facing workspace zoom trigger that dispatches the existing zoom actions:
   - "Zoom In", "Zoom Out", and "Reset Zoom" keybindings.
   - The corresponding command-palette or keybinding-dispatched actions.
   - The corresponding View menu items.
   - Ctrl+scroll zooming on Windows and Linux when that path dispatches zoom actions.

7. The indicator is scoped to the window where the zoom action was invoked. It does not appear in other open Warp windows.

8. If a user invokes zoom in at the maximum supported value (`350%`) or zoom out at the minimum supported value (`50%`), the zoom value remains clamped and the HUD still shows the current value. This confirms that the request was handled and explains why the UI did not scale further.

9. If the current zoom setting is somehow not one of the supported stepped values and a zoom-in or zoom-out action cannot determine the next value, the action should not show a misleading HUD. Existing fallback behavior for invalid values is preserved.

10. Consecutive zoom actions within the HUD lifetime update the visible percentage and extend the display duration from the latest action. The user should never see a stack of old zoom percentages.

11. Opening Settings → Appearance → Window → Zoom and changing the dropdown continues to update the UI zoom and selected value as it does today. Showing the HUD for direct settings-dropdown changes is optional, but the implementation must not regress settings behavior.

12. The HUD must render correctly over the main workspace in common layouts: single terminal pane, split panes, settings open, command palette closed, AI/resource panels open, horizontal tabs, vertical tabs, and full-screen or hover-hidden tab-bar modes.

13. The HUD should respect the active theme and remain readable at all supported zoom levels. It must not obscure modal dialogs, native confirmation dialogs, or blocking overlays more aggressively than existing workspace overlays.

14. The feature remains gated by the same `UIZoom` availability as the underlying zoom actions. When UI zoom is disabled and the same shortcuts adjust terminal font size instead, no UI zoom percentage HUD is shown.

## Success criteria

- A user can press the zoom-in shortcut once from `100%` and see `110%` without opening settings.
- A user can press the zoom-out shortcut once from `100%` and see `90%`.
- A user can reset zoom and see `100%`.
- Repeated keypresses update one transient indicator rather than creating multiple stacked toasts.
- At `50%` and `350%`, additional decrease/increase requests show the clamped current percentage and do not suggest a value outside the supported list.
- The settings dropdown continues to show and persist the same zoom value used by the HUD.
- The indicator appears in the active window only and disappears automatically after roughly one second.
- Existing zoom shortcuts, menu actions, command-palette actions, and Linux/Windows Ctrl+scroll zoom behavior continue to work.
- The implementation can be reviewed with at least one screenshot or short video showing the HUD in the workspace.

## Validation

- Unit test the zoom action helpers so increase, decrease, reset, min clamp, max clamp, and invalid-current-value cases map to the expected HUD behavior.
- Add or update workspace view tests to verify that dispatching zoom actions creates or updates one active zoom indicator and that the indicator can be dismissed by its timeout path.
- Manually verify on macOS that `Cmd =`, `Cmd -`, and reset-to-default zoom show the expected percentage.
- Manually verify on Windows or Linux that Ctrl+scroll zoom shows the same percentage feedback when `UIZoom` is enabled.
- Manually verify Appearance settings still displays and updates the same current zoom level.
- Capture a screenshot or short video artifact of the HUD to satisfy the design-review request in the issue comments if no Figma mock is available.

## Open questions

- Should direct changes from the Appearance zoom dropdown show the HUD, or should feedback be limited to quick workspace zoom actions?
- Should the HUD include accessible live-region announcement text, or is visual feedback sufficient for this first iteration?
- Should final visual placement be top-center of workspace content or centered over the whole window? This spec prefers top-center of workspace content to avoid modal-like interruption.
