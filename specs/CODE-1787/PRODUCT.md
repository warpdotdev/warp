# Windows Quake Mode: Focus and Sizing Fix — Product Spec
Linear: CODE-1787

## Summary
When the quake mode hotkey is pressed while a non-Warp application has focus on Windows, the quake window should appear with correct size and receive keyboard focus. Currently, the window appears but does not receive focus, and its size falls back to a hardcoded default instead of matching the configured screen percentage.

## Behavior

1. Pressing the quake mode hotkey while a non-Warp application has foreground focus must show the quake window and transfer keyboard focus to it. The previously focused application must lose foreground focus.

2. Pressing the quake mode hotkey while a Warp window (non-quake) has focus must continue to show the quake window with correct focus, as it does today.

3. When the quake window is shown from a hidden state, its size must match the configured width and height percentages of the display, regardless of whether Warp or another application had focus before the hotkey was pressed.

4. The quake window must appear on the monitor that contains the application which had keyboard focus when the hotkey was pressed — not necessarily the monitor the cursor is on, and not a hardcoded fallback.

5. On single-monitor setups, invariants 3 and 4 reduce to: the quake window always appears at the correct configured size on the only display.

6. These invariants apply on Windows only. macOS quake mode behavior is unchanged.
