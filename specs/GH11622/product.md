# Product Spec: Double-click the hidden-section bar to fully expand

**Issue:** [warpdotdev/warp#11622](https://github.com/warpdotdev/warp/issues/11622)
**Figma:** none

## Summary

The Code Review panel's diff editor collapses long stretches of unchanged context into hidden sections. Each hidden section renders as a full-width horizontal **bar** in the content area, with one or two **gutter expand buttons** (`▲`, `▼`, or `▲▼`) beside it. Today, clicking a gutter button reveals up to 25 lines at a time, so a 200-line hidden region takes 8 clicks to fully reveal. This spec adds **double-clicking the bar** as a shortcut that fully expands the entire hidden section in one action. The gutter buttons keep their existing chunk-at-a-time behavior unchanged.

## Problem

When a diff has hunks separated by hundreds of unchanged lines — or hidden context regions at the top or bottom of a file — there is no fast way to reveal the entire region. Users have to click a gutter button repeatedly (4–10 times typical, more for very large files). There is no keyboard shortcut and no "expand all" affordance.

### Why the bar, not the gutter button

An earlier iteration put double-click on the gutter button itself. Because the same target serves both "reveal one chunk" (single click) and "reveal everything" (double click), single-click has to be deferred by the OS double-click interval (~300–500ms) to disambiguate the two — otherwise the chunk flashes before the full expansion snaps in on a real double-click. Deferring single-click to wait out a possible double-click is a normal, widely-used UI pattern, so it isn't unusable — but it's avoidable here, and a small gutter arrow that shifts position as earlier chunks reveal is a cramped double-click target.

Giving each gesture its own target removes the conflict entirely. This is the established pattern in VS Code / Monaco's `hideUnchangedRegions`: the gutter arrows reveal a fixed chunk instantly, and double-clicking the full-width hidden bar expands the whole region. Separate targets mean single-click stays instant and no debounce is needed.

## Goals

- Double-clicking the hidden-section bar fully expands that hidden section in one action.
- Single-clicking a gutter expand button still reveals up to one chunk at a time, instantly (no added delay).
- Single-clicking the bar does nothing — and in particular does not start a text selection (today a click on the bar clamps to a text offset and can begin selecting).
- The double-click affordance is discoverable: the bar shows a pointer cursor and a hover state, and a tooltip explains the gesture.
- Adjacent gutter interactions (diff-hunk sliver toggle, add-as-context, revert) are unaffected.

## Non-goals

- A keyboard shortcut for "expand all hidden sections in the current file" — separate feature; could land later.
- A "collapse all" / "expand all" affordance at the file or panel level — covered by issue #9129, a different feature.
- Changes to how hidden sections are *initially* computed (context-line counts, hunk grouping) — out of scope.
- Changes to the `Hoverable::on_double_click` callback API used by tabs and the workflow alias bar — those widgets remain on their existing patterns.

## User experience

### Current behavior (chunked-only)

1. User opens the Code Review panel on a file with a large hidden region between two hunks.
2. The hidden region renders as a full-width bar with one (or two) gutter expand buttons (`▲`, `▼`, or `▲▼`) beside it.
3. User clicks a gutter expand button.
4. The next 25 lines reveal immediately.
5. To see the full region, user clicks the button (or its sibling) 3 to 10 more times.
6. Clicking the bar itself does nothing useful — it clamps to the nearest text offset and can begin a text selection.

### New behavior (chunked + double-click the bar)

1. User opens the Code Review panel on a file with a large hidden region between two hunks.
2. The hidden region renders as a full-width bar with one (or two) gutter expand buttons beside it. Hovering the bar shows a pointer cursor, a hover highlight, and a tooltip describing the double-click gesture.
3. User double-clicks the bar.
4. The entire hidden region unhides in one transition. No intermediate chunked reveal is visible.
5. Alternatively, user single-clicks a gutter expand button. The next 25 lines reveal immediately, with no added delay. The user can continue clicking to chunk further.
6. A single click on the bar does nothing, and in particular does not start a text selection.

## Behavior (testable invariants)

1. Double-clicking a hidden-section bar fully expands the entire hidden section that bar represents, in a single transition with no intermediate chunked reveal.

2. Single-clicking a gutter expand button reveals one chunk (up to 25 lines) of the hidden section, immediately — there is no added delay relative to the pre-feature behavior.

3. Single-clicking the bar is a no-op: it does not expand the section, does not move the text cursor, and does not begin a text selection.

4. The bar is discoverable as a double-click target — on hover it shows a pointer cursor and a hover/highlight state, and a tooltip explains that double-clicking expands the section.

5. Triple or higher click counts on the bar produce one full expansion total (the same as a double-click), not multiple. Once the section is fully expanded the bar no longer exists, so any further clicks have no target.

6. Double-clicking a gutter expand button has no special "expand all" behavior — the two clicks are treated independently and reveal up to two chunks. Full expansion is reached only by double-clicking the bar.

7. Single-clicking a gutter button and double-clicking the bar are independent gestures on independent targets — neither defers, debounces, nor cancels the other.

8. Double-clicking the bar of a *small* hidden section (one that already only requires a single chunked click to fully expand) fully reveals it in one expansion. No regression vs. clicking the lone gutter button.

9. Adjacent gutter interactions are unaffected:
   - Single-click on a diff-hunk's colored sliver still toggles diff navigation.
   - Single-click on the add-as-context (`+`) button still adds the hunk as context.
   - Single-click on the revert button still reverts the hunk.
   - Clicks on the gutter outside any actionable element remain no-ops.

10. The feature works identically across operating systems — macOS, Linux, and Windows users all get double-click full expansion on the bar and instant single-click chunked reveal on the gutter buttons. Double-click *detection* uses the platform's standard double-click interval via the existing framework click handling; this feature introduces no separate per-gesture debounce.

## Open questions

- None at spec time. All edge cases above have a defined behavior.
