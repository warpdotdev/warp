# Hide Warp Dock Icon with Menu Bar Fallback — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/1154
Figma: none provided

## Summary
Add a macOS-only setting that lets users hide Warp from the Dock and Cmd-Tab app switcher while Warp continues running. When the Dock icon is hidden, Warp must show a minimal menu bar icon in the system status area so users can bring Warp back, open a window, access settings, or quit even if they do not have a global hotkey configured.

This behavior should be available as a general app presentation preference, not only when the dedicated global hotkey window is enabled.

## Problem
Users who primarily use Warp through the dedicated hotkey window or another global hotkey do not need a persistent Dock icon. The icon occupies Dock and Cmd-Tab space, and clicking it can open a normal Warp window separate from the user's hotkey workflow. Users currently resort to unsupported bundle edits that are reverted by updates and can leave broken Dock state.

At the same time, hiding the Dock icon without another persistent entry point would strand users who forget their hotkey, disable the hotkey, or have no Warp windows visible. A menu bar fallback is required for safe discoverability and recovery.

## Goals
- Provide a macOS setting to show or hide Warp's Dock icon.
- Hide Warp from both the Dock and Cmd-Tab switcher when the setting is off.
- Keep the setting independent of the global hotkey mode; users can hide the Dock icon whether global hotkey is disabled, dedicated hotkey window is enabled, or show/hide-all-windows hotkey is enabled.
- Always show a minimal Warp menu bar icon while the Dock icon is hidden.
- Let users use the menu bar icon to bring Warp forward, open a new window, open settings, and quit Warp.
- Apply changes without requiring the user to restart Warp.
- Persist the preference across launches and upgrades without requiring unsupported bundle edits.

## Non-goals
- Changing Warp's default behavior. Existing users should continue to see Warp in the Dock unless they opt out.
- Making Warp launch at login or launch silently in the background. Startup behavior can be addressed separately.
- Allowing users to hide both the Dock icon and the menu bar icon at the same time.
- Changing global hotkey keybinding validation or adding new hotkey combinations.
- Changing the icon art options added for the Dock icon; hiding the Dock icon is a separate presentation setting, not another icon style.
- Implementing equivalent Dock/taskbar hiding behavior on Windows, Linux, or web.

## Figma / design references
Figma: none provided.

## User experience
### Settings
1. On macOS, settings include a user-facing control for Dock visibility near the existing app icon customization controls.
2. The default is to show Warp in the Dock.
3. Turning the setting off immediately removes Warp from the Dock and Cmd-Tab switcher.
4. Turning the setting back on immediately restores Warp to the Dock and Cmd-Tab switcher and removes the menu bar fallback icon.
5. The setting is hidden or disabled on non-macOS platforms.
6. The setting is not gated by global hotkey mode. It remains available even when global hotkey is disabled.

### Hidden Dock icon state
1. When the Dock icon is hidden, Warp remains running and existing terminal sessions continue unaffected.
2. Warp does not appear in the Dock.
3. Warp does not appear in Cmd-Tab.
4. Warp shows a minimal menu bar icon in the top-right system status area for as long as Warp is running.
5. The menu bar icon should use a recognizable Warp mark and should be visually suitable for light and dark menu bars.
6. The menu bar icon is not optional in this release. It is the required recovery surface when the Dock icon is hidden.

### Menu bar icon behavior
1. Clicking the menu bar icon opens a menu.
2. The menu includes at least:
   - Show Warp
   - New Window
   - Settings
   - Quit Warp
3. Show Warp brings Warp to a usable foreground state:
   - If the dedicated hotkey window is enabled, Show Warp shows or focuses the dedicated hotkey window without toggling it closed if it is already visible.
   - If the dedicated hotkey window is not enabled and Warp has existing normal windows, Show Warp activates Warp and brings an existing normal window forward.
   - If there are no usable normal windows and no dedicated hotkey window to show, Show Warp opens a new normal window.
4. New Window always opens a normal Warp window, matching the existing New Window action.
5. Settings opens Warp settings, creating or focusing a Warp window as needed.
6. Quit Warp follows the same quit confirmation and unsaved-session behavior as quitting from the app menu today.
7. The menu remains available when all Warp windows are hidden or closed, as long as Warp is still running.

### Interaction with global hotkey modes
1. Dedicated hotkey window mode continues to show, hide, pin, size, and auto-hide the hotkey window exactly as it does today.
2. Show/hide-all-windows hotkey mode continues to hide and show normal windows exactly as it does today.
3. Hiding the Dock icon does not enable a global hotkey, change an existing global hotkey, or require one.
4. The menu bar icon remains visible even when the dedicated hotkey window is hidden by blur or by pressing the hotkey again.
5. If a user disables all global hotkeys while the Dock icon is hidden, the menu bar icon remains the recovery path.

### Startup and session restore
1. The hidden Dock icon preference persists across restart.
2. On launch, Warp should apply the saved Dock visibility preference as early as practical so the Dock icon does not visibly linger longer than necessary.
3. Session restore behavior is unchanged. If Warp would normally restore or create a window on launch, it still does so.
4. If Warp launches with the Dock icon hidden and no visible windows, the menu bar icon must still appear.

### Edge cases
1. If macOS or the user hides overflow menu bar items, Warp is only responsible for creating its menu bar item; system-level menu bar overflow behavior is out of scope.
2. If applying the hidden Dock state fails, Warp should leave the app in the safe visible-Dock state rather than removing all visible entry points.
3. If the app is run unbundled in a local developer environment, the setting should degrade safely. It is acceptable for Dock hiding or icon art to be limited to bundled macOS builds as long as the app does not crash.
4. If the setting changes while a quit confirmation modal or another modal is open, the modal should remain usable and the setting should apply without closing the modal.

## Success criteria
1. A macOS user can disable the Dock icon from settings and immediately no longer sees Warp in the Dock.
2. With the Dock icon disabled, Warp is absent from Cmd-Tab.
3. With the Dock icon disabled, a Warp menu bar icon is present while Warp is running.
4. The menu bar icon offers Show Warp, New Window, Settings, and Quit Warp.
5. Show Warp provides a usable window whether the user has dedicated hotkey mode enabled, show/hide-all-windows mode enabled, or no global hotkey enabled.
6. Users cannot configure Warp into a state with neither a Dock icon nor a menu bar icon.
7. Re-enabling the Dock icon restores Dock and Cmd-Tab presence and removes the menu bar fallback.
8. The preference survives restarting Warp.
9. Existing app icon customization continues to affect the Dock icon when the Dock icon is visible.
10. Non-macOS users do not see broken or irrelevant controls.

## Validation
- On macOS, manually toggle the setting off and verify Warp disappears from the Dock and Cmd-Tab while remaining running.
- Verify the menu bar icon appears immediately when the Dock icon is hidden and disappears when the Dock icon is restored.
- Verify menu bar actions:
  - Show Warp with dedicated hotkey enabled and hidden.
  - Show Warp with dedicated hotkey enabled and visible.
  - Show Warp with global hotkey disabled and no visible windows.
  - New Window.
  - Settings.
  - Quit Warp with and without a quit confirmation.
- Restart Warp with the setting off and verify the hidden Dock state and menu bar icon are restored.
- Toggle the setting while a Warp window and a settings modal are visible and verify the app remains usable.
- Verify existing app icon customization still works when Dock visibility is on.
- Verify the setting is absent or disabled on non-macOS builds.

## Open questions
- Should the menu bar icon support a primary click shortcut that directly performs Show Warp instead of always opening the menu, or is a menu-only first release preferable?
- Should the menu bar menu include a checked "Show in Dock" item so users can restore the Dock icon without opening settings?
