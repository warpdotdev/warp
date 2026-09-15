# GH9435: Windows corner resize hit target

## Summary
Make diagonal window resizing on Windows 11 easy to acquire from Warp window corners. Users should be able to hover near any restored Warp window corner, see the diagonal resize cursor over a forgiving target, and drag diagonally without repeated missed attempts.

## Problem
Warp uses a custom undecorated window frame on Windows. The current diagonal corner resize target feels much narrower than common Windows desktop apps, so users frequently land in an adjacent edge zone or app content instead of starting a diagonal resize.

## Goals
1. Make corner-based diagonal resizing on Windows feel reliable for frequent window management.
2. Preserve existing horizontal and vertical edge resizing behavior outside the corner target.
3. Preserve existing app content, tab/titlebar, maximize, fullscreen, and touch behavior.
4. Keep the first fix invisible and behavior-focused unless a separate visual design is provided.

## Non-goals
1. Adding a persistent visible resize grip or handle in this iteration.
2. Changing the size, appearance, or layout of Warp's custom window frame.
3. Changing macOS or Linux resize behavior except to avoid regressions in shared code.
4. Replacing Warp's custom Windows frame with native Windows decorations.

## Figma
Figma: none provided. The issue includes a user video and an optional suggestion for a visible grip, but no design mock for a new visual affordance.

## Behavior
1. On Windows, when a Warp window is restored and resizable, each of the four corners exposes a diagonal resize target that is large enough to acquire with normal pointer movement. The target must not be limited to a tiny few-pixel square.

2. Hovering inside the top-left corner target shows the northwest/southeast diagonal resize cursor. Hovering inside the top-right, bottom-left, and bottom-right corner targets shows the corresponding diagonal resize cursor for that corner.

3. Pressing and dragging the left mouse button while the pointer is inside a corner target starts a diagonal resize immediately. The drag resizes both adjacent window dimensions in the expected Windows direction until the user releases the mouse button.

4. Corner targets take precedence over adjacent edge targets. If the pointer is near both a horizontal edge and a vertical edge of the same corner, the cursor and drag action are diagonal, not horizontal-only or vertical-only.

5. Adjacent edge resize behavior remains available outside the expanded corner target:
   - Near the left or right edge away from corners, Warp shows and starts horizontal resizing.
   - Near the top or bottom edge away from corners, Warp shows and starts vertical resizing.

6. App content remains normally clickable outside the resize target. The fix must not create a broad invisible border that makes buttons, text selection, pane interactions, or other content near the window edge feel hard to click.

7. Moving the pointer between corner, edge, and non-resize regions updates the cursor promptly. The cursor should not flicker between diagonal and edge resize while the pointer remains inside the intended corner target.

8. When the pointer leaves all resize regions, Warp returns control of the cursor to normal app behavior. Text fields, links, buttons, panes, and other app surfaces should recover their usual cursor shapes after the pointer leaves the window resize target.

9. The titlebar drag region remains usable. Away from the top corners, dragging the top titlebar area continues to move the window. At the top corners, diagonal resizing takes precedence over moving the window.

10. Maximized and fullscreen windows do not expose resize targets. After restoring from maximized or fullscreen, corner and edge resizing become available again without restarting Warp.

11. Windows snap and restore flows continue to behave normally. If Windows allows the current restored or snapped window state to be resized from a corner, Warp's diagonal corner target should remain acquireable.

12. The corner target feels consistent across common Windows display scaling settings, including 100%, 125%, 150%, and 200%. A high-DPI display should not make the practical target feel like only a few physical pixels.

13. The behavior is the same for all four corners and for windows on secondary monitors.

14. The behavior applies only to Warp windows using the custom undecorated frame. If Warp ever uses native OS decorations for a window because of a platform workaround, the native frame owns resize behavior.

15. Touch input behavior does not change in this iteration. The existing mouse-specific resize path remains the only path affected by this fix.

16. There is no new visible corner handle in this iteration. Users discover the improved target through standard Windows cursor feedback when hovering near a corner.
