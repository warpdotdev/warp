# Code review diff selector redesign (APP-4199)
## Summary
Redesign the "diff against" control in the code review header so it reads as a button that opens a searchable picker and clearly shows which target is currently selected. Populated items and ordering are unchanged from today's selector — only presentation and interaction change.
## Figma
Figma: none provided. A single annotated screenshot is the only visual reference.
## Goals
- Make the control look and behave like a button (not a native dropdown) that opens a custom menu below it.
- Let the user search / filter the list of diff targets.
- Make the currently selected target obvious at a glance.
## Non-goals
- Changing which items appear in the list or their order.
- Changing what "diffing against X" means, how diffs are computed, or what shows in the diff body.
- Adding a reverse-diff or swap-base/target action. The double-arrow icon is decorative.
- Adding multi-select or pinning of targets.
- Showing per-target diff stats (file count, additions, deletions) inline in the picker. Stats continue to live in the header only, against the currently selected target.
## Behavior
### Trigger (the header button)
1. The diff selector in the code review header renders as a single clickable button containing, from left to right: a decorative double-arrow / swap icon, the name of the currently selected diff target, and no other affordances (no caret, no stats). The button has a single hit target covering the icon and label.
2. The double-arrow / swap icon is purely decorative — it signals that this control picks what the current branch is being diffed against. Clicking anywhere on the button, including the icon, opens the picker. There is no separate action on the icon.
3. The label shows the selected target's display name using the same naming used in the menu (e.g. `peter/stop-icon-fix`, `Master`, `Uncommitted changes`). Long names truncate with an ellipsis inside the button and do not wrap.
4. The button has idle, hover, pressed, focused, and open states consistent with other buttons in the code review header. When the picker is open, the button renders in a visibly "open" state until the picker closes.
5. The button is keyboard-focusable in the normal header tab order. `Enter` on the focused button opens the picker. A visible focus ring appears when focused via keyboard. Space is intentionally not bound so that once the picker is open, the user can type spaces into the search input without closing it.
6. The button is disabled (non-interactive, muted styling) only in states where the current selector is also disabled today (e.g. no repository detected, diff state still initializing). Enablement rules are unchanged from today.
### Picker (the menu)
7. Activating the button opens a picker anchored directly below the button, left-aligned to the button's left edge. The picker floats above surrounding content and does not reflow the header or the diff body.
8. The picker has two regions, top to bottom: a search input with placeholder text `Search diff sets or branches to compare…`, and a scrollable list of diff target rows.
9. The list of targets, their grouping, and their ordering match the existing selector exactly. This spec does not change what appears in the list. At minimum, the list continues to include:
   - An `Uncommitted changes` entry representing the working-tree diff against `HEAD`.
   - The repository's main / trunk branch entry (e.g. `Master`).
   - The set of other branches shown in the current selector, in the current order.
10. Each row shows, left to right:
    - A checkmark slot. The row for the currently selected target shows a check; all other rows show empty space of the same width so labels stay vertically aligned.
    - The target's display name.
    - No stats, status, or other trailing affordance. Rows are label-only.
11. The search input has focus when the picker opens. Typing filters rows by case-insensitive fuzzy match against the display name only; matched characters in each row are visually emphasized (e.g. bolded).
12. While the query is non-empty, only matching rows are shown, preserving the original order of the unfiltered list. Always-on entries such as `Uncommitted changes` are subject to the same filter (they are only shown if they match).
13. If no rows match the query, the list area shows a single non-interactive empty state (`No matches`). The search input remains focused and editable.
14. Clearing the search query restores the full list exactly as it was when the picker opened, including scroll position reset to the top.
15. The list is scrollable when it exceeds the picker's max height. Scrolling the list does not close the picker. The search input remains pinned at the top while the list scrolls.
### Selecting a target
16. Clicking a row, or pressing `Enter` with a row focused, selects that target: the picker closes, the header button's label updates to the new target, and the code review view begins diffing the current branch against that target. Selection behavior downstream of this choice (loading, error handling, stats in the header, file list refresh) is unchanged from today.
17. Selecting the row that is already selected is a no-op beyond closing the picker — no redundant reload is triggered and no telemetry is emitted for the "change".
18. While a new selection is loading, the header button shows the newly selected label immediately (optimistic); the diff body follows its existing loading behavior.
### Dismissal
19. The picker closes when any of the following happens: the user selects a row, presses `Escape`, clicks the header button again, clicks outside the picker, or the code review view loses focus (e.g. pane closes, tab switch).
20. Closing the picker without making a selection leaves the current selection unchanged and returns focus to the header button.
### Keyboard navigation
21. With the picker open and focus in the search input, `ArrowDown` moves focus to the first visible row; `ArrowUp` from the first row returns focus to the search input. Within the list, `ArrowUp` / `ArrowDown` move between visible rows, wrapping is not required. `Home` / `End` jump to the first / last visible row.
22. `Enter` on a focused row selects it (see 16). `Escape` closes the picker from any focus position inside it.
23. `Tab` and `Shift+Tab` move focus within the picker (search input ↔ list) and do not close the picker. Moving focus out of the picker entirely closes it.
24. Exactly one row shows a keyboard-focus indicator at a time. Mouse hover shows a distinct hover state that does not move keyboard focus.
### Invariants
25. At most one row in the list ever shows a checkmark, and it always matches the target reflected by the header button's label.
26. The set of rows shown, and their order when the search is empty, is identical to the set and order produced by today's selector for the same repository state. Any regression here is a bug.
27. The picker never modifies the repository or the diff selection as a side effect of opening, searching, scrolling, or closing — only an explicit row selection (16) changes what is being diffed.
28. The double-arrow / swap icon never triggers a swap, reverse-diff, or any other action. If we later want such an action, it will be specified separately.
