# Live updates for inline history search — Product Spec

GitHub issue: [warpdotdev/warp#11474](https://github.com/warpdotdev/warp/issues/11474)

Reference video: [reported stale-results behavior](https://github.com/user-attachments/assets/5cf21864-a41c-4240-a911-482c52a07900)

## Summary

The inline history overlay should keep its search results synchronized with text the user edits after the overlay opens. Temporary history-item previews must remain separate from the user's search text so dismissing the overlay preserves the latest user-authored input instead of reverting to an obsolete value or retaining a preview.

## Figma

Figma: none provided.

## Behavior

1. Opening inline history with Up Arrow searches using the input text and active history category at the moment the overlay opens. Matching history entries are shown; when there are no matches, the overlay shows its existing no-results state.

2. While the overlay remains open, every change to the input text made by the user updates the history query. This includes typing, deleting, replacing a selection, cutting or pasting, and undoing or redoing an edit. Moving the cursor or changing the selection without changing text does not change the query.

3. Results always converge on the latest user-authored text without requiring the user to dismiss and reopen the overlay. In particular:
   - Editing an unmatched query into a matching query replaces the no-results state with matching entries.
   - Editing a matching query into an unmatched query replaces the entries with the no-results state.
   - Deleting all query text returns the overlay to its existing unfiltered-history behavior.

4. When a result is highlighted, Warp may continue to preview that command or prompt in the input as it does today. The preview is temporary: it does not become the history query, does not trigger another query, and does not cause the result set to narrow to the previewed item's full text.

5. Moving the highlight through commands, prompts, or conversations does not change the user-authored query. Each highlighted item may update the temporary preview, but the overlay continues to show results for the same query until the user edits the input or changes the active history category.

6. If the user edits the input while a highlighted item is being previewed, the resulting text is a new user-authored query. Results update from that text, and this edit becomes the value Warp preserves if the overlay is dismissed without accepting a result.

7. Switching between the All, Commands, and Prompts categories reruns the selected category with the latest user-authored query, not with the currently previewed item's text. Switching categories does not discard that query.

8. If the user dismisses inline history without editing after it opens, Warp closes the overlay and restores the input text and cursor position that existed when the overlay opened. Temporary previews are discarded.

9. If the user edits while inline history is open and then dismisses it, Warp closes the overlay and preserves the latest user-authored text and the cursor position produced by that edit. It does not restore the older opening text and does not leave a temporary preview in the input. This applies to non-accepting dismissal paths, including Escape and moving Down past the final result.

10. Accepting a highlighted history item keeps the existing acceptance behavior: the accepted command or prompt is used, or the selected conversation is opened. Clicking a prompt row in the Cloud Mode V2 history UI remains an accepting action and keeps that selected prompt in the input. Acceptance is not replaced by the dismissal behavior in invariants 8–9.

11. Rapid or overlapping edits are ordered by recency. Results produced for an older query must never replace results for a newer query, even if the older search completes later. Once searching settles, both the visible results and no-results state correspond to the most recent user-authored text and active category.

12. The input keeps keyboard focus while results refresh, and the overlay remains open across user edits, empty states, and result transitions. Refreshing must not consume the user's edit or require refocusing the input before the next action; the existing highlighted-item preview behavior remains governed by invariants 4–6.

13. The same edit, preview, dismissal, and latest-query rules apply everywhere the inline Up Arrow history UI is used, including the regular terminal/agent history overlay and the Cloud Mode V2 prompt-history overlay.

14. Existing history matching, ordering, category filters, keyboard navigation, mouse behavior, visual styling, and accessibility semantics remain unchanged except where this spec explicitly changes live query updates and dismissal restoration.
