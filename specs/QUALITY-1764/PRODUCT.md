# Child-run deep links in the web session viewer

## Summary
The web session viewer uses the top-level orchestrator as the stable route and encodes the selected child as `#child=<run-id>`. Existing direct child links canonicalize to that root when the viewer can resolve the complete ancestor chain.

## Problem
Today a child-pill click can replace the root URL with the child’s `/conversation/<id>` or `/session/<id>` URL. Refreshing or copying that URL then opens the child without its orchestration context.

## Figma
Figma: none provided. This work does not redesign the existing pill bar.

## Delivery phases

### Phase 0 — Preserve the entry URL
Phase 0 ships in this PR:

1. Non-forced pane focus and pane-link updates keep the current `/conversation/<id>` or `/session/<id>` URL.
2. Child-pill navigation no longer replaces the orchestrator URL with the child URL.
3. Refresh and copy reopen the orchestrator without preserving the selected child.

Phase 0 is temporary. Its blanket URL guard suppresses the fragment write required by the final design. Phase 2 must remove or invert that guard before adding anchor navigation, or pill clicks will silently fail to add `#child=<run-id>`.

### Phase 1 — Canonicalize direct child links
The signed-in viewer resolves a direct child’s top-level root by walking `parent_run_id` through the existing run endpoint. It replaces the child URL with the root URL plus the child anchor. `?view=standalone` skips this canonicalization.

### Phase 2 — Restore anchored selections
The root viewer restores `#child=<run-id>`, writes anchors for pill navigation, and applies browser Back and Forward.

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
- New orchestration, messaging, execution, or pane-management controls.