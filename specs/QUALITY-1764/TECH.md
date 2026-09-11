# Child-run deep links in the web session viewer

## Context
See [PRODUCT.md](./PRODUCT.md) for user-visible behavior. [PR #15317](https://github.com/warpdotdev/warp/pull/15317) shipped Phase 0: the web viewer keeps its entry URL when child focus or session events request another pane URL. That stopgap intentionally loses the selected child on refresh and copy.

The remaining work must ship as one deliverable. A direct child redirect cannot be enabled before the root viewer can restore `#child=<run-id>`; otherwise the redirect lands on the root with the requested child unselected. Phase 0 also blocks every non-forced viewer URL change, so its guard must be replaced before either the cold redirect or pill anchor can work.

Relevant Warp client code at [`56e084445`](https://github.com/warpdotdev/warp/tree/56e084445d5c914983665a187d2d77300624c261):
- [`app/src/uri/browser_url_resolution.rs`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/uri/browser_url_resolution.rs) contains the shipped Phase 0 guard.
- [`app/src/terminal/shared_session/mod.rs (42-68)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/terminal/shared_session/mod.rs#L42-L68) exposes the task ID carried by a joined session.
- [`app/src/ai/agent/api/convert_conversation.rs (84-110)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/ai/agent/api/convert_conversation.rs#L84-L110) restores a conversation’s task ID.
- [`app/src/ai/ambient_agents/task.rs (205-260)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/ai/ambient_agents/task.rs#L205-L260) deserializes `parent_run_id`, child IDs, and session/conversation locators.
- [`app/src/server/server_api/ai.rs (2247-2256)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/server/server_api/ai.rs#L2247-L2256) calls `GET /agent/runs/{run_id}`.
- [`crates/warp_server_client/src/public_api.rs (30-76)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/crates/warp_server_client/src/public_api.rs#L30-L76) requires normal authentication for that call.
- [`app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs (351-460)`](https://github.com/warpdotdev/warp/blob/56e084445d5c914983665a187d2d77300624c261/app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs#L351-L460) registers root-viewer children by run ID.

Signed-in root discovery needs no `warp-server` change. Each existing run response supplies the next `parent_run_id` and the root’s route locators. Each request uses the endpoint’s existing authorization.

## Proposed changes

### Shipped baseline — Phase 0
`resolve_browser_url` currently preserves a `/conversation` or `/session` URL for every non-forced request. This keeps child-pane focus and `ManagerEvent::JoinedSession` from replacing the entry path. Forced login and signup redirects still apply.

The remaining implementation replaces this blanket rule. Do not add canonical redirects or anchor writes behind the Phase 0 early return; it will silently return the unmodified URL.

### Combined implementation — canonical child deep links
Implement the following steps in order on one branch. Do not release or enable cold canonicalization until anchor restoration in step 3 is complete.

#### 1. Replace the Phase 0 guard
Replace the forced/non-forced boolean decision with an explicit navigation origin. The URL layer must distinguish:
- incidental focus, transcript hydration, and session events;
- changed user selection;
- browser history;
- initial anchored restoration;
- cold child canonicalization;
- forced navigation such as login.

Preserve the shipped invariant that incidental events cannot replace the root path with a child path. Allow explicit anchor changes and cold canonicalization. Rewrite the Phase 0 tests around these origins before adding new URL behavior.

#### 2. Parse viewer location state
Parse the raw browser URL separately from `WebIntent`:
- Preserve the current viewer route and supported query parameters.
- Treat the exact, case-sensitive `view=standalone` value as standalone mode.
- Parse `selected_child_run_id` from the exact `child` fragment key.
- Treat a missing `child` key as root selection.
- Treat an empty, malformed, or duplicate `child` key as an invalid anchor.

Keep fragments client-only. Do not add selection state to native `warp://` intents or use a query parameter for it. Mutate a clone of the current root URL when writing selection.

#### 3. Restore anchors after root hydration
Keep a parsed child run ID pending until initial root orchestration hydration explicitly settles. On child registration, resolve the run ID through `BlocklistAIHistoryModel`, then use the existing pane materialization and swap path.

Initial restoration does not write history. If hydration settles without a matching child, select the root and remove the invalid fragment with `replaceState`. Do not use a timeout.

#### 4. Resolve a direct child’s root
Skip this step in standalone mode. Otherwise obtain the entry task ID from the loaded route:
- `/session/<uuid>`: wait for the session join result and read `SharedSessionSource::orchestrator_task_id()`.
- `/conversation/<token>`: read `ambient_agent_task_id` from restored conversation metadata.

If the route has no task ID, keep the existing standalone viewer.

Use `get_ambient_agent_task` for the entry run and each parent. Record the entry run ID, fetch the current run, and follow `parent_run_id` until a run has no parent. Track visited IDs and allow at most 64 parent edges.

Abort on an invalid ID, cycle, missing or unauthorized run, transport failure, or depth overflow. Discard the partial chain and keep the original child viewer. Never redirect to an intermediate ancestor.

A logged-out public viewer cannot call this authenticated endpoint. Its direct child link therefore remains standalone. Do not broaden authorization.

#### 5. Canonicalize to the resolved root
Enable this step only with steps 1-3 present:
1. Prefer the root task’s active, reachable session locator.
2. Otherwise use its stored conversation locator.
3. If neither exists, keep the child standalone.
4. Construct the root URL, set `#child=<percent-encoded-entry-run-id>`, and navigate with replacement semantics.

Do not copy child-only query parameters or credentials to the root. The replacement re-enters the existing root-viewer load path, which restores the anchor through step 3.

Preserve `view=standalone` across same-run session/conversation redirects. Unknown `view` values do not suppress canonicalization.

#### 6. Add selection history
- A changed user child selection pushes the root URL with `#child=<run-id>`.
- A user root selection or in-view back action pushes the unanchored root URL.
- Browser Back and Forward apply selection without writing history.
- Repeated selection, initial restoration, generic focus, transcript hydration, and session events do not write history.
- Automatic changes between the root’s session and conversation routes preserve the current child anchor.

Build redirect destinations with the URL API. Do not derive viewer navigation from the focused pane’s shareable link.

## Decisions
- Canonicalization and anchor restoration are one release unit. Shipping canonicalization alone would regress direct child links.
- Replace the Phase 0 guard first. Both cold canonicalization and anchor writes require explicit navigation origins.
- Use the existing run endpoint instead of a new route-resolution API.
- Canonicalize only after a complete root walk; partial results remain standalone.
- Identify selection by run ID in a client-only fragment.
- Replace cold canonicalizations, push changed user selections, and do not write while applying history.
- Reuse the existing root viewer after replacement instead of hydrating a child viewer with ancestor context.

## Assumptions
- The existing run endpoint remains authoritative for run topology and route locators.
- The root viewer can emit an explicit completion signal for its initial child index.

## Risks and mitigations
- **Guard ordering:** Phase 0 can discard all new writes. Replace it in step 1 and keep origin-specific tests.
- **Partial implementation:** A redirect without restoration loses the requested child. Keep cold canonicalization disabled until step 3 works.
- **Deep or broken ancestry:** Enforce cycle and depth bounds, then fall back to the original child without applying a partial route.
- **Public viewers:** Public session access does not grant run access. Keep the child standalone when the run request cannot start or returns unauthorized.

## Testing and validation
The merged Phase 0 implementation already has focused URI tests for viewer URL preservation, non-viewer fallback, and forced redirects.

Add focused Rust tests for the combined implementation:
- Navigation origins preserve incidental Phase 0 behavior while allowing cold replacement and anchor writes.
- Session and conversation entries resolve their task IDs.
- One-level and deep children replace to the top-level root plus the original run ID.
- An active root session wins over a stored conversation; a root without either locator keeps the child standalone.
- Standalone mode skips the walk and survives same-run redirects.
- Invalid, cyclic, over-depth, unauthorized, and failed walks keep the child standalone and never use an intermediate ancestor.
- Logged-out public entries do not navigate to an inaccessible root.
- Initial anchor restoration waits for hydration completion.
- Invalid and unmatched anchors select the root and replace the URL.
- Changed child and root selections push once; repeated selection and history application do not write.
- Back, Forward, session events, and root session/conversation transitions preserve the correct selection and anchor.

Run:
- The focused URI, pane-group, and orchestration viewer-model tests added by the implementation.
- `cargo check -p warp --lib`.
- `cargo clippy -p warp --all-targets --tests -- -D warnings`.
- `./script/wasm/bundle --check-only`.
- `cargo fmt -- --check`.

## Follow-up
A dedicated copy-link action that emits a canonical root link independently of the address bar is deferred. Standard address-bar copy continues to include the selected child anchor.

## Parallelization
Do not split the remaining work across repositories or release boundaries. Implement the ordered steps on one Warp branch and open one implementation PR.