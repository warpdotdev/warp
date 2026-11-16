# Cycle the active tab color with an editable keybinding — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/14069

Figma: none provided

## Summary

Add one editable, keyless-by-default action that advances the active tab through Warp's existing tab colors. The action gives users a keyboard-only alternative to the tab color picker and `/set-tab-color`, including when a terminal-hosted chat tool consumes slash-prefixed input itself.

## Goals

- Let users assign one keybinding that repeatedly cycles the active tab through the same colors offered by the existing tab color picker.
- Make the action available from both Keyboard Shortcuts settings and the Command Palette without assigning a new default shortcut.
- Preserve existing tab, tab-group, directory-color, focus, and persistence behavior.

## Non-goals

- Adding one action or default shortcut per color.
- Changing terminal themes, directory-to-color rules, automatic status colors, or the available tab color palette.
- Replacing or changing the existing tab context-menu picker, `/set-tab-color` command, or programmatic tab-color controls.
- Applying a color to every selected tab. This action targets only the active tab or the active tab's group.
- Adding this GUI tab-color action to Warp's headless TUI.

## Behavior

1. Warp exposes one action named **Cycle current tab color** in Settings → Keyboard Shortcuts and in the Command Palette whenever workspace actions are available.

2. The action has no default keybinding. A user can assign, change, remove, and reset its shortcut through the existing Keyboard Shortcuts editor, with the same conflict handling and platform-specific keystroke behavior as other editable workspace actions.

3. Invoking the action from either an assigned keybinding or the Command Palette immediately advances the active tab's visible color in this order:
   1. no color → Red
   2. Red → Green
   3. Green → Yellow
   4. Yellow → Blue
   5. Blue → Magenta
   6. Magenta → Cyan
   7. Cyan → no color

4. Repeated invocations continue around the cycle: after Cyan becomes no color, the next invocation sets Red. Warp does not open a picker or require a second confirmation at any point in the cycle.

5. The next color is based on the color currently visible on the active tab, regardless of whether that color came from the context-menu picker, `/set-tab-color`, an earlier invocation of this action, restored workspace state, or a directory color rule. If the current visible color is absent, stale, or not one of the available tab colors, the action starts at Red.

6. Cycling from Cyan to no color explicitly shows no color even when a directory color rule would otherwise supply a default. The following invocation sets Red and resumes the normal order.

7. When the active tab belongs to a tab group, the action cycles the tab group's visible color. The group header and container continue to use that shared group color, while member tab and pane colors retain their existing rendering behavior. The action does not create a per-tab color override hidden behind the group.

8. When the active tab is ungrouped, only that tab changes. Other tabs, other tab groups, and tabs included in a multi-selection remain unchanged.

9. Invoking the action does not move keyboard focus, change the active tab or pane, modify terminal input, or send any text to the running terminal application. It therefore works while a terminal-hosted chat tool is focused without relying on slash-command input.

10. Color changes made through this action are saved and restored under the same conditions as changes made through the existing tab color controls. Closing and restoring a workspace does not change the selected point in the color cycle.

11. The action is local and synchronous: it works while signed out or offline, does not display loading or permission states, and does not depend on a server response.

12. If no active tab exists when the action is dispatched, Warp safely leaves the workspace unchanged and shows no error.

13. The color indicator uses the active Warp theme's existing ANSI tab-color values. The action does not introduce hard-coded RGB values or otherwise change how tab colors are rendered.

14. Existing color controls continue to work unchanged. After using any of them, the next invocation of **Cycle current tab color** continues from the resulting visible color as specified in behavior 3–6.
