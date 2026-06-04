# Async-find query-refinement fast path — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/12037

## Summary
When a user narrows an active terminal find query by continuing to type the same literal prefix, Warp should refine the existing async-find result set without rescanning terminal blocks that could not possibly match the narrower query. The visible find experience remains the same as async find today, but refinement keystrokes on large sessions complete with substantially less CPU work and less scanning latency.

## Problem
Async find currently detects query refinement but still restarts a full block-list scan. This means typing `foo`, then `foob`, then `foobar` repeatedly rechecks blocks that had no `foo` matches and therefore cannot contain `foobar`. Large scrollback sessions pay nearly the same CPU cost for every refinement keystroke even though the result set can only shrink.

This is especially important before async find is promoted beyond Experimental because users expect each additional character in the find bar to feel faster, not to retrigger a full-session CPU spike.

## Goals
- Reduce CPU work and time-to-complete for literal query refinements in async terminal find.
- Preserve final match parity, match ordering, focus behavior, highlighting, and scanning affordances from the current async-find behavior.
- Skip work for blocks and rich-content views that had no matches for the previous broader query.
- Complete immediately when the previous broader query had zero matches.
- Keep non-refinement searches, regex searches, option changes, scoped find, and live-output invalidation behavior unchanged.

## Non-goals
- Changing the find bar UI, labels, keyboard shortcuts, focus rules, or highlight styling.
- Optimizing the legacy synchronous block-list find path while the async path is available.
- Treating regex edits as refinements.
- Broadening refinement detection beyond strict query-prefix edits with unchanged case-sensitivity and regex disabled.
- Introducing persistent search indexes or caching results across unrelated find sessions.
- Shipping row-level candidate scanning as a required part of the first iteration if a block-level fast path already removes the full-buffer rescan bottleneck. Row-level scanning may be added later if profiling shows matched blocks remain expensive.

## Behavior
1. When async find is active and the user edits the query from a non-empty literal query to a strict extension of that same query, Warp treats the new query as a refinement only if regex mode remains disabled and case-sensitivity remains unchanged.

2. For a refinement, every final match shown for the new query is a valid match for that exact new query. The user must not see stale matches from the broader query after the refinement run completes.

3. A refinement never produces more final matches than the previous broader query. The match count may drop as narrower results are processed.

4. Blocks that had no matches for the previous query are not searched again for the refined query, because they cannot contain matches for the stricter query.

5. Rich-content blocks, such as agent output blocks, follow the same inclusion rule: rich-content views that produced no matches for the previous query are skipped during refinement.

6. If the previous completed query had zero matches across terminal and rich-content content, typing a stricter refinement completes immediately with zero matches. The user should not see an extended `Scanning...` state for a search that has no possible candidates.

7. If the previous query had matches in only a small subset of blocks, the refined query scans only those candidate blocks/views. Blocks outside that subset keep no highlights for the refined query.

8. While refinement work is running, the find bar uses the existing async scanning affordance and incremental result updates. The behavior is allowed to complete so quickly that the scanning label is not perceptible.

9. During refinement, highlights and counts are coherent for the active query. Warp should avoid flashing a temporary all-zero state caused solely by clearing the old result set before candidate work is enqueued.

10. Focus behavior remains compatible with current async find:
    - If the previously focused match still exists for the refined query, focus should remain on that same visible occurrence when practical.
    - If the focused match no longer exists, focus clamps to a valid match if any remain.
    - If no matches remain, focus clears.

11. Final match ordering for refined results matches the ordering that a fresh async full scan of the same query, terminal state, scoped block set, and block sort direction would have produced.

12. Find-in-selected-blocks remains scoped. If a refinement is running inside a selected block set, candidate blocks are restricted to the intersection of the selected blocks and the previous result-bearing blocks.

13. Toggling regex mode, toggling case sensitivity, deleting characters, replacing the query with an unrelated string, or clearing the query is not a refinement. Those actions keep the current behavior: cancel the old async run and run the normal search/clear path.

14. Invalid regex behavior is unchanged. Regex searches and invalid regex edits do not use the refinement fast path.

15. Live output, block completion, scrollback truncation, and existing dirty-range invalidations continue to behave as they do today after a refinement. New output that arrives after the refined query becomes active is matched against the refined query.

16. If the previous broader async scan is still in progress when the user types a refinement, Warp does not use the candidate fast path because the broader result set is incomplete. It falls back to the normal async search path for the refined query so final results remain complete.

17. Results from cancelled broader-query generations never update the active refined query. Rapid typing across refinements must not surface stale highlights, stale match counts, or stale focused matches.

18. The optimization is transparent. Users should experience faster refinement, lower CPU usage, and the same final search answer; they should not receive a new prompt, warning, setting, or toast.

## Success criteria
1. In a large session where only a few blocks match `foo`, refining to `foobar` scans only the blocks/views that matched `foo` rather than all visible terminal blocks.
2. Refining from a completed query with zero matches completes immediately with zero matches and no background full-buffer scan.
3. For representative queries, the final refined match set, match count, ordering, and focused-match behavior match a fresh async full scan for the refined query.
4. Non-refinement edits still take the existing full-search path.
5. Regex and case-sensitivity changes remain behaviorally unchanged.
6. No stale highlights or stale generation messages are visible after rapid typing across multiple refinements.
7. CPU time and queue size during repeated prefix typing are materially lower than today on sessions with many blocks and sparse matches.

## Validation
- Add controller/unit tests that seed prior async results, refine the query, and assert that only previously result-bearing terminal blocks are enqueued.
- Add a zero-prior-results test that asserts refinement completes with zero matches and no terminal or rich-content scan work.
- Add parity tests comparing a refined-query fast path with a fresh full scan for the same terminal model.
- Add tests for non-refinement inputs: query deletion, unrelated query replacement, regex enabled, case-sensitivity changed, empty/whitespace query, and selected-block scope changes.
- Add focus tests for retained focused match, removed focused match, and all matches removed.
- Add rapid-typing/generation tests to ensure cancelled broader-query results cannot update the active refined query.
- Manually validate on a large scrollback session by searching a common prefix and then refining to a rarer string while observing responsive typing, stable highlights, and lower CPU usage.

## Open questions
- Should a later implementation add row-level candidate scanning inside matched blocks after the safer block-level fast path lands, or is the block-level win sufficient for promotion?
- Should future work try to preserve refinement speed while the broader query is still scanning, or is falling back to a full refined async search the right correctness tradeoff?
