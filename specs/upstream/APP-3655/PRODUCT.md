# APP-3655: Vertical Tabs — Search Functionality + Control Bar UI Polish

## 1. Summary

Implement search/filter functionality for the vertical tabs panel search bar, and update the search bar's visual appearance to match the design mocks. The search bar will filter the visible pane items and tab groups in real time as the user types.

## 2. Problem

The search bar in the vertical tabs panel was implemented as a non-functional placeholder (APP-3648). Users with many tabs and tab groups have no quick way to navigate to a specific pane. The control bar's current visual treatment (background, border, padding) also diverges from the design mocks.

## 3. Goals

- Make the search bar fully functional: it accepts text input and filters the list of pane items and tab groups in real time.
- Match the design mocks for the control bar: correct horizontal padding and a visually "uncontainerized" search input (no background, no border).
- Preserve tab/group ordering in filtered results.
- Omit tab groups that have no matching panes.

## 4. Non-goals

- Fuzzy matching or ranked/sorted results — simple substring matching is sufficient.
- Searching across window sessions or workspaces beyond the current panel.
- Keyboard-navigable filtered results (arrow-key selection of filtered items) — out of scope for this iteration.
- Persisting or restoring the search query across panel closes or app restarts.
- Highlighting matched text within pane item rows.

## 5. Figma / Design References

Figma: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7080-26025&m=dev

## 6. User Experience

### Search bar activation

- The search input is now fully focusable and accepts keyboard input.
- Clicking anywhere on the search bar focuses it and shows a cursor.
- The "Not yet implemented" hover tooltip is removed.

### Filtering behavior

- As the user types into the search bar, the pane list is filtered in real time — no submit/enter required.
- Filtering is **case-insensitive** substring matching.
- **Search domain**: all visible text within a pane item row is considered, including:
  - Primary title (e.g. working directory or pane title)
  - Secondary title / subtitle (e.g. git branch, terminal title)
  - Pane kind label (e.g. "Terminal", "Code", "Settings")
  - Any badge text visible in the row (e.g. "Unsaved", PR name, diff stats)
- **Ordering**: panes that pass the filter are shown in their original display order — no reranking.
- **Tab groups**: a tab group header is shown only if at least one of its panes matches the query. If no panes in a group match, the group header is omitted entirely.
- **Empty state**: if the query matches no panes across all groups, the list area shows an empty state message, e.g. "No tabs match your search."

### Clearing the search

- Pressing **Escape** while the search bar is focused clears the query and returns focus to the previously active pane (existing behavior for focus return is preserved).
- Deleting all characters from the input restores the full, unfiltered list.
- Clicking outside the search bar (e.g. clicking a pane item) does **not** clear the query — the filter remains active.

### Control bar visual changes

- **Horizontal padding**: The control bar's left and right padding matches the content area of the panel (12px on each side, consistent with `GROUP_HORIZONTAL_PADDING`).
- **Search input container**: The background fill and border of the search input container are removed. The search input appears as a plain, unstyled text field with the magnifying glass icon — visually integrated into the control bar rather than enclosed in a box.
- All other control bar elements (split new-tab button, icon sizes, vertical padding) are unchanged.

### Tab switching while filtered

- When a search query is active, the "next tab" and "previous tab" actions (keyboard shortcuts) cycle through only the **filtered** pane list — not the full natural order. The user steps through exactly the panes that are currently visible.
- Once the query is cleared, next/previous tab navigation resumes using the full natural order.

### Interaction with non-search actions

- Creating a new tab (plus button) while a search query is active does not clear the search.
- Clicking a pane item to navigate to it does not clear the search query.
- The filtered view reflects the current query at all times while the query is non-empty.

## 7. Success Criteria

1. Typing into the search bar visibly filters the pane list to only panes whose row text contains the query (case-insensitive) without pressing Enter.
2. Tab groups with no matching panes are hidden entirely, including their group header row.
3. Tab groups with at least one matching pane are shown, and only their matching panes appear beneath them.
4. Original display ordering of both groups and panes within groups is preserved in filtered results.
5. Clearing the query (by deleting all text) restores the full unfiltered list identically to how it appeared before searching.
6. Pressing Escape while focused on the search bar clears the query and returns focus to the active pane.
7. While a search query is active, next/previous tab keyboard shortcuts cycle through only the filtered (visible) panes, not the full natural order.
8. An empty state is displayed when the query matches no panes.
9. The control bar has 12px horizontal padding on each side, matching the panel content area.
10. The search input has no visible background or border — it is visually "uncontainerized."
11. Clicking on the search bar focuses it (cursor visible, text input accepted) with no tooltip shown.
12. The search domain includes all visible text in the pane row: primary title, secondary title, pane kind label, and badge text.
13. Collapsing and re-opening the panel preserves the active search query and filtered view.
14. Clicking a pane item while a query is active does not clear the query.

## 8. Validation

- **Manual**: Type partial strings that match: only the primary title, only a subtitle, only a kind label (e.g. "code"), only badge text. Confirm only matching panes appear.
- **Manual**: Confirm tab groups with no matches are completely hidden.
- **Manual**: Confirm ordering of surviving results matches the original order.
- **Manual**: Confirm empty state message appears when no panes match.
- **Manual**: Press Escape — query clears, full list restores, focus returns to active pane.
- **Visual**: Screenshot the control bar with the search input focused and unfocused; confirm no background or border on the search input, and 12px horizontal padding aligning with pane item rows.
- **Regression**: Confirm the plus/new-tab button still creates tabs and opens the dropdown while a query is active.
- **Manual**: With a filtered list active, use next/previous tab shortcuts and confirm navigation skips non-matching panes.
- **Manual**: Collapse and re-open the panel; confirm the query and filtered view are preserved.
- **Manual**: Click a pane item in the filtered list; confirm the query is not cleared.

## 9. Open Questions

None — all open questions resolved:
- Search query persists across panel collapse/re-open.
- Clicking a pane item to navigate does not clear the query.
- All visible text in the pane row is searchable; no fields are excluded.
