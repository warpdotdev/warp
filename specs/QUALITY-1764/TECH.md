# Child-run deep links in the web session viewer

## Context
See [PRODUCT.md](./PRODUCT.md) for the approved behavior. This PR implements Phase 0 by keeping a viewer entry URL when child focus or session events request another pane URL. Phases 1 and 2 replace that temporary guard with root canonicalization and `#child=<run-id>` selection.

Relevant Warp client code at [`56e084445`](https://github.com/warpdotdev/warp/tree/56e084445d5c914983665a187d2d77300624c261):
- [`app/src/uri/browser_url_resolution.rs`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/uri/browser_url_resolution.rs) contains the Phase 0 URL guard.
- [`app/src/terminal/shared_session/mod.rs (42-68)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/terminal/shared_session/mod.rs#L42-L68) exposes the task ID carried by a joined session.
- [`app/src/ai/agent/api/convert_conversation.rs (84-110)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/ai/agent/api/convert_conversation.rs#L84-L110) restores a conversation’s task ID.
- [`app/src/ai/ambient_agents/task.rs (205-260)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/ai/ambient_agents/task.rs#L205-L260) deserializes `parent_run_id`, child IDs, and session/conversation locators.
- [`app/src/server/server_api/ai.rs (2247-2256)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/server/server_api/ai.rs#L2247-L2256) calls `GET /agent/runs/{run_id}`.
- [`crates/warp_server_client/src/public_api.rs (30-76)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/crates/warp_server_client/src/public_api.rs#L30-L76) requires the normal access token for that call.
- [`app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs (351-460)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs#L351-L460) registers root-viewer children by run ID.

Root discovery needs no new server API for signed-in viewers. Each existing run response supplies the next `parent_run_id`; each request applies the endpoint’s normal authorization. Both candidate designs required this same upward walk. Root canonicalization is simpler after the walk because it redirects to the root and reuses the existing root-viewer hydration path instead of rebuilding that context around a child viewer.

## Proposed changes

### Phase 0 — Preserve the entry viewer URL
The implemented `resolve_browser_url` guard keeps a current `/conversation` or `/session` URL for non-forced requests. Both child-pane focus and `ManagerEvent::JoinedSession` already use this browser handler. Forced login and signup redirects still apply.

This guard is temporary. It rejects every non-forced viewer URL change, including an explicit `#child=` write. Phase 2 must remove or invert it before adding anchor navigation. Do not put the anchor writer behind the Phase 0 early return.

### Phase 1 — Canonicalize direct child routes

#### Resolve the entry task
Read `?view=standalone` from the raw browser URL. When its exact value is present, skip root discovery.

Otherwise obtain the entry task ID from the loaded route:
- `/session/<uuid>`: wait for the session join result and read `SharedSessionSource::orchestrator_task_id()`.
- `/conversation/<token>`: read `ambient_agent_task_id` from restored conversation metadata.

If the route has no task ID, keep the existing standalone viewer.

#### Walk to the root
Use `get_ambient_agent_task` for the entry run and each parent:
1. Record the entry run ID for the final child anchor.
2. Fetch the current run and read `parent_run_id`.
3. Stop at the first run with no parent.
4. Otherwise fetch that parent and repeat.

Track visited run IDs and stop after 64 parent edges. Abort on an invalid ID, cycle, missing or unauthorized run, transport failure, or depth overflow. Discard the partial result and keep the original child viewer; never redirect to an intermediate ancestor.

Each hop is independently authorized by the existing endpoint. No `warp-server` schema, resolver, handler, or persistence change is required. A logged-out public viewer cannot obtain the access token required by this endpoint, so root discovery fails closed and the public child remains standalone.

#### Replace with the root route
After a complete walk with at least one parent edge:
1. Prefer the root task’s active, reachable session locator.
2. Otherwise use its stored conversation locator.
3. If neither exists, keep the child standalone.
4. Construct the root URL with the URL API, set `#child=<percent-encoded-entry-run-id>`, and navigate with replacement semantics.

Do not copy child-only query parameters or credentials to the root URL. The root route then enters the existing root-viewer load path; Phase 2 owns restoring the child selection after hydration.

Preserve `view=standalone` across existing same-run session/conversation redirects. Unknown `view` values do not suppress root discovery.

### Phase 2 — Restore anchors and selection history

#### Parse viewer location state
Keep route intent separate from viewer selection. A viewer-location parser reads the raw browser URL into:
- the current root route and supported query string;
- the exact `view=standalone` state;
- `selected_child_run_id` from the exact `child` fragment key.

Treat a missing `child` key as root selection. Treat an empty, malformed, or duplicate `child` key as an invalid anchor.

The URL writer mutates a clone of the current root URL. It does not add the fragment to native `warp://` intents or use a query parameter for child selection.

#### Restore after hydration
Retain an anchored run ID as pending state until initial root orchestration hydration explicitly settles. On child registration, resolve the run ID through `BlocklistAIHistoryModel` and use the existing pane materialization and swap path.

Initial restoration does not write history. If hydration settles without a matching child, select the root and remove the stale fragment with `replaceState`. Do not use a timeout.

#### Distinguish selection from incidental focus
Replace Phase 0’s blanket guard with an explicit navigation origin:
- A changed user child selection pushes the root URL with `#child=<run-id>`.
- A user root selection or in-view back action pushes the unanchored root URL.
- Browser Back, Forward, and initial restoration apply selection without writing history.
- Generic focus, transcript hydration, and `JoinedSession` preserve the root URL and current anchor without writing history.

Preserve `#child=` when the root automatically changes between its session and conversation routes. Build redirect destinations with the URL API.

## Decisions
- Use the existing run endpoint instead of a new route-resolution API. The endpoint already returns the topology and authorized root locators needed by the client.
- Canonicalize only after a complete root walk. Partial results remain standalone.
- Keep fragments client-only and identify selections by run ID.
- Replace cold canonicalizations, push changed user selections, and do not write while applying history.
- Reuse the existing root viewer after redirect instead of hydrating a child viewer with ancestor context.

## Assumptions
- The existing run endpoint remains authoritative for run topology and route locators.
- The root viewer can emit an explicit completion signal for its initial child index.

## Risks and mitigations

### Deep or broken ancestry
The walk is sequential and may encounter legacy or inconsistent records. Cache task responses, enforce the cycle and depth bounds, and fall back to the original child without applying a partial route.

### Public viewers
Public session access does not grant authenticated task access. Do not broaden authorization. Keep the existing public child viewer standalone when the run request cannot start or returns unauthorized.

### Phase 0 blocking anchors
The temporary early return can silently erase every Phase 2 fragment write. Remove or invert it first, then rewrite its tests around navigation origin rather than focused-pane URL.

## Testing and validation

### Phase 0
The existing direct tests in `app/src/uri/uri_tests.rs` cover viewer URL preservation, non-viewer fallback, and forced redirects.

The branch has passed:
- `cargo nextest run -p warp -E 'test(/^uri::/)'`
- `cargo check -p warp --lib`
- `cargo clippy -p warp --all-targets --tests -- -D warnings`
- `./script/wasm/bundle --check-only`
- `cargo fmt -- --check`

### Phase 1
Add focused Rust tests that verify:
- session and conversation entries resolve their task IDs;
- one-level and deep children replace to the top-level root plus the original run ID;
- an active root session wins over a stored conversation;
- missing root locators keep the child standalone;
- `view=standalone` skips the walk and survives same-run redirects;
- unauthorized, missing, malformed, cyclic, over-depth, and failed walks keep the child standalone and never use an intermediate ancestor;
- logged-out public entries do not expose or navigate to an inaccessible root.

Run the focused URI and viewer tests, `cargo check -p warp --lib`, and the WASM compile check.

### Phase 2
Add focused Rust tests that verify:
- valid, malformed, percent-encoded, and duplicate `child` fragments;
- initial restoration waits for hydration completion;
- stale anchors select the root and replace the URL;
- child and root pill selections push exactly once;
- repeated selection and history application do not write;
- Back and Forward restore root and child selections;
- `JoinedSession` and root session/conversation transitions preserve the current anchor.

Run the focused URI, pane-group, and orchestration viewer-model tests plus the WASM compile check.

## Parallelization
Do not split the remaining work across repositories. Phase 1 and Phase 2 both change Warp viewer routing state and should land sequentially on this branch.