# Child-page orchestration context in the web session viewer

## Summary
Each run remains a first-class web page. Opening a child run keeps the child’s `/conversation/<id>` or `/session/<id>` route and hydrates enough orchestration context to render the existing pill bar, parent breadcrumbs, and sibling navigation. Pill selections update the address bar to the selected run and create browser-history entries.

This is an alternate proposal to the root-canonicalization design in [PR #15317](https://github.com/warpdotdev/warp/pull/15317). The requester will choose one proposal before implementation.

## Problem
Today, selecting a child pill replaces the root orchestrator URL with the child’s URL by using `history.replaceState`. Refresh then opens the child without orchestration context. Browser Back cannot recover the overwritten root entry. A copied link also loses the surrounding run context.

## Figma
Figma: none provided. This proposal reuses the existing orchestration pill bar without a visual redesign.

## Goals
- Make every accessible direct child link a useful entry point into its orchestration context.
- Keep the address bar aligned with the run currently in view.
- Make browser Back and Forward traverse user-initiated pill selections.
- Preserve the current standalone child viewer when orchestration context cannot be loaded.

## Behavior

### Direct links and context loading
1. Opening `/conversation/<child-conversation-id>` or `/session/<child-session-id>` keeps that route in the address bar. The viewer does not redirect to the root orchestrator.
2. The requested child transcript or live session remains the primary content while orchestration context loads.
3. A signed-in viewer with access to the run tree sees the existing orchestration pill bar after the full ancestor chain and each ancestor level’s sibling cohort load.
4. The pill bar uses the requested child as the selected run.
5. A child without descendants anchors the bar on its parent. The bar therefore shows the child beside its siblings.
6. A child with descendants anchors the bar on itself. The bar therefore shows the child as the orchestrator for its direct children.
7. For a nested child, the existing breadcrumbs lead to the top-level root and any distinct intermediate parent required by the current drill-down level.
8. The viewer does not show a partial or misleading ancestor chain while context is loading. It shows the requested child with the normal viewer loading state until the initial context load either completes or fails.
9. The initial context load stops successfully when a run has no parent.
10. The initial context load fails closed when an ancestor request is unauthorized, missing, malformed, cyclic, over the depth limit, or fails for another reason.
11. On a failed context load, the requested child remains usable as a standalone viewer at its original URL. The viewer does not redirect to a partial ancestor.
12. A non-orchestration run opens as it does today and does not show an empty pill bar.
13. A run that has no attachable live session and no stored transcript remains visible in the pill bar for status context, but its pill is disabled. Selecting it does not change the content, URL, or browser history. The pill becomes navigable when content becomes available.

### Pill navigation and URL meaning
14. The address bar identifies the run currently in view through its session or conversation locator. It does not identify the run tree that the viewer entered through.
15. Selecting a navigable child, sibling, parent, or root pill changes the primary content to that run and updates the address bar to that run’s own shareable `/conversation/<id>` or `/session/<id>` route.
16. A stored conversation route is used when that run has a durable conversation link. A live session route is used when the run is available only through its shared session.
17. Pill navigation does not reload the page. It uses the already hydrated conversation and pane state.
18. Selecting the run that is already in view is a no-op. It does not add a browser-history entry.
19. Incidental focus changes, context hydration, status updates, and hidden-pane materialization do not add browser-history entries.
20. If a selected live run later gains a stored conversation route, the viewer may replace the current route with the same run’s conversation route. This same-run canonicalization does not add a history entry.

### Browser history, refresh, and sharing
21. Each changed, user-initiated pill selection adds one browser-history entry by using push semantics.
22. Browser Back returns to the previously viewed run. Browser Forward returns to the next viewed run.
23. Applying Back or Forward changes the selected pill and primary content without adding another history entry.
24. Refreshing a child URL reopens that child first, then restores its orchestration context for an authorized signed-in viewer.
25. Copying the address-bar URL after pill navigation copies the run currently in view. It does not copy a stable URL for the entire orchestration tree.
26. Two viewers who open the same child URL and can load the required run topology resolve the same ancestor chain and sibling cohort.

### Anonymous public links
27. This feature does not change the existing web authentication handoff for a public shared-session link. If that handoff cannot create a viewer workspace, the existing authentication or error UI remains in control.
28. A viewer who reaches the existing public child viewer without task access can continue to use that child through the session-sharing path.
29. The existing run metadata endpoint requires separate authenticated task access. Public shared-session access alone does not authorize the ancestor walk.
30. The first implementation supports orchestration context for signed-in viewers only. A public child viewer without task access remains a standalone child viewer with no pill bar.
31. Supporting orchestration context for logged-out public child links is deferred until it becomes a concrete requirement. That follow-up requires a new server-authorized topology source or equivalent share-scoped capability.

### Copy-link action
32. The first implementation leaves copy behavior unchanged. The copied URL identifies the run currently in view.
33. A canonical copy-link action is deliberately deferred. A future version may emit the top-level root route plus a child-selection fragment while the address bar continues to identify the run currently in view. This would buy back the canonical share identity that this design gives up, at the cost of adding fragment and restoration behavior from the competing design.

### Phase 0 disposition
34. The URL-preservation guard in PR #15317 suppresses the child-route update that this proposal keeps. It is contrary to this design.
35. Phase 0 does not land as a stopgap. The child-page pill-bar implementation ships directly.
36. The live URL-rewrite bug remains until the complete fix ships. The requester accepts this delay to avoid shipping and then reverting an intermediate behavior.

## Approach comparison

### Proposed: child page with orchestration context
Advantages:
- A direct child link stays where it points.
- Existing child links become richer without a redirect.
- The requested child remains usable when context hydration fails.
- The existing bar already supports parent anchoring and root breadcrumbs.
- No child-selection fragment or standalone escape hatch is required for the default design.
- Push-based pill navigation fixes the unrecoverable browser Back behavior directly.

Disadvantages:
- A cold child load performs one sequential request per ancestor level, then loads sibling and visible-group metadata.
- The viewer must hydrate conversations and hidden panes that were not part of the entry child.
- Deep ancestor chains require multiple bounded direct-child event subscriptions unless the server event contract gains parent attribution.
- The address bar follows navigation. A copied URL identifies the current run, not one canonical tree.
- Logged-out public share viewers need server work to receive the same context.

### Competing: root canonicalization with `#child=<run-id>`
Advantages:
- One stable root route represents the complete run tree.
- The fragment separates selected-run state from resource routing.
- Copying the address bar preserves both the canonical tree and selected child.
- Pill navigation changes only the fragment and never changes the route path.

Disadvantages:
- A direct child URL redirects somewhere other than where it points.
- The design needs child-fragment parsing, restoration, stale-fragment cleanup, and a standalone escape hatch.
- The Phase 0 guard must be removed or inverted before fragment writes can work.
- The address bar identifies the tree rather than the first-class run currently in view.

## Decisions
- The recommended design is the child page with orchestration context.
- The address bar follows the run currently in view.
- Signed-in ancestor resolution uses existing run metadata endpoints. It does not require a new GraphQL or REST endpoint.
- The ancestor walk loads the complete chain to the top-level root or falls back to the requested child.
- User pill selections push history. Incidental route updates replace history or perform no write.
- The existing pill bar and breadcrumb design remain unchanged.
- Logged-out public child links remain standalone in the first implementation.
- Phase 0 does not land.
- Canonical copy-link behavior is deferred.

## Out of scope
- Redesigning the pill bar, breadcrumbs, transcript viewer, or pane swap.
- Changing native desktop URL behavior.
- Redirecting a failed ancestor walk to the nearest loaded ancestor.
- Exposing partial ancestor identifiers to a viewer who cannot load the full chain.
- Adding orchestration topology access for logged-out public child links.
- Adding a canonical root-and-child copy-link target.
