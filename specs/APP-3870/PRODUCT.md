# APP-3870: Vertical Tabs — Show Details on Hover toggle
## Summary
Add a new vertical-tabs display option, `Show details on hover`, that lets users enable or disable the hover detail sidecar.

The toggle lives in the vertical tabs display options menu and defaults to enabled so existing users keep the current behavior unless they explicitly turn it off.
## Problem
The hover detail sidecar is useful when users want richer metadata without changing focus, but always-on hover detail can also feel noisy or distracting for users who prefer a simpler vertical-tabs panel.

Once the sidecar exists, users need a straightforward way to opt out of hover-triggered detail without disabling vertical tabs or changing unrelated display settings.
## Goals
- Let users turn the vertical-tabs hover detail sidecar on or off from the existing display options menu.
- Preserve current behavior for existing users by defaulting the setting to enabled.
- Make the toggle apply immediately without requiring a restart or tab reload.
- Keep the toggle scoped specifically to hover-driven detail sidecar behavior.
- Ensure the setting is persisted like other vertical-tabs display preferences.
## Non-goals
- Changing the content or layout of the detail sidecar itself.
- Changing sidecar eligibility rules for supported and unsupported pane types.
- Adding keyboard-only ways to open the sidecar.
- Reworking other display options such as `View as`, `Density`, `Pane title as`, `Additional metadata`, `PR link`, or `Diff stats`.
- Adding per-workspace or per-tab overrides for the setting.
## Figma / design references
Figma: none provided
## User experience
### Menu placement and labeling
- The vertical tabs display options menu includes a new untitled section separated from the items above by a divider.
- That section contains a single toggle row labeled `Show details on hover`.
- The row uses the same selected / unselected visual treatment as the other menu toggles in this popup.

### Default behavior
- `Show details on hover` defaults to enabled.
- For users who have never changed the setting, hover behavior matches the current sidecar experience.
- The setting persists as part of the user’s vertical-tabs display preferences.

### Enabled state
- When the toggle is enabled, hovering an eligible vertical-tabs row can open the detail sidecar exactly as it does today.
- Existing hover behavior remains unchanged, including:
  - supported-pane eligibility rules
  - panes-vs-tabs behavior
  - safe-triangle behavior while moving the cursor from the row into the sidecar
  - existing sidecar interactions such as clickable PR and diff badges

### Disabled state
- When the toggle is disabled, hovering vertical-tabs rows does not open the detail sidecar.
- Disabling the toggle hides any currently visible hover detail sidecar immediately.
- While disabled, moving the pointer across eligible rows does not create, reopen, or update the sidecar.
- Disabling the toggle affects only the hover detail sidecar; it does not change row focus, click behavior, row rendering, or other popup menu settings.

### Interaction and state transitions
- Toggling the setting on takes effect immediately; the user does not need to close and reopen the panel.
- Toggling the setting off takes effect immediately, including dismissing any currently open sidecar.
- Re-enabling the setting restores normal hover behavior without requiring any additional setup.

### Relationship to other display settings
- The new toggle is independent from `PR link` and `Diff stats`; those settings still control row-level metadata visibility in expanded mode.
- The new toggle is independent from `View as`, `Density`, `Pane title as`, and `Additional metadata`.
- Turning off `Show details on hover` does not alter the content the sidecar would show if it were enabled; it only suppresses hover-based opening.

### Empty and error states
- There is no separate empty state for this feature.
- If the setting cannot be read yet during initial render, the user should see the default enabled behavior rather than a broken or inconsistent menu state.
## Success criteria
1. The vertical tabs display options menu shows a new toggle labeled `Show details on hover`.
2. The toggle appears in its own separated section with no section title.
3. The toggle defaults to enabled for users who have not changed the setting before.
4. When enabled, hover detail sidecar behavior matches the current behavior.
5. When disabled, hovering eligible rows never opens the sidecar.
6. Turning the toggle off immediately dismisses any currently open sidecar.
7. Turning the toggle back on immediately restores hover-open behavior.
8. The setting persists across app restarts and syncs like other vertical-tabs display preferences.
9. Changing this setting does not change row activation, row layout, or the behavior of unrelated vertical-tabs display settings.
## Validation
- Open the vertical tabs display options menu and verify there is a divider followed by an untitled row for `Show details on hover`.
- Verify the toggle is checked by default in a clean settings state.
- With the toggle enabled, hover an eligible pane or tab row and verify the detail sidecar opens with the existing behavior.
- While a sidecar is visible, disable the toggle and verify the sidecar dismisses immediately.
- With the toggle disabled, hover multiple eligible rows and verify no sidecar appears.
- Re-enable the toggle and verify the same rows can open the sidecar again on hover.
- Change unrelated display settings such as `View as`, `Density`, `PR link`, and `Diff stats` and verify the new toggle still only controls hover-sidecar visibility.
- Restart the app or reload settings state and verify the selected value persists.
## Open questions
None.
