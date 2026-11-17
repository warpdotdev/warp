# PRODUCT.md - Add optional confirmation before closing tabs

Issue: https://github.com/warpdotdev/warp/issues/10995

## Summary

Add an opt-in setting that asks users to confirm before closing Warp tabs. The
default remains today's fast-close behavior. Users who often keep many tabs open
can enable the setting to avoid accidental tab loss from close buttons,
middle-clicks, keybindings, tab context-menu actions, or tab-group close
actions.

Figma: none provided.

## Goals / Non-goals

Goals:

- Preserve the current tab-close behavior by default.
- Provide a single opt-in setting under the existing Tabs settings surface.
- Confirm ordinary tab close actions before any tab is removed.
- Apply the same behavior to horizontal tabs, vertical tabs, keyboard actions,
  tab context-menu close actions, and tab-group close actions.
- Preserve existing higher-severity warnings for shared sessions, running
  processes, and unsaved state.
- Avoid double-confirming the same close action when a higher-severity warning is
  already shown.

Non-goals:

- Hiding, moving, resizing, pinning, or locking tab close buttons.
- Changing the existing undo/reopen-closed-tab behavior.
- Changing last-tab/window-close confirmation behavior.
- Changing shared-session or running-process warning copy.
- Adding per-tab or per-profile confirmation rules.

## Behavior

1. A new setting appears in Settings > Appearance > Tabs:
   "Confirm before closing tabs".

2. The setting defaults to off. With the setting off, closing tabs behaves
   exactly as it does today.

3. The setting is persistent and syncs consistently with other global tab
   appearance preferences.

4. When the setting is on, a normal single-tab close shows one confirmation
   dialog before removing the tab.

5. Single-tab dialog copy:
   - Title: "Close tab?"
   - Body: "This tab will be closed."
   - Primary button: "Close tab"
   - Secondary button: "Cancel"

6. When the user chooses "Cancel", no tabs are closed, the active tab is
   unchanged, and the existing undo-close stack is not modified.

7. When the user chooses "Close tab", the requested tab closes through the same
   close path used today. Existing telemetry and undo/reopen behavior apply only
   after the tab actually closes.

8. Bulk close actions show a single confirmation dialog for the whole action,
   not one dialog per tab. Bulk actions include "Close other tabs", "Close tabs
   to the right", the vertical-tabs equivalent "Close tabs below", "Close tab
   group", and tab-group menu actions that close tabs outside, above, or below a
   group.

9. Bulk dialog copy:
   - Title: "Close N tabs?"
   - Body: "These tabs will be closed."
   - Primary button: "Close tabs"
   - Secondary button: "Cancel"
   where N is the number of tabs that would be closed.

10. A bulk confirmation is not shown when the close action would close zero tabs.

11. The confirmation applies to close requests that route through workspace tab
    close actions, including:
    - horizontal tab close button
    - horizontal tab middle-click
    - vertical tab close button
    - vertical tab middle-click
    - keyboard close action for the active tab
    - tab context-menu close actions
    - tab-group close buttons
    - tab-group context-menu close actions
    - command-palette actions that close tabs

12. Existing higher-severity close warnings take precedence over this new
    general confirmation. If a close action would show an existing shared-session
    warning, running-process warning, or unsaved-state warning, Warp shows that
    warning instead of the general tab-close confirmation.

13. Confirming an existing higher-severity warning closes the tabs as it does
    today. Warp does not show an additional "Close tab?" or "Close N tabs?"
    dialog for the same user action.

14. If a tab's risk state changes while the general confirmation dialog is open
    (for example, a long-running process starts), confirming the general dialog
    must not bypass the higher-severity warning checks.

15. Closing the last tab keeps today's behavior: if the action is effectively a
    window close, the existing window-close confirmation path remains
    responsible for any warning. The new tab-close confirmation does not add a
    second dialog for last-tab close.

16. The confirmation is shown before tab rename state is cancelled and before
    tab removal begins. Cancelling the dialog leaves any in-progress tab rename
    state unchanged.

17. Settings search can find the setting with terms such as "confirm", "close
    tab", and "closing tabs".

18. The command palette exposes a toggle action for the setting, so users can
    enable or disable confirmation without navigating to Settings.

19. Accessibility: the setting row, dialog title, dialog body, and dialog buttons
    expose the same text to assistive technologies. The dialog is keyboard
    reachable, and Escape/cancel behavior leaves tabs unchanged.

## Open Questions

- Should the dialog body mention the platform-specific reopen shortcut, or avoid
  shortcut copy because the behavior differs by platform and keybinding?
- Should the implementation add a new telemetry event when users toggle the
  setting, or rely on existing tab-operation telemetry?
