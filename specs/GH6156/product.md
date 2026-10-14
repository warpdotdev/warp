# Product Spec: Windows Jump List for Tab Configs

**Issue:** [warpdotdev/warp#6156](https://github.com/warpdotdev/warp/issues/6156)
**Figma:** none provided

## Summary

Right-clicking Warp's taskbar button on Windows shows a "New Window" task followed by one task per saved Tab Config. Selecting an entry opens it in Warp through the existing `<scheme>://action/new_window` and `<scheme>://tab_config/<file-stem>` deeplinks. Windows Terminal offers its profiles from the taskbar in the same way.

## Behavior

1. On Windows, the Warp jump list (opened by right-clicking the taskbar button, pinned or running) contains, in its "Tasks" section:
   - A "New Window" task.
   - One task per saved Tab Config, after "New Window".

2. Selecting "New Window" opens a new Warp window at the home directory (`<scheme>://action/new_window?path=~`).

3. Tab Config tasks appear in the same order, and with the same labels, as Tab Configs in the `+` menu. When several Tab Configs share a `name`, the first is labeled `Name`, the next `Name (1)`, then `Name (2)`, and so on.

4. Selecting a Tab Config task behaves exactly like opening its `<scheme>://tab_config/<file-stem>` deeplink: the config opens as a new tab in the active window, which is brought to the front, or in a new window if none is open. If it declares params, the params modal appears first.

5. When Warp is not running, selecting a task starts Warp the way any `warp://` link does: the previous session is restored (if session restore is on), then the task's deeplink is applied. When Warp is running, the click is handed to the running instance and no second Warp process stays open.

6. The list stays in sync with the Tab Configs directory while Warp runs. Creating, editing, renaming or deleting a Tab Config file updates the list after the directory watcher picks up the change. Tab Configs that fail to parse are omitted until they parse again.

7. When there are no saved Tab Configs, the list contains only "New Window".

8. The list is written when Warp starts and on every change described in 6; the first window never waits for it. If Windows rejects an update, the error is reported, no UI is shown, and the previous list stays.

9. While Warp is closed, the list shows the entries from its last update. A task for a Tab Config deleted while Warp was closed starts Warp but opens no Tab Config; the next launch removes it from the list.

10. Each channel (Stable, Preview, Dev, OSS) has its own list, tied to that channel's taskbar button and URI scheme.

11. Users can't remove or pin individual tasks; Windows doesn't offer that for tasks. To change the list, users edit their Tab Configs.

12. Tasks are shown whether or not "Show recently opened items in Start, Jump Lists, and File Explorer" is enabled in Windows settings.

13. Every task uses the Warp app icon.

14. The list exists only while the `WindowsJumpList` and `TabConfigs` feature flags are enabled. Turning `WindowsJumpList` off stops further updates; a list written earlier stays, and its entries keep working.

15. macOS and Linux are unaffected.

## Goals

- One-click access from the taskbar to a new window and to the user's saved Tab Configs.
- Reuse existing deeplinks, so jump list tasks behave exactly like `warp://` links.

## Non-goals

- **Listing detected shells directly**, as in the issue's original request. That needs a new shell deeplink. Until then, a one-pane Tab Config per shell gives the same result, and a shells group can be added to the same task list later.
- A separate "Tab Configs" category, which would be subject to the recent-items privacy setting, slot limits and per-item removal.
- Launch configs, workflows, recent directories, or Warp Drive objects.
- Per-entry icons, a settings toggle, or removing the list on uninstall.
- Changing what `warp://tab_config/...` or `warp://action/new_window` open. The one change is that a `tab_config` link opened into an existing window now brings that window to the front, where the windowing system allows apps to raise their own windows (not on Wayland).
