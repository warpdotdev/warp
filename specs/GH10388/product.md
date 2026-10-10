# GH-10388: Hide Mouse Cursor While Typing on macOS

## Summary

Warp should match normal macOS text-entry behavior: while the user types in a terminal window, the mouse cursor hides until the mouse moves. The behavior is enabled by default on macOS and can be disabled from Settings.

## Behavior

1. On macOS, typing in a Warp terminal window hides the mouse cursor by default.

2. The cursor reappears when the user moves the mouse.

3. Warp exposes a Settings -> Features -> Terminal Input toggle labeled `Hide mouse cursor while typing`.

4. When the toggle is on, typing hides the cursor until mouse movement.

5. When the toggle is off, typing leaves the cursor visible.

6. Toggling the setting applies without restarting Warp and updates currently open windows.

7. New, restored, transferred, and Quake Mode windows use the current setting value when they are created.

8. The setting is macOS-only; non-macOS platforms do not show the setting and get no behavior change.

9. The setting is a public preference at `terminal.input.hide_cursor_while_typing` and syncs per platform.

10. Text input and mouse interactions otherwise behave as they do today, including keybindings, dead keys, IME composition, selection, dragging, scrolling, hover states, and cursor restoration on mouse movement.
