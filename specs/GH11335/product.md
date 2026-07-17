# Product Spec: Find in collapsed reasoning traces

**Issue:** [warpdotdev/warp#11335](https://github.com/warpdotdev/warp/issues/11335)
**Figma:** none provided
**Reference recording:** [Issue Loom](https://www.loom.com/share/1d8e649310f64b6abdc867971d116204)

## Summary

Find should make matches inside collapsed agent reasoning traces discoverable without expanding traces while the user types. When the user explicitly navigates to one of those matches, Warp should expand the containing trace and reveal the focused match.

## Behavior

1. Find includes text inside reasoning traces in its result count and traversal order whether each trace is expanded or collapsed.

2. Entering or editing a Find query does not expand a collapsed reasoning trace. This remains true while results are still arriving and when a newly streamed match appears.

3. A collapsed reasoning trace that contains one or more matches for the current Find query shows an indicator in its header with that trace's exact match count. The indicator is absent when the trace has no matches.

4. The indicator is scoped to its containing trace. If several collapsed reasoning traces contain matches, each shows only its own count; matches in normal agent text, terminal output, or another reasoning trace do not affect it.

5. The indicator updates whenever the query or Find options change, including case sensitivity, regular-expression mode, and Find-within-block scope. Results from an older query must never reappear after a newer query is active.

6. While the current query is being evaluated asynchronously, Warp may show counts as current-query results arrive, but it does not retain a count from the previous query as if it applied to the new query.

7. Explicitly navigating to a match inside a collapsed reasoning trace—by pressing Enter or using the next/previous match controls or shortcuts—expands that trace and makes the focused match visible.

8. After navigation expands a trace, Warp scrolls both the conversation and the trace's own scrollable content as needed so a focused textual match itself is in view, not merely the trace header. The focused match uses the focused-match highlight, while the trace's other matches keep the normal match highlight. If an existing Find result refers only to non-visible source text behind a rendered image or diagram, Warp reveals that containing visual section instead of promising a glyph highlight that cannot be displayed.

9. Forward and backward traversal behave consistently across terminal matches, normal agent text, and multiple reasoning traces, including the existing wraparound behavior. Crossing into a collapsed trace expands only the trace containing the newly focused match.

10. Navigating between multiple matches in one expanded reasoning trace keeps the trace expanded and brings each newly focused match into view. Navigating away does not automatically collapse a trace that Find expanded.

11. A user may manually collapse an auto-expanded trace. It stays collapsed until the user expands it or a later explicit Find navigation focuses a match inside it; typing alone never reopens it.

12. Clearing the query or closing Find immediately removes reasoning-trace match indicators and match highlights. It does not otherwise change the user's current expanded or collapsed trace states.

13. An empty query, an invalid regular expression, a query with no matches, or a query whose matches are excluded by Find-within-block scope shows no reasoning-trace match indicator.

14. The collapsed-state indicator is understandable without relying on color alone: its visible text includes the numeric count with singular or plural wording such as “1 match” or “3 matches.” Existing pointer behavior for manually expanding the reasoning header remains unchanged.
