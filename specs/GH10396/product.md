# Code review find navigation scrolls selected branch-diff matches into view — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/10396
Figma: none provided

## Summary
The code review panel's Find navigation should scroll the diff viewport to the selected match every time the user advances to the next or previous result. This must work consistently when the diff selector is set to "Uncommitted changes", the repository's main branch such as `master`, or any other available branch.

The reported bug is that advancing Find within a branch comparison can update the selected match without bringing that match into view, especially when the target match is in an editor pane that has not been fully rendered yet. The fixed behavior should require only one navigation action per match; users should not need to press Enter twice or manually scroll to force off-screen diff editors to render.

## Problem
Find is a primary way to inspect large code review diffs. When the panel is comparing against a branch, a user can search for a term with multiple matches, press Enter or click the next button, and remain visually stuck at the current viewport even though Find has advanced internally. This makes it hard to trust search results, interrupts review flow, and is particularly confusing because the same query works when switching back to "Uncommitted changes".

The observed workaround is to switch modes, which is not acceptable for branch-diff review because it changes the set of changes being reviewed.

## Goals
- Advancing or reversing Find in the code review panel scrolls the selected match into view in branch comparisons and uncommitted-change comparisons.
- The first navigation action to an off-screen match eventually settles on the precise match location without requiring a second navigation action.
- Behavior remains correct for long diffsets, matches near the end of the diff, matches in different files, and matches in files whose editor layout is produced lazily.
- Existing Find semantics are preserved: result ordering, wraparound, search options, match highlighting, and horizontal scrolling still behave as they do today.
- The fix is resilient to scroll preservation and lazy rendering; neither should override or drop a user-initiated Find navigation.

## Non-goals
- Changing which files are included in code review Find. Collapsed files, binary files, unrenderable large diffs, and files without editor state should keep their current searchability rules.
- Changing the Find UI, keyboard shortcuts, result count display, case-sensitive option, regex option, or unsupported find-in-block behavior.
- Changing diff generation, branch selection, merge-base calculation, or which files appear when diffing against a branch.
- Adding a new visible loading indicator for Find navigation.
- Reworking code review list virtualization or editor lazy layout beyond what is needed to make selected Find matches scroll reliably.

## Figma / design references
No Figma mock was provided. A screen recording exists in the originating Slack thread, but it is not included in this repository artifact.

## User experience
1. A user opens the code review panel, selects a branch comparison such as `master`, and searches for a term with multiple results across the diff. Pressing Enter or clicking the Find next button selects the next result and scrolls the main diff viewport until that selected result is visible.

2. The behavior is the same when the selected diff mode is "Uncommitted changes", the detected main branch, or another branch from the diff selector. Switching between modes must not be required for Find scrolling to work.

3. If the selected match is in a file editor that is already laid out, the viewport scrolls directly to the match. If the selected match is in an off-screen or lazily laid-out editor, the panel may briefly scroll to that file to trigger layout, but it must then complete the precise scroll to the selected match without another user action.

4. The selected match should be visible with the same approximate vertical buffer currently used by code review Find. If the match is already fully visible, navigation may leave the vertical scroll position unchanged while still updating the selected highlight.

5. Horizontal scroll follows the selected match after vertical positioning succeeds. Long lines should be scrolled horizontally using the same editor behavior as existing code review Find.

6. Find next and previous keep wrapping at the beginning and end of the result list. Wrapping to a match in a far-away file still scrolls the selected match into view.

7. Find highlighting remains in sync with navigation. The newly selected match is visually distinguished from other matches after the viewport settles.

8. Scroll preservation must not undo a Find navigation. After the user advances to a match, any subsequent lazy height adjustment or editor layout update should preserve the target match's visibility rather than restoring the pre-navigation viewport.

9. If the user changes the query, toggles case sensitivity, toggles regex mode, changes diff mode, collapses a file, or closes the Find bar while a lazy scroll is pending, stale pending scroll work must be cancelled or ignored. The viewport must not jump later to a match from an old search state.

10. If a selected result belongs to a file that is no longer searchable or no longer has editor state, the panel should avoid crashing and should keep the existing no-op behavior for unavailable targets.

11. No new user-visible error should be shown for normal lazy-layout delays. Logging or telemetry for unexpected failures is acceptable, but the normal expected path should feel like ordinary Find navigation.

## Success criteria
- In a branch comparison with a long diffset, searching for a term whose next match is below the current viewport and pressing Enter once scrolls to that match.
- A first navigation to a match near the end of the final file in a long branch diff succeeds without pressing Enter a second time.
- Navigating backward to an off-screen match above the current viewport also scrolls correctly.
- The same search and navigation sequence continues to work in "Uncommitted changes" mode.
- Search result counts and selected result ordering do not change for expanded text diffs.
- The selected highlight is visible after scrolling, and horizontal position updates for matches on long lines.
- Changing diff mode or query while a lazy scroll is pending does not cause a later stale jump.
- Branch-diff review remains stable: no panic, no blank diff, and no persistent jump loop if a target editor cannot provide character bounds.

## Validation
- Add automated coverage for code review Find navigation to an off-screen match whose editor layout is initially unavailable. The test should assert that the pending scroll is retained or retried until bounds are available, then cleared only after the precise scroll is applied.
- Add or update a regression test for the reported branch-comparison flow: multiple matches across multiple files, target match initially outside the viewport, one next action, viewport ends on the selected match.
- Add coverage for stale pending scroll cancellation when the selected match/query/diff mode changes before layout completes.
- Manually verify on a dev build:
  - Open a code review panel with a long branch diff against `master`.
  - Search for a term with matches in several files.
  - Press Enter repeatedly and confirm each selected match scrolls into view on the first navigation action.
  - Repeat with previous navigation, with "Uncommitted changes", and after switching back to the branch comparison.
  - Confirm no unrelated scroll jump occurs after closing the Find bar or changing the query.

## Open questions
- None for product behavior. The issue-provided suspected cause is technical; the user-facing expectation is that Find navigation scrolls reliably across all code review diff modes.
