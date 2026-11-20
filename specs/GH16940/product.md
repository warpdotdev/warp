# Product Spec: Spaces — switchable sets of tabs within a window

**Issue:** [warpdotdev/warp#16940](https://github.com/warpdotdev/warp/issues/16940)
**Figma:** none provided. A working prototype exists behind the `Spaces` feature flag; a GIF will be attached to the issue.

## Summary

A **space** is a named, colored set of tabs and tab groups inside one Warp window. A window shows one space at a time; switching spaces swaps the whole tab list in one keystroke, palette command, click, or two-finger trackpad swipe, and the new list slides in from the side of the space you switched to. Sessions in hidden spaces keep running. The model is borrowed from Arc / Zen / Dia browser spaces and Vivaldi workspaces.

## Problem

People who work across several projects, clients, or long-running agent sessions accumulate dozens of tabs. Today the options are:

- **One window per context.** Contexts are invisible until you hunt for the right window (see #16305); window switching is an OS-level, unlabeled operation.
- **Tab groups.** Groups label related tabs, but every group stays in the same list. Collapsing a group hides its rows, not the noise: the list, Ctrl+Tab, and tab cycling still span every context.
- **Vertical tabs search.** Good for finding one tab, not for *staying* in a context.

What is missing is a cheap way to say "now I'm working on X" and see only X — without closing or restarting anything from Y.

## Goals

- Switch the entire visible tab list between contexts in one action.
- Keep every session in hidden spaces alive and reachable.
- Fast switching from keyboard, command palette, mouse, and trackpad.
- Organize by direct manipulation: drag tabs onto a space, drag spaces to reorder them.
- Zero change for users who don't opt in.

## Non-goals

- Per-space settings: theme, default directory, shell, or profile. Possible follow-ups.
- Moving a space between windows, or spaces as entities independent of a window (#9382 D1/D2).
- Multiple sessions stacked inside one tab (#9382 D3 option 2).
- Automatic grouping by repository (#9875).
- CLI / URI scheme for spaces.
- A swipe animation that follows the fingers. v1 switches when the swipe completes, then slides.

## User experience

With the `Spaces` feature enabled, the bottom of the vertical tabs panel shows a switcher row:

```
┌─ Warp ─────────────────────────────────────────┐
│ ▸ suslicketeam            │ $ pnpm dev          │
│   ├ dev server            │ ready on :3000      │
│   └ claude                │                     │
│ ▸ tests                   │                     │
│                           │                     │
│ ───────────────────────── │                     │
│ [● Work] [ Infra] [ Pet] + │                     │
└────────────────────────────────────────────────┘
```

- **Switch:** click a chip, run **Switch to space 1–9** / **Switch to next space** / **Switch to previous space**, or swipe horizontally with two fingers over the panel. The tab list slides in from the side of the target space.
- **Create:** `+` (or **Create new space**) adds a space at the end and switches to it.
- **Rename and recolor:** double-click a chip to rename it inline, or right-click it for **Rename space**, a row of color dots, and **Delete space** (hidden while only one space exists).
- **Reorder:** drag a chip along the row.
- **Move tabs:** use **Move to space ▸**, next to **Move to group** in the tab, group, and multi-selection context menus, or drag a tab or group onto another space's chip.
- **Find:** the Ctrl+Tab switcher covers every space and labels entries with their space once there are two or more.
- With horizontal tabs, the tab bar shows only the active space. There are no chips, so switching and management go through keybindings, the command palette, and the tab context menu.

## Behavior invariants

### Model

1. With the `Spaces` feature disabled, tabs, persistence, keybindings, and UI behave exactly as today.
2. With it enabled, every window has at least one space. A new window starts with a single space with no color. Unnamed spaces show a positional placeholder ("Space N"), the same way unnamed tab groups do. Each new space is added at the end of the switcher with the next color from the tab color palette.
3. Every tab belongs to exactly one space of its window. All members of a tab group belong to the same space.

### Switching

4. The tab list (vertical and horizontal) shows only the active space's tabs and groups.
5. Switching to a space activates the tab that was active when that space was last left.
6. Switching never closes, restarts, or pauses a session. Processes in hidden spaces keep running and producing output. A hidden tab whose session exits is removed from its space.
7. Switching to a space with no tabs opens a new terminal tab in it. This is a plain terminal tab, not the configured default session mode (see Open questions).
8. **Switch to space N** (N = 1–9) does nothing if space N does not exist. **Next space** / **Previous space** do not wrap at the ends.
9. Any jump to a specific tab from outside the tab list switches to that tab's space first and activates the tab. This includes the Ctrl+Tab switcher, notification clicks, session and conversation navigation, and deep links.

### Switch animation

10. While the vertical tabs panel is shown, every switch slides the new space's tab list in: from the right when the target space is later in the switcher, from the left when it is earlier. This covers chip clicks, keybindings, palette commands, swipes, jumps (9), creating a space, and the automatic switch in 24. The slide spans the list's width, takes 200 ms with an ease-out curve, and moves only the tab list, not the control bar or the switcher row.
11. A switch during a slide restarts the slide toward the new target. Slides do not queue.
12. There is no slide while the vertical tabs panel is hidden (horizontal tabs, or panel closed), when a window is restored, or when deleting the active space moves you to its neighbor.

### Trackpad swipe

13. A horizontal two-finger swipe over the vertical tabs panel (tab list, control bar, or switcher row) switches to the adjacent space. One swipe switches at most once. Only finger movement counts: the momentum scrolling after the fingers lift neither completes a swipe nor switches again.
14. Each new touch on the trackpad starts a new gesture, so quick repeated swipes each switch once, even while the previous swipe's momentum is still arriving.
15. When the platform reports no gesture phase, a pause of at least 200 ms between scroll events ends a gesture instead. Today only macOS reports phases.
16. Vertical scrolling, non-precise wheel input (including Shift+wheel), and horizontal scrolling anywhere outside the vertical tabs panel never switch spaces. The tab list keeps scrolling vertically as before.
17. Swipes are ignored while a tab, group, or space chip is being dragged, while a tab, pane, group, or space is being renamed, or while the panel's search field has focus. A swipe that completes while blocked does not switch later in the same gesture.
18. Swipe direction follows the system scroll-direction setting, matching back/forward swipes in browsers. Swiping past the first or last space does nothing.

### Managing tabs and spaces

19. New tabs open in the active space, from every entrypoint: `+` button, keybinding, tab config.
20. **Move to space ▸** lists the other spaces in switcher order, then **New space**. It moves the tab, group, or multi-selection to the end of the target space; pinned tabs join the target's pinned tabs. The active space does not change.
21. A group moves with its tabs only when all of its members move. A member moved on its own leaves its group.
22. Moving the active tab away activates the tab that followed it in the source space, else the new last tab. If the source space becomes empty, 24 applies.
23. Dragging a tab or a group onto another space's chip moves it there, as **Move to space** does. The chip under the cursor is highlighted during the drag, and dropping on it does not reorder tabs. The active space does not change, and its own chip is not a drop target.
24. When the active space loses its last tab (closed, moved away, or dragged to another window): if any other space in the window has tabs, the window stays open and switches to the nearest space with tabs, preferring the previous one at equal distance. The emptied space remains in the switcher. If no other space has tabs, the window closes as it does today.
25. Deleting a space moves its tabs and groups to the end of the previous space (else the next one). No session is closed. Deleting the active space switches to that neighbor and shows the neighbor's own active tab. The last remaining space cannot be deleted, and its context menu has no Delete item.
26. Double-clicking a chip, or choosing **Rename space** in its context menu, edits the name inline in the tab group name editor; an empty name keeps the current one. The color dots set the space's color, and choosing the current color clears it. **Rename current space** in the command palette does the same for the active space while the vertical tabs panel is shown, and does nothing otherwise.
27. Dragging a chip reorders spaces. A chip moves one slot once its center passes the middle of a neighboring chip, including across wrapped rows. An unnamed space that moves keeps the placeholder it was showing, so "Space 2" stays "Space 2" in its new position.

### Search and tab switcher

28. The vertical tabs search field filters within the active space only.
29. The Ctrl+Tab switcher lists the tabs of every space in the window: the active space first, then the other spaces in switcher order, each in its own most-recently-used order. With two or more spaces, every entry is labeled with its space name and the query also matches space names. With one space, entries look as they do today. Selecting a tab from another space switches to that space (9).

### Persistence

30. The order, names, and colors of spaces (including empty ones), each space's active tab, each tab's space, and each window's active space survive an app restart.
31. On restore, a tab whose space cannot be resolved joins the window's first space. Tabs saved before this feature existed load into a single space. If the saved active space has no tabs, the nearest space with tabs is shown instead.
32. Disabling the feature after creating spaces shows every tab of the window in one list, with pinned tabs first, and no tab is lost. Space assignments are not saved while the feature is off, so re-enabling it starts from a single space.

### Window-wide behavior

33. Warnings about running sessions on window close, app quit, and logout count sessions in hidden spaces too.
34. Synced inputs and the local control API act on the visible space only, matching what the user sees.

### Cross-window

These depend on Warp's existing `DragTabsToWindows` flag, which gates dragging tabs between windows today.

35. A tab dragged into another window joins that window's active space.
36. Dragging the last tab of a space out of a window does not close the window while other spaces still have tabs. The source window switches as in 24.
37. When the dragged tab is the window's only tab and no other space has tabs, the whole window moves, as today. That drag cannot be dropped on a chip.

### Accessibility

38. Every switching action is available without a trackpad or mouse, through keybindings and the command palette.

## Open questions

- **Default keybindings.** On macOS, Ctrl+1–4 already toggle panels (Project Explorer, conversations, global search, Warp Drive), so Arc-style Ctrl+N is taken. The prototype ships every space binding unbound; maintainers may want defaults, at least for Next/Previous space.
- **New tab in an empty space.** It is a plain terminal tab (7), because some default session modes open a modal that expects an existing active tab instead of adding one. Should it honor the default session mode once that path can handle an empty tab list?
- **Ctrl+Tab order.** Active space first, then the other spaces, each by its own MRU (29). The alternative is one MRU order across the window, which could put a tab from another space second.
- **Chip accessibility.** Chips don't yet expose their name and active state to assistive technology, and a switch has no dedicated announcement. What is the preferred warpui pattern for a custom control like this?
- **Reduced motion.** Neither Warp nor warpui has a reduce-motion preference, so the slide (10) always runs. Should one be added, or should the slide follow the macOS accessibility setting?
- **Placement.** The prototype puts the switcher row at the bottom of the panel (Arc). Zen puts it at the top.
- **Attention badges.** Should a chip show a dot when a hidden space has a finished command or an agent waiting for input? Very useful for agent workflows; proposed as a follow-up to keep v1 small.
- **Relation to Project (#9233) and #9382.** A space is window-scoped and only groups visible tabs, so it does not lock in the first-class project/workspace model. It could later become "a space bound to a project".
