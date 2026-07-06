# Product Spec: Windows Jump List for Tab Configs

**Issue:** [warpdotdev/warp#6156](https://github.com/warpdotdev/warp/issues/6156)
**Figma:** none provided

## Summary

When Warp is pinned to the Windows taskbar, right-clicking its taskbar icon shows the user's saved Tab Configs as jump list entries, plus a "New Window" task. Selecting a Tab Config opens it in Warp (as a new tab, or a new window), mirroring the existing `<scheme>://tab_config/<name>` deeplink behavior. The "New Window" task opens a fresh Warp window at the home directory. This brings Warp to parity with Windows Terminal, which exposes its profiles via the taskbar jump list.

## Behavior

1. On Windows, when Warp is pinned to the taskbar, right-clicking the Warp taskbar icon shows a jump list containing:
   - A "New Window" task in the Windows Tasks section (rendered above custom categories).
   - A "Tab Configs" category listing each saved Tab Config as a destination.

2. Selecting "New Window" opens a fresh Warp window at the default cwd (home directory), via the current channel's URI scheme (e.g. `warp://action/new_window?path=~`).

3. Each entry under "Tab Configs" is labeled with the Tab Config's display `name`.

4. Selecting a Tab Config entry opens the corresponding config via the existing `<scheme>://tab_config/<file-stem>` deeplink path: as a new tab in the active window if Warp is running, or in a new window if Warp is not running.

5. If the selected Tab Config declares params, the existing params modal appears before panes are created.

6. The jump list stays in sync with the user's Tab Configs without an app restart: creating, editing, renaming, or deleting a `.toml` file in the tab configs directory updates the jump list.

7. The "Tab Configs" category contains only successfully parsed Tab Configs. Configs that failed to parse are omitted.

8. When the user has no saved Tab Configs, the "Tab Configs" category is omitted entirely; only the "New Window" task appears.

9. The jump list is per-app, not per-window: all Warp windows show the same jump list.

10. This feature is Windows-only. macOS and Linux are unaffected; no jump list entries, categories, or AUMID changes are introduced there.

11. When the `WindowsJumpList` feature flag is disabled, no jump list entries are populated and any previously committed custom destinations/tasks are cleared so stale entries do not remain. Existing behavior is otherwise unchanged. The process AUMID is set independently by the existing app builder initialization and is unaffected by this flag.

12. When two Tab Config files have the same display `name`, both appear in the jump list with identical labels, but each entry resolves via its unique file stem. This matches the `+` menu behavior.

13. Renaming a Tab Config file removes the old jump list entry and adds the new one after the next directory-watcher refresh.

14. Windows jump lists have a user-configurable limit on the total number of destinations shown (default 10). When the number of saved Tab Configs exceeds the available destination slots, only the capped subset appears. The subset is deterministic, ordered alphabetically by display name to match the `+` menu source order.

15. When Warp is not running and a jump list entry is clicked, Windows launches Warp and delivers the deeplink on startup, opening the Tab Config or new window. This relies on the existing single-instance + URI delivery path used by `warp://` links today.

16. The jump list surfaces only "New Window" and Tab Configs. It does not surface launch configs, workflows, themes, recent files, frequent shells, or custom categories.

17. All jump list entries use the Warp app icon initially. Per-entry or per-shell icons are not supported in this iteration.
