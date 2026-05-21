# Cross-Window Tab Drag — Product Spec

## Summary

Define the expected user-facing behavior for dragging tabs within a window, detaching tabs into new windows, and attaching dragged tabs into existing windows. The desired outcome is a continuous, Chrome-like drag interaction that preserves tab identity, keeps windows visually stable, and behaves predictably across repeated attach and detach cycles.

## Problem

Tab drag-and-drop crosses several user-visible states: in-window reordering, detach into a standalone window, attach into an existing window, continued dragging after attach, and final drop. If any transition behaves inconsistently the interaction feels broken: tabs can appear duplicated, windows can flash transparent, close-confirmation dialogs can appear during harmless transfers, or the drag can feel like it unexpectedly ends mid-gesture.

Warp needs one coherent product definition for this interaction so implementers and reviewers can preserve the same mental model across all drag states. From the user's perspective, dragging a tab should feel like one uninterrupted gesture even when the tab changes windows multiple times before mouse-up.

## Goals

- Match the core interaction model of Chrome-style tab dragging.
- Support continuous drag gestures across reordering, detach, attach, and re-detach without requiring the user to release the mouse.
- Make single-tab and multi-tab windows behave consistently from the user's perspective.
- Make drop targeting and z-order behavior predictable when windows overlap.
- Keep the UI visually stable during the drag lifecycle: no transparent windows, no misplaced hover overlays, and no placeholder states that diverge from the eventual drop result.
- Preserve user trust by avoiding data-loss prompts or duplicate tabs during tab transfer.
- Leave the resulting active tab ready for immediate typing when the drag ends.
- Apply the same product behavior whether tabs are presented in the horizontal tab bar or the vertical tabs panel.

## Non-goals

- Adding new drop targets outside a window's tab bar / vertical tabs panel.
- Changing non-tab window drag behavior outside the tab drag lifecycle.
- Redesigning the visual style of tabs, tab bars, or window chrome.
- Changing process lifetime or shell semantics during tab transfer; the feature only changes where the tab is hosted.

## Figma / design references

Figma: none provided.

## User experience

### Reference model

The intended interaction should feel like Google Chrome tab dragging. The same drag gesture can pass through multiple states without interruption, and every state change should feel like a direct consequence of the cursor position.

### Default behavior

- A drag begins from the tab itself.
- If the tab remains within a multi-tab window's tab bar / vertical panel and moves along the tab axis, the interaction is an in-window reorder.
- If the tab moves perpendicular to the tab axis past the detach threshold, the interaction becomes a detached-window drag.
- While detached, moving over an eligible window's tab bar or vertical tabs panel previews attaching the tab into that window at the current landing position.
- The drag remains active until mouse-up, even if the tab changes host windows one or more times before the drag ends.

### Single-tab windows

- If a window has exactly one tab, dragging anywhere in that tab drags the entire window.
- The tab is not treated as detached just because the drag began in a single-tab window.
- The tab title/text and in-window rendering must remain visually stable while the window follows the cursor. The interaction must not introduce text jitter, shimmer, or a second drag preview layered over the moving window.
- Once that tab is attached into another window during the same gesture, it behaves like any other dragged tab: the user may continue dragging it, detach it again, attach it to a different window, or drop it as a standalone window without releasing the mouse first.

### Multi-tab reordering

- If a window has more than one tab, dragging a tab along the tab axis reorders it within that window.
- For horizontal tabs the axis is X; for the vertical tabs panel the axis is Y.
- Reordering feedback updates continuously as the cursor moves.

### Detaching a tab

- Dragging a tab perpendicular to the tab axis past a small detach threshold removes it from its current window and creates a new window containing only that tab.
- The detached tab appears in the first position in the new window's tab bar.
- The new window is positioned so the cursor keeps the same relative offset within the dragged tab that it had at the moment of detachment.
- After detachment, the new window continues following the cursor for the remainder of the drag unless the tab is attached somewhere else or the user drops it.

### Dropping without a target window

- If the user releases the mouse while the detached tab is not over any eligible tab bar or vertical tabs panel, the detached window remains where it was dropped.
- The dropped result is immediately a normal, usable single-tab window.

### Attaching to a window

While still dragging, the user may move the detached tab over the tab bar or vertical tabs panel of either the original window or any other existing window. Attaching to the original window and attaching to a different window behave the same way visually.

When the cursor enters another window's tab bar or vertical panel, two visual elements appear inside that target window:

1. **Floating chip** — a small tab-shaped element (icon + title, the same visual as a real tab) that follows the cursor in real time. It looks and moves exactly like dragging a tab within the same window.
2. **Insertion slot** — an empty space with the standard insertion-slot background appears in the target's tab bar / vertical panel at the position where the tab will land if dropped. This matches what same-window drag shows for the dragged tab's origin slot.

If the user releases the mouse at that moment, there is no additional visual jump because the chip is already shown in its final position. The user does **not** see:

- the entire source / preview window overlaid on top of the target window
- a static non-moving placeholder stuck at one position in the tab list
- visible flicker or stutter as the cursor moves across the tab bar

### State transitions after attach

- Releasing over a target tab bar / vertical panel attaches the dragged tab into that window. Releasing over empty space leaves the detached window where it was dropped.
- During an active drag, moving the cursor off a target's tab bar reverts to the detached-window state without releasing.
- Reversing direction must behave predictably: if the user drags into a target, the floating chip / insertion slot appears; if they then drag back out, the chip / slot disappears and the detached window resumes following the cursor with no duplicate entries left behind.
- This cycle may repeat any number of times in a single uninterrupted drag.
- Dragging back over the tab bar of the **original** window the tab came from behaves the same as attaching to a different window from the user's perspective: the tab appears in its landing position and reorders continuously.

### Reordering after attach

Once a dragged tab is attached into a target window (or into the source window's own tab bar via drag-back), movement along that target's tab axis reorders the tab in real time just like a normal in-window reorder. This must feel the same regardless of whether the tab came from another multi-tab window, a single-tab window, or the same window it is currently in.

### No close confirmation during transfer

If a window closes as part of the tab drag lifecycle, Warp must not show the "Close window?" confirmation dialog during that transfer. This applies when:

- a source window loses its last tab
- a temporary preview or transfer window is cleaned up after a handoff

Running processes continue in the tab's new home window. The transfer should not be treated as user intent to close those processes.

### Single-tab handoff behavior

- When a tab that started in a single-tab window is attached into another window, the drag must continue seamlessly.
- The user must be able to drag that same tab back out again, attach it elsewhere, or drop it as a standalone window, all within the same gesture.
- The transfer must not cause visible duplicate windows, broken drag tracking, or any interruption that makes the user release and restart the drag.

### Hover behavior during drag

While any tab drag is in progress, hover-only overlays inside tabs must not appear. This includes elements such as tab tooltips and close-button hover treatments. The dragged tab must not show overlays that appear mispositioned relative to the moving tab.

### Z-order and drop targeting

- A dragged tab may only attach into a window whose tab bar / vertical panel is actually reachable based on the current z-order.
- The valid drop target is the window directly below the dragged preview window in z-order.
- If another window sits in front of a potential target and occludes its tab bar, that front window blocks the drop.
- The dragged tab must not pass through an intervening window to attach to a window behind it.

### Window opacity and content readiness

- No visible window may become transparent or appear see-through during any part of the drag sequence.
- When a tab is detached from a multi-tab window, the source window must immediately show another valid tab so it continues rendering normal content.
- The new detached preview window must not appear onscreen until its tab content is ready to render.
- Users should never observe a transparent flash or blank window caused by the new window appearing before its content is ready.

### Input focus after drag ends

- When the drag completes, the resulting active tab must have terminal input focus.
- This applies whether the tab ended as a standalone detached window, a tab attached into another window, or a tab attached back into its original window.
- The user should be able to type immediately without clicking again.

### Invariants

- The dragged tab always represents one real tab, never a duplicate.
- The chip and insertion slot in a target window during attach reflect the dragged tab's identity (same icon and title) and its live landing position, not a placeholder.
- The total number of tab instances across all windows remains constant throughout the drag lifecycle.
- No visible window flashes transparent, blank, or otherwise appears incomplete during transfer.
- Any window that closes solely because of tab transfer closes silently, without treating the transfer as destructive user intent.
- The resulting active tab is always usable immediately after mouse-up.
- Hovering over a target window for any length of time has no observable cost beyond redrawing the chip and slot — terminal output, agent activity, and animations in either window must continue at their normal frame rate.

## Success criteria

1. Dragging the only tab in a window moves the whole window until that tab is attached into another window, and starting that drag from the tab itself does not cause visible tab-text jitter or layered-preview artifacts while the window moves.
2. Dragging a tab along the tab axis inside a multi-tab window reorders it in real time. This applies to both the horizontal tab bar and the vertical tabs panel.
3. Dragging a tab perpendicular to the tab axis past the detach threshold creates a new single-tab window that follows the cursor while preserving the cursor's relative grab position within the tab.
4. Releasing a detached tab when it is not over an eligible tab bar or vertical tabs panel leaves it as a normal standalone single-tab window.
5. Moving a detached tab over an eligible target window shows a floating chip following the cursor and an insertion slot at the predicted drop position, with no placeholder or ghost state diverging from the eventual drop.
6. After an attach, the user can keep dragging without releasing and can reorder, detach, or attach the same tab again in one continuous gesture.
7. After a successful drop into a target tab bar, that target window now hosts the real tab in the final position; the floating chip and insertion slot disappear with no visible jump.
8. Hovering over a target window's tab bar with the drag in progress does not visibly slow the target window (no terminal lag, no animation hitches) — the visual feedback is rendered by the target without involving the dragged tab's pane group.
9. During tab transfer, Warp never shows a close-confirmation dialog for windows that close only because the tab changed host windows.
10. Single-tab-origin drags behave continuously across attach and detach cycles without interruption or duplicate windows.
11. No hover-only tab overlays appear while a drag is active.
12. Drop targeting respects z-order: an occluded window cannot receive the drop through a window in front of it.
13. No visible window becomes transparent, flashes blank, or shows invalid content during detach or handoff.
14. When the drag ends, the resulting active tab is focused and ready to accept typing immediately.
15. The total tab count across all windows stays constant throughout repeated attach/detach cycles, including drops over the same window the tab originally came from.

## Validation

- **Single-tab drag**: Start with a window that has one tab. Drag the tab and verify the whole window moves with no tab-text jitter. Attach it into another window, then drag it back out again without releasing first and verify the gesture continues normally.
- **In-window reorder (horizontal)**: In a multi-tab window with horizontal tabs, drag a tab left and right and verify the order updates continuously.
- **In-window reorder (vertical)**: With vertical tabs enabled, drag a tab up and down inside the panel and verify the order updates continuously.
- **Detach behavior**: Drag a tab perpendicular to the axis out of a multi-tab window and verify a new single-tab window appears, follows the cursor, and preserves the original cursor grab offset within the tab.
- **Drop with no target**: Detach a tab and release it over empty space. Verify it remains as a usable standalone window at the dropped location.
- **Attach behavior (chip + slot)**: Detach a tab and drag it over another window's tab bar. Verify the floating chip follows the cursor and an insertion slot is shown at the expected landing position. Move the cursor across the tab bar and verify the slot moves between tabs in real time. Verify the target window's terminal output and any in-progress animations remain smooth during this hover.
- **Vertical-target attach**: Repeat the previous step where the target window has vertical tabs enabled. Verify the chip follows the cursor and the insertion slot appears between rows in the vertical panel.
- **Drop into target**: Release the mouse while over the target's tab bar / vertical panel and verify the real tab appears at the indicated position with no extra visual jump and no duplicate.
- **Repeated attach/detach cycle**: During one uninterrupted drag, attach a tab into a target window, reorder it there, drag it back out, and attach it into a different window. Verify the drag remains continuous and no duplicate tabs appear.
- **Drop back over source**: Detach a tab, drag it over the source window's tab bar, and release. Verify the tab is reinserted into the source at the expected position (not promoted to a new empty window) and the total tab count is unchanged.
- **Single-tab handoff**: Repeat the previous validations starting from a single-tab window. Verify the same continuity and no duplicate or stuck windows.
- **No close dialog**: Perform transfers involving tabs with running processes and verify no "Close window?" dialog appears as transfer windows close.
- **Hover suppression**: While dragging a tab, move across other tabs and verify tooltips and hover-only close affordances do not appear.
- **Z-order targeting**: Arrange overlapping windows so one tab bar is visually blocked by another window. Verify the blocked window cannot receive the dragged tab until it is actually the reachable top target.
- **No transparency**: During detach and handoff, verify that neither the source window nor the detached window flashes transparent or blank.
- **Focus after drop**: Complete the drag in each final state (standalone, other window, original window) and verify the terminal input is focused with a blinking cursor and accepts typing immediately.
- **Tab count stability**: Repeatedly drag a tab between windows and back out again, then verify the total number of tabs across windows matches the starting count.

## Open questions

None.
