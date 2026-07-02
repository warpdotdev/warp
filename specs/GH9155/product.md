# Product Spec: Search sessions by renamed tab and pane name

**Issue:** [warpdotdev/warp#9155](https://github.com/warpdotdev/warp/issues/9155)
**Figma:** none provided

## Summary

The session navigation palette (`Cmd+P`) should let users find a session by the
name they assigned its tab or pane. Today the palette matches only on working
directory, running/last command, and status, so a session a user renamed to
`deploy` is not findable by typing `deploy` unless its directory or command also
contains that text. This makes renaming sessions far less useful for navigation,
which is the main reason users rename them.

## Problem

Users with many open sessions rename tabs and panes to meaningful labels
(`meetings`, `frontend`, `deploy`) precisely so they can jump back to them. The
tab strip and vertical-tabs sidebar display those names, but the `Cmd+P` session
navigation palette does not search them: typing a custom name returns
"No Results Found" when the directory/command do not also match. Users fall back
to `Cmd+1/2/3` positional switching or visual scanning, defeating the purpose of
naming.

A custom name is also not shown in the palette result row, so even when a session
appears, there is nothing tying it to the name the user remembers.

## Goals

- A session is findable in the navigation palette by its custom **tab** name
  (set via rename-tab).
- A session is findable by its custom **pane** name (set via rename-pane).
- The palette row shows the user-assigned name so a name-matched result is
  recognizable next to its directory.
- Sessions without a custom name behave exactly as before.

## Non-goals

- The sidebar "Search tabs…" Panes-mode filter (tracked separately in #9666).
- Auto-generated terminal titles (e.g. `user@host:~`) becoming searchable — only
  user-assigned names are added.
- Highlighting the matched characters within the name (the directory/command
  highlight behavior is unchanged; the name is matchable and displayed but not
  range-highlighted).
- Renaming UX, persistence, or where names are stored — unchanged.

## User experience

### Current behavior

1. User renames a tab to `deploy` (its directory is `~/project`).
2. User opens the navigation palette and types `deploy`.
3. "No Results Found" — the name is not searched.

### Expected behavior

1. User renames a tab to `deploy`.
2. User opens the navigation palette and types `deploy`.
3. The session under the `deploy` tab appears, with `deploy` shown on its row.
4. The same holds for a pane renamed to `deploy`.

## Behavior invariants

1. Typing a substring of a session's custom **tab** name surfaces that session
   in the navigation palette.
2. Typing a substring of a session's custom **pane** name surfaces that session.
3. A matched session's row displays the user-assigned name: the custom pane name
   if set, otherwise the custom tab name.
4. When both a custom tab name and a custom pane name are set, both are
   searchable, and the displayed name is the pane name (more specific).
5. A session with no custom tab or pane name shows no extra name line, and its
   matching behavior (prompt/command/status) is unchanged from today.
6. Renaming a tab or pane updates what is searchable/displayed without requiring
   the palette to be reopened from scratch (the palette reads live session data
   each query).
7. The change applies regardless of which session-search backend is active
   (fuzzy or full-text), so results are consistent across configurations.
8. Empty custom names (e.g. cleared back to default) are treated as no custom
   name (invariant 5 applies).
