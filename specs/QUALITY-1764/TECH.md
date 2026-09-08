# Child-page orchestration context in the web session viewer

## Context
See [PRODUCT.md](./PRODUCT.md) for the proposed user behavior and the side-by-side product comparison. This document specifies the child-page pill-bar approach. It is an alternate to the root-canonicalization plan in [PR #15317](https://github.com/warpdotdev/warp/pull/15317).

The web session viewer is the Warp WASM client mounted by the React shell. A route selects one shared session or stored conversation, then the WASM client joins or loads that run.
The route does not contain a run ID. `/session/<id>` carries a shared-session UUID, and `/conversation/<id>` carries a server conversation token. The implementation must resolve that locator to the selected run’s task ID before it can walk `parent_run_id`.

Relevant client behavior at [`warpdotdev/warp@1f0cf55`](https://github.com/warpdotdev/warp/tree/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a):
- [`app/src/uri/web_intent_parser.rs (33-91)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/uri/web_intent_parser.rs#L33-L91) parses session UUIDs and conversation tokens from web routes.
- [`app/src/terminal/shared_session/mod.rs (42-68)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/terminal/shared_session/mod.rs#L42-L68) obtains the run task ID carried by a joined shared session.
- [`app/src/terminal/shared_session/viewer/terminal_manager.rs (885-959)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/terminal/shared_session/viewer/terminal_manager.rs#L885-L959) constructs `OrchestrationViewerModel` with that task ID.
- [`app/src/ai/agent/api/convert_conversation.rs (84-110)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/ai/agent/api/convert_conversation.rs#L84-L110) stores `ambient_agent_task_id` as the run ID when it restores a cloud conversation.
- [`app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs (90-235)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs#L90-L235) treats the supplied task as the viewer-mode orchestrator and registers its children.
- [`app/src/ai/blocklist/orchestration_event_streamer.rs (1223-1538)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/ai/blocklist/orchestration_event_streamer.rs#L1223-L1538) seeds and streams children by `ancestor_run_id`.
- [`app/src/server/server_api/ai.rs (371-385)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/server/server_api/ai.rs#L371-L385) defines streamed run events without a parent run ID.
- [`app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs (630-756)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs#L630-L756) renders direct children of the active anchor, anchors a childless run on its parent, and resolves breadcrumbs to the loaded root.
- [`app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs (1764-1781)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs#L1764-L1781) maps pills to existing conversation and child-pane actions.
- [`app/src/pane_group/pane/terminal_pane.rs (1433-1478)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/pane_group/pane/terminal_pane.rs#L1433-L1478) materializes or reveals the target hidden pane and swaps it into view.
- [`app/src/pane_group/mod.rs (6997-7082)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/pane_group/mod.rs#L6997-L7082) derives the browser URL from the focused pane.
- [`app/src/uri/browser_url_handler.rs (9-33)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/uri/browser_url_handler.rs#L9-L33) currently commits non-forced URL changes with `history.replaceState`.

The existing run endpoint already carries the topology required by signed-in viewers:
- [`app/src/ai/ambient_agents/task.rs (220-290)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/ai/ambient_agents/task.rs#L220-L290) deserializes `parent_run_id` and `children`.
- [`router/handlers/public_api/agent_webhooks.go (1605-1640) @ 2682640`](https://github.com/warpdotdev/warp-server/blob/26826400a510c0b03ba3f64c89e91dce62ad50db/router/handlers/public_api/agent_webhooks.go#L1605-L1640) includes both fields in `TaskItem`.
- [`router/handlers/public_api/agent_webhooks.go (976-1055) @ 2682640`](https://github.com/warpdotdev/warp-server/blob/26826400a510c0b03ba3f64c89e91dce62ad50db/router/handlers/public_api/agent_webhooks.go#L976-L1055) authenticates each run request and checks task access.
- [`logic/ai/ambient_agents/dispatcher.go (2633-2664) @ 2682640`](https://github.com/warpdotdev/warp-server/blob/26826400a510c0b03ba3f64c89e91dce62ad50db/logic/ai/ambient_agents/dispatcher.go#L2633-L2664) enforces `AITask` `ViewAction`.

The missing behavior is client hydration. After a cold child route resolves its locator to a task ID, the current viewer treats that task as its orchestrator. Its seed and stream discover descendants of the child, while local history has no loaded ancestors or sibling panes. `drill_down_anchor_id` and `breadcrumb_ids` already render the desired structure after those records exist.

## Proposed changes

### 1. Separate the entry run from the orchestration root
Extend `OrchestrationViewerModel` to track:
- `entry_locator`: the browser’s shared-session UUID or server conversation token.
- `entry_task_id`: the run selected by the browser route.
- `context_root_task_id`: the top-level run after the ancestor walk succeeds.
- The ordered root-to-entry task chain.
- The initial context-hydration state: `Pending`, `Complete`, or `Standalone`.

Do not replace the active entry transcript while this state resolves. The active conversation remains the requested child.

Resolve `entry_task_id` before starting the walk:
- For `/session/<uuid>`, wait for `JoinedSuccessfully`, read `SharedSessionSource::orchestrator_task_id()`, fetch that task, and verify that its session locator matches the joined session.
- For `/conversation/<token>`, read `ambient_agent_task_id` from the restored `ServerAIConversationMetadata`.
- If either locator has no task ID or resolves to a different session or conversation, enter `Standalone`. Do not guess from another loaded run.

Root pages keep the current fast path. If the entry task has no `parent_run_id`, set `context_root_task_id = entry_task_id` and continue with current root hydration.

### 2. Walk ancestors with existing run endpoints
Use `ServerApiProvider::get_ai_client().get_ambient_agent_task` for every run lookup. Do not add or change a GraphQL or REST endpoint for signed-in viewers.

For a child entry:
1. Fetch the entry task if it is not already cached.
2. Read `parent_run_id`.
3. Fetch that parent through `GET /agent/runs/<parent-run-id>`.
4. Record the parent task and its `children` in the hydration snapshot.
5. Repeat with the parent’s `parent_run_id`.
6. Stop successfully at the first task with no parent.

The walk is sequential because each request reveals the next parent ID. Use a visited run-ID set and a maximum depth of 64, matching the bounded server helper in [`logic/root_run.go (11-48) @ 2682640`](https://github.com/warpdotdev/warp-server/blob/26826400a510c0b03ba3f64c89e91dce62ad50db/logic/root_run.go#L11-L48).

Abort the context load on:
- An unauthorized or unauthenticated response.
- A missing task.
- An empty or malformed parent ID.
- A repeated run ID.
- More than 64 parent edges.
- A transport or server error.

On abort, discard the incomplete ancestor snapshot and enter `Standalone`. Do not attach the entry conversation to the nearest loaded parent.

### 3. Hydrate each ancestor level’s sibling cohort
Each fetched parent returns the run IDs of its direct children. For each parent in the root-to-entry chain:
1. Validate that the next chain member appears in the parent’s `children` list. A mismatch aborts the complete context load.
2. Fetch metadata for the parent’s direct children that is not already cached. Fetch siblings at the same level concurrently.
3. Preserve the server’s child order.
4. Register the parent and children in `BlocklistAIHistoryModel` with their actual parent relationship.
5. Assign each conversation its durable run ID, display metadata, status, and available session or transcript locator.
6. For every child initially visible in an ancestor-level sibling cohort whose task has non-empty `children`, fetch and register that child’s direct children. The pill bar derives group state and subtree badges from loaded history relationships, not from `AmbientAgentTask.children` alone.
7. Run `decide_child_pane_materialization` for every registered task.
8. Emit `EnsureUnifiedViewerChildPane` or the legacy shared-session event only when the existing materialization rules produce an attachable session or loadable transcript.

Apply the snapshot from root to entry after all required requests settle. This prevents an incomplete chain from briefly appearing as a false root.

The initial scope loads the direct children required to render every visible group in the ancestor and sibling snapshot. It does not repeat step 6 for those newly fetched children until the viewer drills into that level. The implementation may batch already known task IDs through existing client caching, but it must not change ordering or completion semantics.

### 4. Stream each loaded parent with existing direct-child subscriptions
After the snapshot is applied:
- Register one viewer-mode consumer for each loaded parent whose direct children are part of the active pill and breadcrumb context.
- Supply that parent’s local conversation ID as its placeholder.
- Seed each consumer with `ancestor_run_id=<parent-task-id>`.
- Preserve the entry child as the active conversation.
- Attach each emitted child to the parent task ID that owns that consumer.
- Register an additional consumer when the viewer drills into a loaded group. Unregister consumers when their parent leaves the hydrated context.

`AgentRunEvent` does not carry a parent task ID. The current viewer streamer stamps events with the single parent supplied at registration. A single root consumer cannot preserve nested parentage.

Reuse the existing direct-child ancestor seed and SSE once per loaded parent. This adds client subscriptions but no server endpoint, schema, or event type. Bound concurrent subscriptions to the root-to-entry ancestor chain plus the current drill-down anchor. A depth-64 chain therefore has at most 65 active subscriptions. Wide sibling fan-out adds metadata requests and panes, but it does not add one subscription per sibling unless the viewer drills into that sibling as a parent.

Update `OrchestrationViewerModel::register_viewer_mode_consumer_if_possible` so it does not assume the active conversation is the only parent placeholder. It must register the parent conversations from the completed snapshot.

An alternative is to add nullable `parent_run_id` attribution to `AgentRunEvent`. The client could then use one root-scoped subscription and attach every event to its actual parent. This requires a server event-contract change but scales better for deep trees. Prefer this alternative if the field can be added compatibly at low cost; otherwise use the bounded per-parent design above.

### 5. Preserve existing pill rendering and pane actions
Do not fork the visual pill bar.

After hydration:
- A leaf entry resolves its parent through the local history and renders beside its siblings.
- A group entry anchors on itself and renders its direct children.
- Breadcrumb resolution walks the loaded chain to the top-level root.
- Existing `SwitchAgentViewToConversation` and `RevealChildAgent` actions continue to swap already registered conversations and panes.

If `decide_child_pane_materialization` returns `Pending`, keep the task’s conversation metadata available for topology and status. Add an explicit disabled state to `PillSpec`. A disabled pill does not dispatch a navigation action or write history. Re-enable it after refreshed task metadata produces `AttachLive` or `LoadTranscript`.

### 6. Distinguish user navigation from incidental URL updates
Replace the one-mode browser URL write with an explicit navigation mode:
- `PushSelection`: a changed, user-initiated pill or breadcrumb selection.
- `ReplaceSameRun`: a live-session-to-conversation locator change for the run already in view.
- `Preserve`: context hydration, generic focus, pane materialization, status updates, and repeated selection.
- `ApplyHistory`: a browser Back or Forward event.

`PushSelection` calls `history.pushState` with the selected pane’s existing safe session or conversation locator. It does not change React routes or reload WASM.

`ReplaceSameRun` calls `history.replaceState`. It must use the task-to-session and task-to-conversation mappings in the hydrated snapshot to verify that the requested locator belongs to the selected task ID.

`ApplyHistory` selects the conversation represented by the history entry without writing history again.

Pass the navigation mode through the pill action, pane swap, and browser URL handler. Do not infer user intent from focused-pane changes after the swap.

### 7. Apply browser Back and Forward to the hydrated viewer
Subscribe to browser `popstate` in viewer mode.

When a history entry points to a loaded run in the current tree:
1. Resolve the route to its registered conversation.
2. Reveal or switch to the existing pane.
3. Update the selected pill.
4. Perform no browser URL write.

History entries created by this feature always target locators that were navigable when selected. `popstate` does not reload the WASM app. If a prior locator cannot be resolved after refresh or access loss, explicitly hand the route back to the viewer loader or reload the current browser location. The destination then uses its standard standalone or error behavior. Do not leave the old pane visible under the new URL.

### 8. Do not land Phase 0
PR #15317’s Phase 0 guard suppresses the URL update that this design makes push-based. It has no merge path under this proposal.

- Implement navigation modes directly on the current focused-pane URL path.
- Do not copy the guard or its blanket viewer-route preservation rule into this branch.
- Keep PR #15317 unchanged as the competing root-canonicalization proposal until the requester chooses between the two PRs.
- Accept that the live bug remains until the complete child-page fix ships.

### 9. Keep anonymous public links standalone
The implementation is client-only only for a principal that can call the run endpoint.

A public shared-session viewer that completes the existing web authentication handoff can join with session-sharing identity, but the run metadata client has no anonymous-token fallback. The run route is independently protected and checks `AITask ViewAction`:
- [`app/src/terminal/shared_session/viewer/network.rs (231-253)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/app/src/terminal/shared_session/viewer/network.rs#L231-L253) sends a random anonymous session identity when no access token exists.
- [`crates/warp_server_client/src/public_api.rs (30-76)`](https://github.com/warpdotdev/warp/blob/1f0cf55afb29c71d94f2980b384aa11cb3cdb85a/crates/warp_server_client/src/public_api.rs#L30-L76) requires normal client authentication for public API calls.
- [`router/middleware/auth.go (31-45) @ 2682640`](https://github.com/warpdotdev/warp-server/blob/26826400a510c0b03ba3f64c89e91dce62ad50db/router/middleware/auth.go#L31-L45) rejects a missing principal.

For the first implementation:
- Preserve the existing web authentication handoff and its authentication or error UI when a workspace cannot be created.
- Treat an unauthenticated ancestor request as a context-hydration failure.
- If the existing viewer workspace is available, keep the public child session open standalone with no pill bar.
- Add no server work.

If anonymous orchestration context becomes a concrete requirement, write a separate server and security design that binds a share-session grant to the minimum topology and task metadata needed by the viewer. Do not grant general `AITask ViewAction` solely from possession of a public session link.

### 10. Defer a canonical copy target
Keep the current copy target: the selected run’s session or conversation locator in the address bar.

A future app copy action may:
- Resolve the loaded top-level root’s shareable route.
- Append `#child=<selected-run-id>` when the selected run is not the root.
- Do not change the address bar.
- Add fragment restoration for recipients of the copied link.
This deferred hybrid would add the fragment parsing and pending-selection behavior from the root-canonicalization design even though normal navigation remains child-route based. Its purpose is to buy back canonical tree sharing, which is the principal product advantage this design gives up.

## Decisions and trade-offs

### Child route versus root canonicalization
Chosen recommendation: keep the selected run’s direct route and hydrate its context.

Why:
- It makes existing child links survivable instead of redirecting them.
- It treats each run as an addressable resource.
- It uses the pill bar’s existing below-root rendering behavior.
- It fixes browser Back by changing the history operation at the source.
- It avoids fragment and standalone-suppression plumbing in the default design.

Trade-off:
- The URL follows the current selection. A copied address-bar link identifies the current run, not one canonical tree. Root canonicalization provides a stronger canonical share identity and a cheaper root-first data model. This is the strongest reason to choose PR #15317’s design instead.

### Existing endpoint walk versus a new resolver
Chosen for signed-in viewers: existing endpoint walk.

Advantages:
- No server schema or handler work.
- Each parent fetch uses the normal run authorization path.
- The parent task supplies the next parent ID and direct sibling IDs.

Disadvantages:
- One sequential request per ancestor level.
- The client owns cycle, depth, snapshot, and fallback handling.

Rejected for signed-in viewers: a new GraphQL or REST root resolver. It duplicates topology already present in `parent_run_id` and `children`.

### Complete snapshot versus incremental ancestor display
Chosen: complete snapshot.

Advantages:
- The bar never presents a partial parent as the root.
- Both cold and warm entry paths produce the same loaded chain.
- Failure has one clear fallback: standalone entry child.

Rejected: display each level as it arrives. It causes visible re-anchoring and exposes transient incomplete topology.

### Push versus replace
Chosen: push changed user selections; replace same-run locator changes.

`replaceState` caused the unrecoverable Back behavior in QUALITY-1764. Replacing every URL remains correct only for automatic changes that do not represent a new user selection.

### Per-parent subscriptions versus parent-attributed events
Chosen for the client-only implementation: one existing direct-child subscription per loaded parent in the active context.

Advantages:
- Uses the current event contract and authorization path.
- Preserves the correct parent for every direct-child event.
- Bounds subscriptions by ancestor depth and the active drill-down anchor instead of total sibling width.

Disadvantages:
- A maximally deep valid tree can require 65 concurrent subscriptions.
- Subscription registration and cleanup become client responsibilities.

Alternative: add nullable `parent_run_id` to `AgentRunEvent` and use one root-scoped subscription. Prefer this alternative if the event field can be added compatibly at low cost. It has the simpler runtime model, but it requires coordinated client and server work.

### Phase 0 versus direct implementation
Chosen: do not land Phase 0.

Advantages:
- Avoids shipping a URL-preservation rule that the final design must revert.
- Gives the implementation one navigation model and one test contract.

Disadvantage:
- The live bug remains until the complete context-hydration and history fix ships.

The requester explicitly accepts this delay.

### Anonymous orchestration context
Chosen: signed-in context hydration with standalone public fallback.

Advantages:
- Keeps the first implementation client-only.
- Preserves every currently valid public child link.
- Does not widen task access based on a public session link.

Deferred: share-scoped anonymous topology access. It requires a separate server and security decision.

### Canonical copy target
Chosen: defer.

The default copy target remains the selected run’s session or conversation locator. A root-plus-child copy target remains a valid follow-up because it directly addresses this design’s loss of one canonical share identity.

## Assumptions
- The existing run endpoint remains the authoritative topology source for signed-in viewers.
- The server’s depth limit of 64 parent edges is also the client’s validity limit.
- Existing task-to-session and task-to-conversation metadata can verify that a locator belongs to a selected run.
- A task may become materializable after its initial metadata load. The disabled pill state therefore listens to the existing task refresh path.

## Risks and mitigations

### Cold-load request and subscription volume
A depth-N child requires N sequential ancestor requests, sibling and direct-child metadata requests at each level, and direct-child subscriptions for the active loaded parents.

Mitigations:
- Reuse `AgentConversationsModel` task caching.
- Fetch siblings within one level concurrently.
- Fetch only the direct children required to render visible groups.
- Register streams only for parents in the active hydrated context.
- Apply one completed snapshot to the local history.

### Late and inconsistent topology
A parent may gain a child while the snapshot loads, or stored `children` and `parent_run_id` may temporarily disagree.

Mitigations:
- Require the chain member to appear in its parent’s child list.
- Abort an inconsistent initial snapshot instead of guessing.
- After completion, let the relevant parent-scoped SSE reconcile later direct-child events.

### URL writes racing with pane swaps
Generic focus and session-join events can run after a user pill selection.

Mitigation:
- Carry an explicit navigation mode through the action.
- Deduplicate writes by selected durable run ID and URL.
- Never infer push behavior from a focus event.

### Anonymous public links
Public session access and task topology access are different authorization surfaces.

Mitigation:
- Keep the standalone child usable when topology access fails.
- Review a narrow server capability separately if anonymous context becomes a concrete follow-up.

## Testing and validation

### Unit tests
Add ancestor-hydration tests covering PRODUCT behaviors 1-13:
- Session entry resolves its task ID after `JoinedSuccessfully` and verifies the session locator.
- Conversation entry obtains its task ID from restored server metadata.
- A missing or mismatched entry task ID enters standalone mode.
- Root entry performs no parent walk.
- One-level child loads its parent and siblings.
- A grandchild loads the complete chain and both sibling cohorts.
- A loaded group registers its direct children before rendering a group pill.
- Sibling requests at one level run concurrently.
- Cache hits do not repeat task requests.
- Missing parent, unauthorized parent, transport error, malformed ID, cycle, depth overflow, and parent/child mismatch discard the snapshot and enter standalone mode.
- A partial snapshot never mutates `BlocklistAIHistoryModel`.
- Separate parent consumers attach direct-child events to the correct local parent.

Add pill and history tests covering PRODUCT behaviors 14-26:
- A pending task renders a disabled pill and performs no navigation or history write.
- A disabled pill becomes navigable after task metadata becomes materializable.
- A leaf child anchors on its parent and shows siblings.
- A group child anchors on itself and shows direct children.
- Breadcrumbs resolve to the top-level root after a cold nested entry.
- A changed user pill selection performs exactly one push.
- Repeated selection performs no write.
- Same-run session-to-conversation resolution performs one replace.
- Incidental focus, materialization, and status updates perform no write.
- Back and Forward switch panes without recursive history writes.
- An unresolved popstate locator explicitly loads or reloads the destination route.
- Refreshing each session or conversation locator reconstructs the same selected run and context.

Add anonymous tests for PRODUCT behaviors 27-31:
- A web authentication handoff failure preserves the existing authentication or error UI.
- An existing public child viewer workspace remains usable standalone when run metadata authentication fails.
- An unauthenticated failure does not expose a partial pill bar or redirect the child.

Add copy tests for PRODUCT behaviors 32-33:
- Copying uses the current session or conversation locator.
- No root fragment is synthesized by the first implementation.

### Commands
Run the focused Rust tests added for:
- `orchestration_viewer_model`
- `orchestration_event_streamer`
- `orchestration_pill_bar`
- `pane_group`
- `browser_url_handler`

Then run:
- `cargo nextest run -p warp -E 'test(/^uri::/)'`
- `cargo check -p warp --lib`
- `cargo clippy -p warp --all-targets --tests -- -D warnings`
- `./script/wasm/bundle --check-only`
- `cargo fmt -- --check`

### Optional exploratory verification
Computer-use verification was not requested, so this section is not an acceptance criterion. When a browser-capable test environment and credentials are available:
1. Open a signed-in depth-two child link.
2. Confirm the URL remains the child route.
3. Confirm root and parent breadcrumbs plus the sibling cohort appear after hydration.
4. Navigate child → sibling → parent → root.
5. Confirm each address-bar route matches the run in view.
6. Confirm Back and Forward traverse those selections.
7. Refresh a child entry and confirm the same child reopens with its full context.
8. Open a public child link without task access. Confirm the existing web authentication handoff remains unchanged and, when the viewer workspace loads, the child remains usable without a pill bar.
9. Copy after navigating to a sibling and confirm the copied URL identifies that sibling.

## Parallelization
Use two local implementation agents after this spec is approved because hydration and browser history touch separable modules before integration:

- `context-hydration` owns `orchestration_viewer_model.rs`, `orchestration_event_streamer.rs`, history registration, pane materialization, and their tests. Use worktree `../warp-quality-1764-context` on branch `factory/quality-1764-context-hydration`.
- `viewer-history` owns `browser_url_handler.rs`, URL resolution, navigation-origin plumbing, popstate handling, and their tests. Use worktree `../warp-quality-1764-history` on branch `factory/quality-1764-viewer-history`.

Both agents work from the approved alternate-spec branch. The lead cherry-picks hydration first and history second into `factory/quality-1764-child-page-pill-bar-spec`, resolves overlap in pill-action and pane-swap plumbing, runs the full validation set, and updates these specs if implementation changes any behavior. The existing draft PR remains the single implementation PR.
