# GH10406: Code review find bar lifecycle reset
GitHub issue: [#10406](https://github.com/warpdotdev/warp/issues/10406)
## Summary
The code review panel find bar should be scoped to the currently open panel session and selected repository. When the user closes the panel or switches repositories, any active find UI, query, match selection, and highlights are dismissed so reopening the panel or viewing another repository starts from a clean find state.
## Problem
The find bar can remain visible after the code review panel is closed and reopened, or after the selected repository changes. It still shows the previous query, but next/previous navigation no longer works because its stored matches point at editors from the previous view state.
## Goals
1. Closing the code review panel always dismisses code-review find state.
2. Switching repositories from the code review panel always dismisses code-review find state for the repository being left, and the selected repository appears with find closed.
3. Reopening find after either transition behaves like a fresh find session over the currently visible expanded diff editors.
4. Users never see stale match counts, selected-match state, or highlights after the underlying editor set changes due to panel close/reopen or repository switch.
## Non-goals
1. This does not change terminal find, editor find, notebook find, or other surfaces that reuse the shared find component.
2. This does not add persistence for code review find queries across panel sessions or repositories.
3. This does not redesign the find bar UI, keyboard shortcuts, match-count display, or case-sensitive/regex toggles.
4. This does not change which code review files are searchable; find continues to search visible/expanded code review editors only.
## Figma
Figma: none provided. This is a lifecycle/state bug fix for an existing UI.
## Behavior
1. When a user opens the code review panel and invokes find, the find bar opens with the existing code review find behavior: focus moves into the find input, typing a query searches the currently visible expanded diff editors, matches highlight in the editors, and next/previous navigation moves through the current results.
2. When the user closes the code review panel while the find bar is open, the find bar is dismissed immediately as part of the close action.
3. Closing the panel clears the find query text. Reopening the panel must not show the previous query in the find input.
4. Closing the panel clears the current match list, selected match, and match count. Reopening the panel must not show a stale `n/m` match count or selected match from the previous panel session.
5. Closing the panel removes find highlights and focused-match styling from all code review editors that were part of the closing panel session.
6. Reopening the code review panel after a close starts with the find bar closed, even if the same repository and same diff set are still selected.
7. After reopening the panel, invoking find again starts a fresh search against the currently rendered code review editors. Next/previous navigation must work when the new search has results.
8. If the panel is closed while a search is in flight, stale asynchronous search results from the closing session must not reopen the find bar, restore match counts, or reapply highlights after the panel has closed.
9. If the panel is closed while the find bar is already closed, the close action is a no-op from the user's perspective: no query appears on next open, no highlights remain, and no extra UI is shown.
10. When the user switches repositories using the code review panel's repository switcher while find is open, the find bar is dismissed as part of leaving the old repository.
11. Repository switching clears the old repository's find query text, match list, selected match, match count, and highlights before or at the same time that the old repository view is hidden.
12. After a repository switch completes, the newly selected repository's code review panel is shown with the find bar closed.
13. The newly selected repository must not inherit the old repository's query text, match count, selected match, or highlights.
14. After switching repositories, invoking find again starts a fresh search against the newly selected repository's visible expanded diff editors. Next/previous navigation must move through matches in the newly selected repository.
15. Switching away from a repository and later switching back to it still shows find closed for that repository. Previously typed query text from before the switch must not be restored.
16. If a repository switch occurs while a search is in flight for the old repository, stale results from the old repository must not affect the new repository's find bar, match count, selected match, or editor highlights.
17. The behavior is the same whether the repository switch is triggered by the repo switcher dropdown, by the focused terminal/repository changing, or by the available repository list changing such that the selected repository changes.
18. The behavior is the same whether the code review panel is maximized or not maximized.
19. Find's close affordances still work normally. Pressing Escape or clicking the close button while the find bar is open dismisses find, clears matches, and removes highlights for the current panel session.
20. Reopening find within the same still-open panel after manually closing it starts from an empty query and a fresh match state.
21. Case-sensitive and regex toggle state may keep the current shared find-bar behavior, but stale query text, results, selected match, and highlights must not persist across panel close or repository switch.
22. Code review content edits, file expansion/collapse, diff mode changes, branch metadata refreshes, and ordinary diff reloads keep their existing find behavior unless they also close the panel or switch repositories. This spec only requires lifecycle resets for panel close and repository switch.
23. If the user closes the panel with unsaved code review edits and then cancels the close in an unsaved-changes confirmation flow, the panel remains open and active find state may remain unchanged. If the close actually proceeds, find is dismissed and cleared.
24. If there are no matches after reopening find, the find bar shows the normal no-results state for the new empty or typed query; it must not reuse the old no-results or match-count state.
25. The reset must be silent: no toast, banner, error state, or warning is shown when find state is dismissed due to panel close or repository switch.
## Success criteria
1. The issue reproduction path no longer leaves a visible broken find bar after close/reopen.
2. The repository-switch reproduction path no longer leaves a visible broken find bar after selecting another repository.
3. Next/previous match navigation works after the user reopens find and searches again in the current panel session.
4. No stale find highlights remain visible after close/reopen or repository switch.
## Validation
1. Manual validation should cover the close/reopen path: open code review, open find, type a query with matches, close the panel, reopen it, confirm find is closed and highlights are gone, then open find again and confirm navigation works.
2. Manual validation should cover the repository-switch path: open code review with multiple repositories available, open find in one repository, type a query with matches, switch repositories, confirm find is closed and no stale query/highlights remain, then open find and search in the newly selected repository.
3. Automated regression coverage should assert that panel close and repository switch reset the code review find model and visible query state.
## Open questions
1. No product open questions. The expected behavior is to discard, not preserve, code review find state across these lifecycle boundaries.
