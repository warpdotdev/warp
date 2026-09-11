# Child-run deep links in the web session viewer

## Summary
Phase 0 now keeps the web session viewer’s entry URL when a child opens. The remaining work ships as one deliverable: use the top-level orchestrator as the stable route, encode the selected child as `#child=<run-id>`, and make direct child links restore that anchored selection.

## Problem
Phase 0 preserves the root URL but loses the selected child on refresh or copy. Canonicalizing a child URL without also restoring its anchor would regress direct links: the viewer would open the root instead of the requested child.

## Figma
Figma: none provided. This work does not redesign the existing pill bar.

## Delivery

### Shipped Phase 0
[PR #15317](https://github.com/warpdotdev/warp/pull/15317) shipped the following behavior:

1. Non-forced pane focus and pane-link updates keep the current `/conversation/<id>` or `/session/<id>` URL.
2. Child-pill navigation no longer replaces the orchestrator URL with the child URL.
3. Refresh and copy reopen the orchestrator without preserving the selected child.

Its blanket URL guard is temporary. The remaining implementation must remove or invert that guard before any canonical redirect or anchor write. Otherwise the guard will silently discard `#child=<run-id>`.

### Remaining implementation — canonical child deep links
Canonicalization, anchor restoration, and selection history ship together. The viewer must be able to parse and restore the anchor before direct child links start redirecting to it. `?view=standalone` remains the escape hatch.

## Behavior

### Canonical URL and selection
1. The stable route is the top-level root’s `/conversation/<id>` or `/session/<id>` URL.
2. A selected child adds `#child=<child-run-id>`. Selecting the root removes the fragment.
3. Pill navigation changes only the fragment. It preserves the root path and supported query parameters.
4. A child without a durable run ID remains selectable but leaves the URL unanchored.
5. Automatic routing between the root’s live session and stored conversation preserves the child fragment.

### Opening anchored root links
6. A root URL with `#child=<run-id>` loads the root viewer, waits for initial orchestration hydration, then selects the matching child.
7. Refreshing or copying a valid anchored URL restores the same root and child for an authorized viewer.
8. The viewer does not use a timeout to classify an anchor as stale.
9. After initial hydration settles, a malformed, stale, inaccessible, or out-of-tree anchor selects the root and removes the fragment with history replacement.

### Opening direct child links
10. Existing direct child `/conversation` and `/session` links remain valid.
11. By default, a signed-in viewer walks from the child to the top-level root and replaces the URL with the root route plus the original child’s run ID.
12. The walk stops successfully only at a run with no parent. It never canonicalizes to an intermediate ancestor.
13. The walk fails on an unauthorized or missing ancestor, a malformed or repeated run ID, more than 64 parent edges, or a request failure. The child then stays at its original URL as a standalone viewer.
14. The root’s active, reachable session route is preferred. Its stored conversation route is the fallback. If neither route is available, the child stays standalone.
15. A non-orchestration run opens normally without a child fragment.
16. A logged-out public viewer cannot use the authenticated run endpoint. A public child link therefore remains standalone.

### Standalone escape hatch
17. The first implementation supports the exact, case-sensitive `?view=standalone` value on direct child URLs.
18. `view=standalone` skips only child-to-root canonicalization. Same-run `/session` and `/conversation` redirects preserve it.
19. Unknown `view` values use the default canonicalization behavior.
20. When a standalone child exposes descendants, selecting one keeps the standalone child as the base route and adds the descendant’s child fragment.

### Browser history
21. A cold child-to-root canonicalization replaces the current history entry.
22. A changed, user-initiated pill selection pushes one history entry. Repeated selection is a no-op.
23. Browser Back and Forward apply the prior root or child selection without writing another entry.
24. Focus changes, session events, transcript loading, initial anchor restoration, and other non-navigation updates do not write history.

## Decisions
- Ship canonicalization and anchor restoration as one deliverable. Neither behavior ships alone.
- Canonicalize to the top-level root, not the immediate parent.
- Use `#child=<run-id>` because the run ID survives session-to-conversation route changes.
- Ship `?view=standalone` in v1.
- Replace cold canonicalizations and push user pill selections.
- Keep an accessible child standalone when the complete root chain or root route cannot be loaded.

## Out of scope
- Pill-bar or transcript-viewer redesign.
- Native desktop URL behavior.
- Nearest-ancestor fallback.
- Anonymous access to run topology.
- A dedicated copy-link action that emits a canonical root link independently of the address bar. Standard address-bar copy continues to include the selected child anchor.
- New orchestration, messaging, execution, or pane-management controls.