# Product Spec: Tab transfer between standard and dedicated hotkey windows

**Issue:** [warpdotdev/warp#12306](https://github.com/warpdotdev/warp/issues/12306)

## Summary

Three command-palette actions that let users move active tabs (or multi-selections) between standard terminal windows and the dedicated hotkey (quake) window. This covers the command-palette half of the issue's first proposal; the drag-and-drop half is being shipped concurrently by the Warp team.

## Problem

The dedicated hotkey window (quake overlay) is an isolated sandbox. Users who start a task there that scales up — editing a file, running a long build, researching in the AI terminal — have no way to promote those tabs to a standard window. Conversely, users in a standard window who want a distraction-free view of a particular session in the quake overlay cannot move it there.

The issue proposes two solution families:
1. **Tab transfer** — move tabs between window types via drag-and-drop or command-palette actions
2. **Non-dedicated hotkey mode** — a setting that makes the global hotkey present the current active window as a quake-style overlay rather than using a separate dedicated window

The Warp team is shipping cross-window tab drag (hotkey→standard direction) concurrently with this spec ([maintainer comment](https://github.com/warpdotdev/warp/issues/12306#issuecomment-4749224419)). This spec covers the complementary keyboard- and command-palette-driven half of option 1, which applies in both directions (standard↔hotkey).

## Goals

- Users can move active tab(s) from a standard window into the dedicated hotkey window.
- Users can move active tab(s) from the dedicated hotkey window back to a standard window (existing or newly created).
- A single shortcut (toggle) lets users move active tab(s) between window types without needing to know which direction.
- Commands are only visible when the "Dedicated hotkey window" setting is enabled.

## Non-goals

- **Drag-and-drop between standard and hotkey windows** — The Warp team is shipping cross-window drag (hotkey→standard) concurrently. The reverse direction (standard→hotkey) is deferred to a follow-up. This spec covers the keyboard/command-palette path instead, which is complementary to drag and works in both directions.
- **Non-dedicated hotkey mode** — The second proposal from the issue is deferred per maintainer signal.
- Moving tabs between two standard windows (already exists via cross-window drag).
- Changing auto-hide behavior of the hotkey window.

## User experience

### Current behavior

1. User is in the quake overlay with a long-running build and wants to continue working in a standard window. No action exists — they must open a new terminal session from scratch.
2. User is in a standard window with a relevant session and wants to move it to the quake overlay for focused monitoring. No action exists.

### Expected behavior

**`workspace:move_active_tab_to_dedicated_hotkey_window`**
1. User invokes from a standard window while the hotkey window setting is enabled.
2. If the hotkey window is not open, it is programmatically launched.
3. The active tab (or selection of tabs) is removed from the source window and inserted at the end of the hotkey window's tab bar.
4. Focus moves to the hotkey window.

**`workspace:move_active_tab_to_standard_window`**
1. User invokes from the hotkey window.
2. If another standard window is open, tabs are moved there — the most recently focused one.
3. If no standard window is open, tabs are promoted into a newly created standard window.
4. Focus moves to the target standard window.

**`workspace:toggle_active_tab_window_type`**
1. Invoked from any window, moves the active tab(s) to the opposite window type.

### Edge cases

- **Hotkey window disabled in settings:** All three commands are hidden (gated behind `QUAKE_MODE_ENABLED_CONTEXT_FLAG`).
- **Multi-tab hotkey window:** Moving tabs out leaves remaining tabs in place; the window does not auto-hide.
- **Last tab moved:** Source window closes automatically.
- **Multi-selection active:** All selected tabs transfer together. If no multi-selection, only the active tab transfers.
- **Hotkey window closed at invocation:** Opened programmatically first.
- **No standard window available:** A new standard window is created.
- **Stale window ID:** Quake window existence and visibility verified via `quake_mode_window_is_open()` at invocation time.

## Success criteria

1. `workspace:move_active_tab_to_dedicated_hotkey_window` moves the active tab to the hotkey window (opens it first if needed).
2. `workspace:move_active_tab_to_standard_window` moves the active tab to an existing standard window, or creates a new one if none exists.
3. `workspace:toggle_active_tab_window_type` moves the active tab(s) to the opposite window type from any window.
4. Multi-selected tabs are transferred together by all three actions.
5. When the last tab is moved out of any window (standard or hotkey), the source window closes.
6. All three commands are hidden in the command palette and keybindings UI when the dedicated hotkey window setting is disabled.
7. Moving tabs out of a multi-tab hotkey window does not force the window to hide.
8. The hotkey window ID is verified dynamically (not cached) to prevent targeting a closed window.

## Validation

- **Unit test:** `test_move_active_tab_to_window` verifies core tab transfer between two mock workspaces.
- **Manual:** Enable hotkey window, invoke each command, verify transfer in both directions.
- **Manual:** Multi-select 2+ tabs (requires `FeatureFlag::GroupedTabs`), invoke each action, verify all selected tabs move.
- **Manual:** Disable hotkey window setting, confirm commands disappear from command palette.
## Follow-ups

- **Drag-and-drop standard→hotkey** — Extend the cross-window drag system to accept the hotkey overlay as a drop target (the reverse of what the team is shipping). The transfer primitives built here are the same ones a drop handler would call.
- **Non-dedicated hotkey mode** — Deferred per maintainer signal. Would require a new `GlobalHotkeyMode` variant, a different window creation path, and changes to the quake state lifecycle.
