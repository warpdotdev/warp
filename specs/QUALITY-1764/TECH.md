# Child-run deep links in the web session viewer

## Context
See [PRODUCT.md](./PRODUCT.md) for the approved behavior and delivery phases. This PR combines the specification with the narrow QUALITY-1764 fix as Phase 0. Phase 0 keeps the current `ConversationView` or `SessionView` URL and suppresses focused-child URL rewrites. It fixes the reported refresh and copied-link failure now, but it intentionally loses the child selection. Phases 1 and 2 replace that temporary behavior with permission-aware root routing and child-fragment navigation.

The web viewer is the WASM Warp client mounted by the React shell at [`client/src/app.tsx (277-293) @ df4801c`](https://github.com/warpdotdev/warp-server/blob/df4801cc884f3dbbd85bfdf8368bc362a63de4d1/client/src/app.tsx#L277-L293). The React entry points are [`client/src/ConversationView.tsx @ df4801c`](https://github.com/warpdotdev/warp-server/blob/df4801cc884f3dbbd85bfdf8368bc362a63de4d1/client/src/ConversationView.tsx) and [`client/src/SessionShareView.tsx @ df4801c`](https://github.com/warpdotdev/warp-server/blob/df4801cc884f3dbbd85bfdf8368bc362a63de4d1/client/src/SessionShareView.tsx).

In the Warp client:
- A child-pill click dispatches `RevealChildAgent` in [`app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs (1764-1781) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs#L1764-L1781).
- The pane swaps and focuses the child in [`app/src/pane_group/mod.rs (7231-7345) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/pane_group/mod.rs#L7231-L7345).
- Focus and `ManagerEvent::JoinedSession` both reach the dynamic URL writer in [`app/src/pane_group/mod.rs (6997-7047) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/pane_group/mod.rs#L6997-L7047) and [`app/src/pane_group/pane/terminal_pane.rs (280-292) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/pane_group/pane/terminal_pane.rs#L280-L292).
- The focused child reports its own `/conversation` or `/session` link in [`app/src/pane_group/pane/terminal_pane.rs (555-606) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/pane_group/pane/terminal_pane.rs#L555-L606).
- The browser handler commits that link with `history.replaceState` in [`app/src/uri/browser_url_handler.rs (9-33) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/uri/browser_url_handler.rs#L9-L33).
- `/session` query parameters are preserved while `/conversation` query parameters are dropped by [`app/src/uri/web_intent_parser.rs (49-91) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/uri/web_intent_parser.rs#L49-L91).

### Cold parent-resolution finding
Cold parent resolution requires server support.

The GraphQL `AIConversation` response contains `ambientAgentTaskId` but no parent or root reference in [`graphql/v2/ai_conversation.graphqls (4-33) @ df4801c`](https://github.com/warpdotdev/warp-server/blob/df4801cc884f3dbbd85bfdf8368bc362a63de4d1/graphql/v2/ai_conversation.graphqls#L4-L33). The Rust metadata fragment also has no ancestry field in [`crates/graphql/src/api/queries/list_ai_conversations.rs (48-63) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/crates/graphql/src/api/queries/list_ai_conversations.rs#L48-L63). Cloud conversion explicitly initializes `parent_agent_id` and `parent_conversation_id` to `None` in [`app/src/ai/agent/api/convert_conversation.rs (84-110) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/ai/agent/api/convert_conversation.rs#L84-L110). The existing local parent resolver therefore works only after the parent or its run-ID index is already in memory.

The server already stores every required relationship. `Task` carries `ParentRunID` and `AgentConversationID` in [`model/types/ai_tasks.go (519-557) @ df4801c`](https://github.com/warpdotdev/warp-server/blob/df4801cc884f3dbbd85bfdf8368bc362a63de4d1/model/types/ai_tasks.go#L519-L557). A conversation maps to a task through `GetTaskByAgentConversationID`, and a session UUID maps to a run execution. [`logic/root_run.go (11-48) @ df4801c`](https://github.com/warpdotdev/warp-server/blob/df4801cc884f3dbbd85bfdf8368bc362a63de4d1/logic/root_run.go#L11-L48) already provides a bounded, cycle-safe ancestor walk. No migration or new persisted relationship is required.

## Delivery phases

### Phase 0 — Preserve the entry viewer URL
Phase 0 is implemented in this PR.

`app/src/uri/browser_url_resolution.rs` adds the pure `resolve_browser_url` decision. For a non-forced request, it returns the current URL whenever the current URL parses as `ConversationView` or `SessionView`. `app/src/uri/browser_url_handler.rs` uses that decision before calling `history.replaceState`.

Both known unwanted write paths already call the same browser handler:
- A child-pill pane focus reaches `PaneGroup::focus` and `update_browser_url`.
- `ManagerEvent::JoinedSession` reaches `handle_pane_link_updated` and the same `update_browser_url`.

The guard therefore keeps the orchestrator's entry route for both a focused child's existing shareable link and a child session link that resolves later. It also keeps forced login and signup redirects unchanged.

Phase 0 has an intentional limitation: it does not encode the selected child. Refresh and copied links return to the orchestrator with no child selected.

Phase 0 is not a foundation for Phase 2. Its blanket early return suppresses every non-forced requested URL while the current URL is a viewer route. That includes the root URL with a new `#child=<run-id>` fragment. Phase 2 must remove or invert this early return before it wires user selection to the anchor writer. Do not layer fragment writes behind the Phase 0 rule. If both rules remain active, the anchor write silently resolves back to the unmodified URL and no fragment appears.

When Phase 2 replaces the guard:
- Preserve the Phase 0 invariant that generic pane focus and `ManagerEvent::JoinedSession` never replace the root path with a child's path.
- Allow explicit selection navigation to add or remove the child fragment.
- Rewrite the Phase 0 regression tests to distinguish a generic focused-pane request from an explicit selection request.

### Phase 1 — Canonicalize direct child routes

#### 1. Add a permission-aware route-resolution query in `warp-server`
Add an authenticated GraphQL query named `resolveChildRunViewerRoute`. The React viewer already runs inside `AuthenticatedApolloWrapper` with an authenticated or anonymous principal, so the query can use the same principal and authorization engine as existing conversation metadata.

Use this schema shape:

```graphql
enum WebSessionViewerRouteKind {
  CONVERSATION
  SESSION
}

input ResolveChildRunViewerRouteInput {
  kind: WebSessionViewerRouteKind!
  id: ID!
}

type WebSessionViewerRoute {
  kind: WebSessionViewerRouteKind!
  id: ID!
}

type ResolveChildRunViewerRouteOutput implements Response {
  childRunId: ID
  rootRoute: WebSessionViewerRoute
  responseContext: ResponseContext!
}

union ResolveChildRunViewerRouteResult =
    ResolveChildRunViewerRouteOutput
  | UserFacingError
```

`childRunId` and `rootRoute` are either both present or both null. Both null means "render the requested route without parent canonicalization." It covers a non-child, inaccessible root, incomplete ancestry, missing route locator, or safe fallback after an internal resolution failure.

Resolver flow:
1. Validate the input ID for its route kind.
2. Resolve the source run:
   - `CONVERSATION`: `AiTasksStore.GetTaskByAgentConversationID`.
   - `SESSION`: latest run execution for the shared-session UUID, then its `RunID`.
3. Require `ViewAction` on the source conversation or shared-session object. Do not return ancestry for a source the principal cannot view.
4. If the source run has no `ParentRunID`, return both nullable fields as null.
5. Call the existing bounded, cycle-safe root resolver. Do not fall back to the nearest known ancestor. A missing, cyclic, partial, or over-depth chain returns both fields as null.
6. Select the root route:
   - Prefer a reachable active shared session when the principal has `ViewAction` on that shared-session object.
   - Otherwise use the root's stored conversation when the principal has `ViewAction` on that conversation object.
   - Otherwise return both fields as null.
7. Return the source run ID as `childRunId` and the authorized root route.

The access check is part of route resolution, not a client-side follow-up. This is the enforcement behind PRODUCT behavior 22-24: the response must not disclose any root locator when only the child is accessible. Internal failures must log server-side and return a null resolution without internal details, so an accessible child remains usable.

Keep the current unauthenticated `/agent/sessions/:session_uuid/redirect` and `/agent/conversations/:conversation_id/redirect` endpoints for same-run live-session/transcript canonicalization. Do not add ancestry to those public responses.

#### 2. Resolve direct routes in the React shell before mounting WASM
Add the query and generated client types under `warp-server/client`. Update `ConversationView.tsx` and `SessionShareView.tsx` to run route resolution after their existing source-access check succeeds and before `WasmView` mounts.

For a non-null resolution:
1. Construct the root path from `rootRoute`.
2. Set `#child=<percent-encoded-childRunId>`.
3. Navigate with replacement semantics. Use React Router's replace navigation or `window.location.replace`; do not push a child redirect entry.

For a null resolution or query failure, continue the current direct-view flow. Do not turn route-resolution failure into a full-page error.

Read `view=standalone` from the raw browser query string before dispatching the resolution query. If the exact value is present, skip child-to-root resolution. Preserve `view=standalone` and the existing fragment across the current conversation-to-session and session-to-conversation redirects. Unknown `view` values do not suppress resolution.

Do not copy child-only query parameters, such as a child session password, onto a root route. The only cross-route query behavior introduced by this work is preservation of `view=standalone` across redirects for the same run.

### Phase 2 — Add anchor selection and history
Remove or invert Phase 0's blanket viewer-route preservation before implementing the work below. The Phase 2 browser URL API must distinguish explicit selection navigation from incidental pane focus. An implementation that leaves the Phase 0 early return in front of the anchor writer is incomplete even if its selection state changes in memory.

#### 3. Parse viewer location state separately from `WebIntent`
Keep route intent and viewer selection as separate concepts:
- `WebIntent` continues to choose `ConversationView` or `SessionView`.
- A new viewer-location parser reads the raw browser URL and returns:
  - the current root route and query string;
  - `standalone: bool`;
  - `selected_child_run_id: Option<String>` from the exact `child` fragment key.

Fragments stay client-only. Do not put the child selection in a query parameter and do not add it to the native `warp://` intent URL. This avoids the current `/conversation` query-dropping behavior and prevents the server/router from interpreting selection state.

The browser URL writer must mutate the fragment on a clone of the current root URL. It must preserve the root path and supported query parameters.

#### 4. Restore anchored selection after explicit hydration
The viewer must retain the parsed child run ID as pending state until initial orchestration hydration explicitly settles.

The existing viewer path already assigns child task IDs as run IDs and indexes them in `BlocklistAIHistoryModel` while registering children in [`app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs (351-460) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs#L351-L460). The run-ID lookup is exposed by [`app/src/ai/blocklist/history_model.rs (1555-1567) @ 04a7f83`](https://github.com/warpdotdev/warp/blob/04a7f8342c0b78978f12ecd2a3e032ff439bd56f/app/src/ai/blocklist/history_model.rs#L1555-L1567).

Add an explicit "initial child index settled" event to the orchestration hydration path. The event fires only after the initial ancestor snapshot has been applied and every child discovered by that snapshot has either registered a run-ID mapping or reached a terminal metadata-fetch failure.

`PaneGroup` owns pending anchored selection because it already owns pane materialization and swaps:
1. On each child registration, attempt `conversation_id_for_agent_id(pending_run_id)`.
2. When found, use the existing hidden-pane materialization and `swap_active_pane_to_conversation` path.
3. Mark the pending selection resolved without writing history. The loaded URL already contains the fragment.
4. If the settled event arrives with no match, select the root and remove the stale fragment using `replaceState`.

Do not use a timer to clear pending selection. Slow child discovery must not turn a valid link into the root fallback.

#### 5. Replace viewer-mode dynamic URL suppression with explicit selection history
Retain generic focused-pane URL behavior outside `ConversationView` and `SessionView`. In viewer mode, stop deriving the browser path from `focused_pane_content().shareable_link()`.

Replace the stopgap's "preserve the current viewer URL for every focused pane" rule with:
- A user child-pill action pushes the current root URL with `#child=<run-id>`.
- A user root-pill or in-view back action pushes the same root URL without the fragment.
- A browser `popstate` or fragment-history event selects the referenced child or root without writing history.
- Initial anchored restoration selects without writing history.
- `ManagerEvent::JoinedSession` preserves the root path, query, and current child fragment. It does not write the joined child's session URL.
- Generic focus, transcript hydration, and repeated selection do not write history.

Pass a navigation origin through the selection path, for example `User`, `BrowserHistory`, or `InitialUrl`, so the URL writer cannot recursively push while applying Back or Forward.

Use `pushState` only for a changed user selection. Use `replaceState` for a stale-fragment cleanup. The cold child-to-root replacement remains owned by the React shell.

#### 6. Preserve same-run automatic redirects
Both existing automatic redirect directions must carry viewer location state:
- Root deep link: preserve `#child=<run-id>`.
- Standalone child: preserve `?view=standalone`.

Construct the destination with the URL API instead of string concatenation. Copy only the supported state above. This prevents query/fragment ordering mistakes and avoids copying a child session's route-specific credentials to the root.

## Decisions

### Fragment versus query parameter
- Chosen: URL fragment `#child=<run-id>`.
- Advantages: client-only, survives copying and refresh, avoids `/conversation` query loss, and separates viewer selection from resource routing.
- Rejected: `?child=...`. It requires query propagation through `WebIntent` and gives server routing ownership of client-only state.

### Run ID versus conversation/session ID
- Chosen: run ID.
- Advantages: one durable identity across live `/session` and stored `/conversation` routes; already indexed by the viewer.
- Rejected: conversation or session ID. Either identifier can be absent or change relevance as a run transitions.

### Root versus immediate parent
- Chosen: top-level root.
- Advantages: one stable orchestration context for any tree depth.
- Rejected: immediate parent. Deep links would canonicalize to different viewer roots depending on depth.

### Standalone suppression
- Chosen: ship `?view=standalone` in the first implementation.
- Advantages: preserves a direct-child debugging and sharing path without changing the default canonical route.
- Rejected: defer the escape hatch. Deferral would make direct child viewing unavailable as soon as canonicalization ships.

### Child access without root access
- Chosen: render the accessible child at its original URL and return no root locator.
- Advantages: preserves valid child access and does not disclose an inaccessible root identifier.
- Rejected: redirect to the root and show an access error. This would break a valid child link and disclose a parent relationship the viewer cannot access.

### New authenticated query versus extending public redirect endpoints
- Chosen: a permission-aware GraphQL query.
- Advantages: uses the authenticated or anonymous principal already established by the React wrapper and can fail closed without leaking root identifiers.
- Rejected: add ancestry to the unauthenticated redirect endpoints. Those endpoints intentionally expose only same-run session/conversation canonicalization and cannot enforce the two-object access rule safely.

### History writes
- Chosen: replace cold redirects, push user selections, and perform no write while applying history.
- Rejected: replace every selection. It prevents Back from recovering the root and reproduces the reported history loss.

## Assumptions
- Existing root session and conversation access checks remain authoritative after navigation.

## Risks and mitigations

### Legacy children without durable run ancestry
Older children may lack an `ai_tasks` row, `ParentRunID`, or another link needed to walk from the child to the root. They may also lack the run ID needed for `#child=<run-id>`. Root canonicalization must not guess from conversation-local state or redirect to a partial ancestor.

Mitigation:
- The server returns a null route resolution for missing or partial ancestry.
- A direct child remains available at its original URL.
- A child pill without a durable run ID remains selectable but leaves the root URL unanchored.
- Tests include legacy/missing-link fixtures so this fallback remains intentional.

### Anonymous principal readiness
Link-shared routes create or load an anonymous principal before rendering the viewer, but route resolution crosses the React auth wrapper, GraphQL auth middleware, and object ACLs. A race or unsupported legacy anonymous session could leave no principal available when resolution starts.

Mitigation:
- `ConversationView` and `SessionShareView` do not call route resolution until their existing auth and source-access checks complete.
- The GraphQL resolver requires a principal and returns no ancestry without one.
- Resolution failure never blocks an accessible child; the client continues to the standalone child viewer.
- Tests cover anonymous link sharing with access to both resources and child-only access.

## Out of scope
- Persisting new ancestry data or migrating existing conversations.
- Adding ancestry fields to the transcript protobuf.
- Changing the session-sharing protocol.
- Redesigning orchestration discovery, pane materialization, or the pill bar.
- Native desktop URL handling.
- Nearest-ancestor fallback for broken trees.

## Testing and validation

### Phase 0 — implemented in this PR
The pure guard and its direct tests are in `app/src/uri/browser_url_resolution.rs:22-37` and `app/src/uri/uri_tests.rs:279-434`.

The Phase 0 branch has passed:
- `cargo nextest run -p warp -E 'test(/^uri::/)'` — 78 of 78 tests passed.
- `cargo check -p warp --lib` — passed.
- `cargo clippy -p warp --all-targets --tests -- -D warnings` — passed.
- `./script/wasm/bundle --check-only` — the Warp library type-checked for `wasm32-unknown-unknown`.
- `cargo fmt -- --check` on the touched files — passed.

The regression tests prove:
- A parent `/conversation` URL survives a child `/session` request.
- A parent `/session` URL survives a child `/conversation` request.
- The existing no-shareable-link fallback preserves a viewer URL.
- Non-viewer URLs still use the requested URL or `/app` fallback.
- Forced redirects bypass the guard.

Visual verification is blocked, not skipped. The local Postgres, Redis, Temporal, `warp-server`, WASM client, and classic web shell started successfully. The real browser flow stopped at Warp's sign-in screen. The environment had no real test-account credentials and no supported way to mint a valid local browser session. Phase 0 browser behavior therefore remains unproven by a live capture. The guard is verified by the direct unit tests and call-path tracing above.

To complete Phase 0 visual verification, provide a real test-account login or a supported non-UI method to mint a valid local browser session. Then open a parent run with a child, select the child, confirm the path remains the parent route, and refresh to confirm the parent reopens with no child selected.

### Phase 1 — `warp-server` resolver and React shell
- Add Go tests for `resolveChildRunViewerRoute`:
  - conversation source resolves a one-level child to the root;
  - session source resolves through its execution run ID;
  - a grandchild resolves to the top-level root;
  - a reachable root session wins over a root transcript;
  - a completed root falls back to its conversation;
  - non-child, missing ancestor, cycle, depth overflow, missing root route, and internal lookup error return a null resolution;
  - source-only access and revoked root access return a null resolution with no root identifiers;
  - access to both objects returns the route and child run ID.
- Run the focused Go package tests that contain the new resolver, then run `go test ./graphql/v2/resolvers/... ./logic/...`.
- Add Jest tests for `ConversationView` and `SessionShareView`:
  - default child route replaces to root plus fragment;
  - `view=standalone` skips root resolution;
  - null resolution and resolution failure render the child;
  - same-run redirects preserve standalone and child-fragment state;
  - cold redirects replace rather than push.
- Run `yarn --cwd client test --runInBand ConversationView SessionShareView`.
- Run `yarn --cwd client type-check`.

### Phase 2 — Warp WASM client
- Add parser unit tests for valid, missing, malformed, percent-encoded, and duplicate `child` fragments and exact `view=standalone` handling.
- Add `PaneGroup` or browser URL handler tests for:
  - child selection preserves root path/query and pushes one entry;
  - root selection removes the fragment and pushes one entry;
  - repeated selection is a no-op;
  - Back and Forward apply selection without new writes;
  - `JoinedSession` preserves the root route and current anchor;
  - initial restoration waits for the settled signal;
  - stale anchor cleanup selects root and replaces the URL.
- Run `cargo test -p warp web_intent_parser`.
- Run the focused `PaneGroup` and orchestration viewer-model tests added by the implementation.
- Run the repository's WASM compile check or `cargo check -p warp --target wasm32-unknown-unknown`.

### Final end-to-end visual proof
Use browser computer-use verification against an orchestrated run with at least two children. Record one video that proves:
1. Clicking two child pills changes only `#child=<run-id>`.
2. Browser Back returns through the first child to the unanchored root.
3. Refresh restores an anchored child inside the root viewer.
4. Opening a copied direct child URL replaces it with the root anchored URL.
5. Opening the same child with `?view=standalone` keeps the standalone child route.

Run a second access-control case where the test principal can view the child but not the root. Record that the child remains standalone and no root identifier appears in the URL or response.

## Parallelization
Phase 0 and the approved specification share `factory/quality-1764-preserve-viewer-url` and PR #15317. Keep this branch as the Warp integration point.

After spec approval, use two implementation branches because the server/React route resolver and the Warp WASM selection state are independent until integration:
- `factory/quality-1764-route-resolution` in `warp-server` owns the GraphQL resolver, access checks, React route handling, and their tests.
- PR #15317's Warp branch owns fragment parsing, pending selection, pane navigation origin, history handling, and WASM tests. Its Phase 2 change must replace or invert the Phase 0 guard before adding anchor writes.

The server branch lands first or provides a stable query contract. The Warp work can implement against the schema shape in this spec in parallel. Final end-to-end verification begins after a test environment contains both changes. The implementation produces one PR per repository; PR #15317 retains `specs/QUALITY-1764` and the phased history.
