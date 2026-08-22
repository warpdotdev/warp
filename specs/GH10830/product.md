# Keep Completion Arrow Navigation Preview-Only — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/10830
Figma: none provided

## Summary

When Warp's completion suggestions menu is open, pressing Up or Down should move the
highlight without inserting the highlighted completion into the terminal input. The
typed command remains unchanged until the user performs an existing explicit
completion action such as Tab, Shift-Tab, Enter, or clicking a suggestion.

This fixes a reproducible completion-navigation bug reported on macOS and Windows.
It does not redesign the menu or change how suggestions are generated, ranked,
accepted, or executed.

## Goals

- Make Up and Down navigation through completion suggestions non-destructive.
- Preserve the user's typed command, cursor, selection, and undo history while the
  highlighted completion changes.
- Preserve existing explicit completion behavior for Tab, Shift-Tab, Enter, and
  mouse selection.
- Apply the same behavior whether the completion menu was opened manually or by
  completions-as-you-type.
- Keep existing completion menu visuals, focus, accessibility announcements,
  ranking, filtering, and cycling boundaries.

## Non-goals

- Redesigning the completion menu or adding an inline/ghost-text preview.
- Changing completion generation, matching, ordering, icons, descriptions, or
  replacement spans.
- Changing which keys open the completion menu or introducing new accept keys.
- Changing history navigation, workflow enum suggestions, slash commands, or other
  menus that reuse suggestion infrastructure.
- Changing shell-native completion behavior outside Warp's completion menu.

## Behavior

1. When the completion suggestions menu is open, pressing Down moves the highlight
   to the next completion and pressing Up moves it to the previous completion.
   Neither action changes the terminal input buffer.

2. The first Up or Down press in a completion menu with no selected item highlights
   the first item without inserting it. Further Up or Down presses continue using
   the menu's existing wraparound order.

3. Arrow navigation preserves the exact command text that was present immediately
   before the keypress, including whitespace, quoting, escaping, path separators,
   and text after the completion replacement span.

4. Arrow navigation preserves the input cursor position, any active editor
   selection, and the editor's undo/redo history. Moving the highlight is a menu
   state change, not an editor edit.

5. The highlighted row remains the only visual preview. Its existing selected
   styling, details panel, scrolling behavior, and accessibility announcement are
   unchanged. No candidate text is rendered into the terminal input as a preview.

6. Tab and Shift-Tab retain their existing completion behavior. After arrow
   navigation, Enter or a click accepts the currently highlighted item. Tab and
   Shift-Tab preserve their existing advance-then-apply classic-cycle semantics:
   they first move in the requested direction and then apply that candidate. In
   regular completions they retain their current selection behavior.

7. Enter retains its existing behavior: when a completion is selected, it accepts
   that completion according to the current completion mode. Clicking a completion
   likewise selects and confirms it through the existing mouse interaction.

8. Pressing Escape after using only Up or Down closes the completion menu and leaves
   the command, cursor, selection, and undo/redo history exactly as they were before
   arrow navigation.

9. If the user types, deletes, or pastes after arrow navigation, that edit starts
   from the user's unchanged command rather than from the highlighted completion.
   Existing filtering and stale-result dismissal rules then apply.

10. If completion results refresh asynchronously while the menu is open, the
    refreshed menu may update or clear its highlighted row according to existing
    behavior, but it does not insert a completion solely because selection changed.

11. The behavior is the same for completion menus opened by the configured manual
    completion keybinding and by completions-as-you-type.

12. The behavior is the same on all platforms that use this completion UI,
    including macOS and Windows, and across supported completion modes and
    settings. Regular completions remain non-destructive during arrow navigation.

13. Up and Down continue to act as editor/history navigation when the completion
    menu is closed. History-menu selection continues to populate the input as it
    does today; this change is scoped only to completion suggestions.

14. Focus remains in the terminal input while the completion menu is open. Existing
    assistive-technology announcements continue to identify the newly highlighted
    suggestion without announcing it as accepted.
