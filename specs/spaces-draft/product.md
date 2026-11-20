# Product Spec: Spaces — switchable sets of tabs within a window

**Issue:** TBD (rename this folder to `specs/GH<issue-number>/` once filed)
**Figma:** none provided (a working prototype GIF will be attached to the issue)

## Summary

A **space** is a named, colored set of tabs and tab groups inside one Warp window. A window shows one space at a time; switching spaces swaps the whole tab list in one keystroke, palette command, click, or two-finger trackpad swipe. Sessions in hidden spaces keep running. The model is borrowed from Arc / Zen / Dia browser spaces and Vivaldi workspaces.

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
- Zero change for users who don't opt in.

## Non-goals

- Per-space settings: theme, default directory, shell, or profile. Possible follow-ups.
- Moving a space between windows, or spaces as entities independent of a window (#9382 D1/D2).
- Multiple sessions stacked inside one tab (#9382 D3 option 2).
- Automatic grouping by repository (#9875).
- CLI / URI scheme for spaces.

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

- Clicking a chip, running **Switch to space N** / **Next space** / **Previous space**, or swiping horizontally with two fingers over the panel switches spaces.
- **Move to space ▸** appears next to **Move to group** in the tab, group, and multi-selection context menus.
- `+` creates a space; right-clicking a chip offers Rename, Change color, and Delete.
- With horizontal tabs, the tab bar shows only the active space; switching is via keybindings and the command palette.

## Behavior invariants

### Model

1. With the `Spaces` feature disabled, tabs, persistence, keybindings, and UI behave exactly as today.
2. With it enabled, every window has at least one space. A new window starts with a single space shown with a placeholder name, the same way unnamed tab groups use one.
3. Every tab belongs to exactly one space of its window. All members of a tab group belong to the same space.

### Switching

4. The tab list (vertical and horizontal) shows only the active space's tabs and groups.
5. Switching to a space activates the tab that was active when that space was last left.
6. Switching never closes, restarts, or pauses a session. Processes in hidden spaces keep running and producing output.
7. Switching to a space with no tabs opens a new tab in it, using the normal new-tab behavior.
8. **Switch to space N** (N = 1–9) does nothing if space N does not exist. **Next space** / **Previous space** do not wrap at the ends.
9. Any jump to a specific tab from outside the tab list switches to that tab's space first. This includes notification clicks, the Ctrl+Tab switcher, and other jump-to-pane actions.

### Trackpad swipe

10. A horizontal two-finger trackpad swipe over the vertical tabs panel switches to the adjacent space. One gesture, including its momentum tail, switches at most once.
11. Vertical scrolling, non-precise wheel input (including Shift+wheel), and horizontal scrolling anywhere outside the vertical tabs panel never switch spaces.
12. Swipes are ignored while a tab is being dragged, a tab or group is being renamed, or the panel's search field has focus.
13. Swipe direction follows the system scroll-direction setting, matching back/forward swipes in browsers.

### Managing tabs and spaces

14. New tabs open in the active space, from every entrypoint: `+` button, keybinding, tab config.
15. **Move to space ▸** moves the selected tab, group, or multi-selection to the chosen space, or to a new one. Moving a group moves all its members. The active space does not change.
16. Moving the active tab away activates a neighboring tab in the source space. If the source space becomes empty, invariant 17 applies.
17. Closing the last tab of a space: if any other space in the window has tabs, the window stays open and switches to the nearest non-empty space, preferring the previous one. The empty space remains in the switcher. If no other space has tabs, the window closes as it does today.
18. Deleting a space moves its tabs and groups to the neighboring space (previous, else next). No session is closed. The last remaining space cannot be deleted.
19. Creating, renaming, and recoloring spaces reuses the tab group name editor and color palette.

### Search

20. The vertical tabs search field filters within the active space only.
21. The Ctrl+Tab tab switcher lists tabs from all spaces of the window, each labeled with its space. Selecting a tab from another space switches to that space (invariant 9).

### Persistence

22. The order, names, and colors of spaces, each tab's space, and each window's active space survive an app restart.
23. On restore, a tab whose space cannot be resolved joins the window's first space. Tabs saved before this feature existed load into a single space.
24. Disabling the feature after creating spaces shows every tab of the window in one list, and no tab is lost. Space assignments may be reset if the feature is re-enabled later.

### Cross-window

25. A tab dragged into another window joins that window's active space.
26. Dragging the last tab of a space out of a window does not close the window while other spaces still have tabs.

### Accessibility

27. Every switching action is available without a trackpad or mouse, through keybindings and the command palette.
28. Each chip exposes its space name and active state to assistive technology.

## Open questions

- **Default keybindings.** On macOS, Ctrl+1–4 already toggle panels (Project Explorer, conversations, global search, Warp Drive), so Arc-style Ctrl+N is taken. Proposal: leave **Switch to space N** unbound by default and pick a default for Next/Previous space during review.
- **Attention badges.** Should a chip show a dot when a hidden space has a finished command or an agent waiting for input? Very useful for agent workflows; proposed as a follow-up to keep v1 small.
- **Placement.** Switcher row at the bottom of the panel (Arc) or at the top (Zen)?
- **Relation to Project (#9233) and #9382.** A space is window-scoped and only groups visible tabs, so it does not lock in the first-class project/workspace model. It could later become "a space bound to a project".
