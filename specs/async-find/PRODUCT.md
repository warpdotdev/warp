# Async Find

## Summary

Move terminal find/search off the main thread so opening the find bar, typing a query, or running a long-running command never freezes the UI — even on a session with thousands of blocks of scrollback. Results stream into the find bar and onto highlighted blocks incrementally, with a clear "scanning" affordance while the background scan is still in flight.

This is gated behind the `AsyncFind` feature flag (default-on for dogfood). When the flag is off, terminal find behavior is unchanged.

## Problem

Terminal find currently runs synchronously on the main thread: every keystroke in the find bar (and every rerun triggered by new output) walks the entire block list, every grid, and runs the regex DFA against every row before the UI can update again. On long sessions this produces visible UI hitches in the editor, the input box, and the block list while a query is in flight, and the find bar shows a stale match count until the scan completes.

## Goals

1. Find never blocks the UI — typing in the find bar, scrolling, and running commands stay smooth regardless of scrollback size.
2. Match count and highlights become visible as soon as any matches are known, rather than only after the whole session has been scanned.
3. The user can tell the difference between "scan complete, X matches" and "still scanning, X+ matches so far".
4. Behavior with the flag enabled stays consistent with sync find for everything that is not about *when* results arrive: same match set, same focus traversal, same alt-screen behavior, same find-in-block scoping, same regex/case toggles.

## Non-goals

1. Changing the find bar UI beyond adding a scanning indicator.
2. Changing find behavior on the alternate screen — alt screen find continues to run synchronously (its content is bounded by the visible viewport).
3. Indexing or caching results across find sessions.

## Behavior

1. When the `AsyncFind` feature flag is enabled, opening the find bar and entering a query starts a *background* scan of the block list. The UI remains fully responsive while the scan runs: typing, scrolling, and running new commands are unaffected by scan progress.

2. While the scan is in flight, the find bar replaces the usual `current/total` label with a scanning affordance:
   - When at least one match has been found so far: `<count>+ ...` (e.g. `12+ ...`).
   - When no matches have been found yet: `Scanning...`.
   The label is rendered in the same muted style as the normal index label.

3. When the scan completes, the indicator returns to the standard `current/total` form and reflects the final counts.

4. Match highlights paint into blocks incrementally as results stream in, in the same visual style as sync find. The user sees highlights appear in the most recently scanned blocks first (newest-first scan order), even before the rest of the session has been processed.

5. As soon as the first match arrives, that match is auto-focused (its index becomes 1). The user can immediately start pressing the next/previous shortcut even while scanning is still ongoing — focus traversal and scroll-to-match work over the partial result set.

6. Pressing next/previous wraps around the *currently known* match set. The wrap point may grow as more results arrive; this is expected and intentional, not a bug.

7. The order in which matches are visited by next/previous matches sync find for the same query, options, and block sort direction:
   - In `MostRecentLast` (pinned-to-bottom / waterfall) sessions, the first focused match is the newest match (closest to the prompt). Pressing the "up" direction moves toward older matches; "down" wraps toward newer.
   - In `MostRecentFirst` (pinned-to-top) sessions, the first focused match is again the newest match (now at the top), with the directions inverted accordingly.
   - Within a single block, the grid traversal order (output vs. prompt-and-command) and the in-grid order also match sync find.

8. Editing the query mid-scan cancels the in-flight scan and starts a new one against the new query. The match count resets, the scanning indicator reappears, and results stream in for the new query. There is no flash of stale highlights from the previous query.

9. **Query refinement.** Editing the query into a strict extension of the previous query (e.g. `foo` → `foob`) with regex disabled and the same case-sensitivity setting is treated as a refinement. The user does not see a flash of zero results: the refined query produces a new scan but stale matches are cleared coherently. *Open question: future versions may filter the existing result set in place rather than rescanning, but visible behavior is the same.*

10. Toggling case-sensitivity or regex restarts the scan from scratch with the new options.

11. **Live scrollback during a scan.** When a command is producing output while a find is active, new rows in the active block are scanned incrementally as a "dirty range" — only the changed rows are re-examined, not the whole block. The match count and highlights update as new rows are processed.

12. **Block completion.** When a running command finishes, the completed block is scanned with its final output (using the dirty range accumulated during execution, falling back to a full block scan when no dirty range is available). The user sees any new matches in that block appear shortly after the command exits.

13. **Scrollback truncation.** When the active block's grid has lines truncated out of scrollback while a find is active, matches that fall in truncated rows are dropped from the result set. The user-visible match count decrements accordingly; the focused index is clamped so it never points past the end of the new result set.

14. **Find in selected blocks.** When find is scoped to a subset of blocks (the existing find-in-block flow), only those blocks are enqueued for scanning. The scanning indicator and streaming results behave identically; the indicator clears as soon as the scoped scan finishes, which is typically near-instant.

15. **AI / rich-content blocks.** Rich-content blocks (e.g. agent output) are still scanned on the main thread, but they are interleaved into the same streaming pipeline as terminal blocks. From the user's perspective they appear in the result list in the same visual order they would with sync find. Match counts include AI matches; focus traversal walks through them in display order alongside terminal matches.

16. **Alt screen.** When the alt screen is active, find continues to run synchronously against the alt-screen viewport. The scanning indicator does not appear, because alt-screen find is bounded and effectively instant.

17. **Closing the find bar / clearing the query.** Closing the find bar or clearing the query cancels any in-flight background scan, clears all highlights from blocks, and clears any AI match annotations. The find bar's `current/total` label resets to `0/0`.

18. **Match parity with sync find.** For any given (query, case-sensitivity, regex, block sort direction, scoped block set, terminal state), the *final* set of matches surfaced by async find is the same as the set sync find would have produced. Match identity, ranges, ordering, and focus-traversal order all match. This is what allows the flag to be flipped on or off without behavioral surprises.
