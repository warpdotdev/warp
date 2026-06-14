# Cross-Window Tab Drag Redrag — Product Spec

## Summary
Users should be able to drag a terminal tab out of a window, drag it back into the source window, and then drag it out again during the same uninterrupted mouse gesture. The drag should remain continuous until mouse-up, regardless of how many times the tab crosses between floating preview and source-window insertion states.

## Problem
Cross-window tab dragging currently supports the first detach and the first put-back within a single mouse-down gesture:
- dragging a tab out creates a preview window
- dragging it back into the source window hides the preview and puts the tab back into the source

After that put-back, dragging the same tab out again without releasing does not correctly continue the drag. The preview does not reappear as a usable dragged preview, and attempted fixes have produced additional visual bugs. This makes the interaction feel broken compared to browser-style tab dragging, where the user can move a tab in and out of a window continuously before deciding where to drop it.

## Goals
- Preserve a continuous drag across repeated out → in → out transitions during the same mouse-down gesture.
- Re-show the preview window when a tab that was put back into the source is dragged out again before mouse-up.
- Keep the drag bound to the same logical tab throughout the gesture.
- Avoid duplicate tabs, duplicate pane-group ownership, stale placeholders, and incorrect source-tab cleanup.
- Keep visual state coherent while transitioning:
  - exactly one visible dragged representation at a time
  - no stale zero-width tab placeholder after put-back
  - no hidden preview when the tab is logically floating
  - no ghost insertion state in a window that no longer owns the dragged tab
- Preserve existing behavior for simpler cases:
  - drag out and drop as a new window
  - drag out and drop into another window
  - drag out and back into the source, then release
  - single-tab source-window dragging

## Non-goals
- Redesigning tab dragging visuals.
- Supporting simultaneous multi-tab dragging beyond the existing implementation model.
- Changing drag thresholds or tab insertion-index behavior except where required for correctness.
- Changing persistence or session restoration behavior beyond avoiding duplicate state during the drag.
- Adding new user-facing controls or preferences.

## User Experience

### Primary redrag flow
Given a source window with multiple tabs:
1. The user presses on a tab and drags it outside the tab bar.
2. Warp creates a floating preview window that follows the cursor.
3. The user drags the tab back over the source window tab bar.
4. Warp hides or de-emphasizes the preview and reinserts the dragged tab into the source tab bar at the hovered index.
5. Without releasing the mouse, the user drags that same tab outside the source tab bar again.
6. Warp restores the floating preview window and continues the drag.
7. The user can drop the tab as a new window, drop it into the source, or drop it into another window.

The user should not need to release and start a second drag to detach the tab again.

### Repeated transitions
The same gesture may cross the boundary multiple times:
- out → in → out
- out → in → out → in
- out → in → out → other window
- out → other window → out → source → out

Warp should treat these as state transitions of one continuous drag, not as separate drags that require a new mouse-down.

### Visual behavior
When the tab is floating:
- the preview window is visible and follows the cursor
- the source window does not render a stale dragged placeholder for a tab it no longer owns
- target windows may show normal ghost insertion visuals

When the tab is inserted back into the source during the same drag:
- the preview window is hidden or visually inactive
- the source window renders the dragged tab in the tab bar
- the dragged tab continues to move with the cursor within the tab bar
- the source tab list should not contain a duplicate placeholder for the same pane group

When the tab is dragged out again:
- the preview becomes visible again
- the source removes or suppresses the inserted dragged tab representation as appropriate
- the visual drag overlay continues from the current cursor position without jumping or disappearing

### Drop behavior
On mouse-up, the final visible/logical state determines the result:
- If floating outside any target, promote the preview to a normal window.
- If inserted in the source, keep the tab in the source and close or discard any temporary preview.
- If inserted or ghosted over another window, attach the tab to that window.

Drop handling must not remove an unrelated source tab because a stored source index became stale earlier in the gesture.

## Edge Cases
- Source window has exactly one tab.
- Source window has multiple tabs and the dragged tab is not the active tab.
- Dragged tab is moved back into a different source insertion index than its original index.
- User drags rapidly across the source tab bar boundary, causing multiple drag events while a transition is in progress.
- User releases during or immediately after a transition.
- Preview window is hidden, alpha-zeroed, or behind another window when it needs to reappear.
- Vertical tabs and horizontal tabs both participate in attach-target hit testing.
- A target workspace is unavailable because the workspace is already borrowed during the current event handler.

## Success Criteria
- The primary out → in → out same-mousedown flow works reliably.
- The second drag-out visibly recreates or restores a floating preview.
- The drag remains continuous; mouse-up after the second drag-out promotes the preview to a new window.
- Mouse-up after dragging back into the source keeps the tab in the source and does not leave a preview window behind.
- Mouse-up over another target window attaches the tab to that window.
- No duplicate tab state is persisted.
- No stale source index removes the wrong tab.
- No visual ghost or preview remains after drop finalization.
- Existing first-drag and first-put-back behavior does not regress.

## Validation
- Manual test with a multi-tab source window:
  - drag tab out
  - drag it back into the source
  - drag it out again without releasing
  - release outside all windows and verify the preview becomes a normal window
- Manual test with the same sequence, releasing after dragging back into the source.
- Manual test with the same sequence, dropping into a different existing window after the second drag-out.
- Manual test with vertical tabs enabled.
- Manual test with horizontal tabs enabled.
- Log validation should show a coherent state transition for the second drag-out rather than a missing-workspace fallback.
