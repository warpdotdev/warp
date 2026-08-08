# Product Spec: Tab search matches tab group names

**Issue:** [warpdotdev/warp#14689](https://github.com/warpdotdev/warp/issues/14689)
**Figma:** none provided

## Summary

The vertical tabs panel's search field should match tab group names. When a query matches a group's displayed name, that group and every tab under it appear in the filtered list, regardless of whether the individual tab titles match. Today the field matches only pane and tab text, so a group named `backend` cannot be found by name and there is no way to use search to pull up everything inside it.

## Problem

With the `GroupedTabs` feature, a user can organize tabs into named groups. Naming a group creates a single handle for a set of tabs — but search, the fastest path to a tab, does not recognize that handle.

Typing `backend` surfaces whichever tabs happen to have "backend" in their own titles and silently omits the group named `backend` along with its members. The more a user leans on groups, the worse this gets: the tabs inside a well-named group often have titles that share nothing with the group name (`api server`, `worker`, `db shell`), which is exactly when the group name is the only thing the user remembers.

There is a second, compounding problem. If a group is collapsed, its members are hidden behind the header. Even a member that *does* match the query by its own title stays invisible, so the search appears to return nothing when a match exists.

## Goals

- A query matching a group's displayed name surfaces that group and all of its member tabs.
- Matches inside a collapsed group are visible without the user having to expand it first.
- Tabs outside a matched group continue to filter on their own text, unchanged.
- Search matches what the user can actually read on screen, including the placeholder name shown for a group that was never renamed.

## Non-goals

- The Ctrl+Tab command palette tab switcher (`app/src/search/command_palette/tabs/`). It is a flat MRU list with no group concept; adding group awareness there needs new item types and grouped result rendering. Separate work.
- Fuzzy or ranked matching. The existing filter is a case-insensitive substring test, and this keeps that.
- Searching group *color*, pinned state, or any group attribute other than its name.
- Changing a group's persisted collapsed state as a side effect of searching.
- Fixing the adjacent Panes-mode custom-tab-name gap covered by #9666.

## User experience

### Setup for all scenarios

A workspace with a tab group named `backend` containing three tabs titled `api server`, `worker`, and `db shell`, plus an ungrouped tab titled `backend-notes`.

### Current behavior (broken)

1. User opens the vertical tabs panel and types `backend` in the search field.
2. Only `backend-notes` remains — it matched on its own title.
3. The `backend` group and its three members are filtered out entirely.

### Expected behavior

1. User opens the vertical tabs panel and types `backend`.
2. The `backend` group appears, expanded, showing all three members (`api server`, `worker`, `db shell`) even though none of their titles contain "backend".
3. `backend-notes` also appears, having matched on its own title.
4. Clearing the query restores the full list, and any group that was collapsed before the search is collapsed again.

## Behavior invariants

1. A query that is a case-insensitive substring of a group's displayed name matches that group.
2. When a group matches by name, every one of its member tabs appears in the results, regardless of the member's own text.
3. When a group matches by name in a pane-level display mode, each member tab shows **all** of its pane rows, not only panes matching the query.
4. A member tab that matches on its own text *and* belongs to a name-matched group appears exactly once, showing all its pane rows.
5. Tabs that do not belong to a name-matched group continue to filter on their own text exactly as before.
6. When no group name matches the query, results are identical to the previous behavior.
7. Results remain in tab order; a group's members render as one contiguous group container, never split into several.
8. A group with no member tabs contributes nothing to the results even if its name matches.
9. A group whose name was never set matches the placeholder shown in its header (`New Group`).
10. While a query is active, every group present in the results renders expanded, so no match is hidden behind a collapsed header.
11. Expanding a group during search does not alter its stored collapsed state; clearing the query restores the state the user last chose.
12. An empty query shows every tab and every group at its stored collapsed state.
13. A query matching neither any tab text nor any group name shows the existing "No tabs match your search." empty state.
14. Behavior is consistent across all three vertical tabs display modes (Summary, FocusedSession, Panes), since group headers render in each.
15. The next/previous-tab keybindings, while a search is active, cycle through exactly the tabs the panel shows — including members admitted by a group-name match. What is visible and what is reachable by keyboard never disagree.

## Success criteria

1. Typing a group's name shows that group with all its tabs.
2. Typing a group's name when that group is collapsed shows the group expanded with all its tabs.
3. Clearing the query returns the collapsed group to collapsed.
4. Typing text that matches only a tab title still filters to that tab, with no group behavior triggered.
5. Typing text matching nothing shows the empty state.
6. A tab in a matched group that also matches by title is not duplicated in the list.
7. With a group-name query active, pressing the next/previous-tab keybindings visits the matched group's members rather than skipping them.

## Validation

- **Unit tests:** cover name matching including the untitled placeholder, and the merge of group matches into text matches (member inclusion, ordering, pane-row upgrade, empty group, non-member exclusion, no-match passthrough).
- **Manual test:** create a group named `backend` with tabs whose titles do not contain "backend", collapse it, type `backend` in the panel search, and confirm the group appears expanded with all members; clear the query and confirm it re-collapses.
- **Regression test:** confirm searching for a plain tab title still behaves as before, and that the empty state still appears for a nonsense query.
- **Navigation test:** with a group-name query active, cycle tabs with the next/previous-tab keybindings and confirm the matched group's members are visited.

## Open questions

1. Should a matched group's *header* be visually distinguished from a group that is present only because a member matched? This spec treats them identically.
2. Should this fold into #9155 ("Search sessions by renamed tab name") as one "search matches every name you can see" effort? The two are independent but adjacent.
