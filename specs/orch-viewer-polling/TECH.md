# Orchestration viewer polling — Tech Spec
## Context
`OrchestrationViewerModel` (`app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs`) drives the orchestration pill bar in the shared-session viewer. It is the only steady-cadence REST poller in the orchestration code paths and the only orchestration delivery path that has not migrated to SSE.
### How it works today
- Created lazily on the root viewer pane's `NetworkEvent::JoinedSuccessfully` (`viewer/terminal_manager.rs:797-816`). Per-child viewer panes are constructed with `enable_orchestration_polling = false`, so the model is not nested.
- Endpoint: `GET /agent/runs?ancestor_run_id={parent_task_id}` with `CHILD_DISCOVERY_FETCH_LIMIT = 100`. Despite the parameter name, the server filter is `WHERE parent_run_id = $1` (`warp-server/model/ai_tasks.go:1814-1818`, confirmed by `test/integration/public_api_run_parent_run_id_test.go:298-321`) — direct children only. The response is a snapshot of those children's task records.
- Cadence: `STATUS_POLL_INTERVAL = 5s` while at least one tracked child is non-terminal, `STATUS_POLL_INTERVAL_IDLE = 30s` once all are terminal. Idle polling never stops.
- Stale-fetch guard: `fetch_generation` is bumped before each dispatch; older callbacks are dropped.
- Kick: subscribes to `BlocklistAIHistoryEvent::AppendedExchange` and, only during the idle phase, issues an immediate fetch when an exchange lands on the orchestrator or a tracked child.
- Teardown: `stop_orchestration_polling` drops the model on `SessionEnded` (non-owner), `ViewerRemoved`, `FailedToReconnect`, or `DetachType::Closed` (via `TerminalManager::on_view_detached`, covering tab-close and split-pane-close). Spawn continuations become no-ops on entity drop; the in-flight HTTP request is not aborted explicitly.
### Why it needs to change
For one viewer pane on a remote orchestration with N children: 12 polls/min while anything is non-terminal, 2 polls/min forever after everything's done. N affects payload size, not request count. Multiple viewer panes of the same orchestrator each poll independently.
Specific defects:
1. Idle polling has no upper bound — finished orchestrations keep hitting the endpoint every 30s forever.
2. No transient/permanent error backoff; failed responses reschedule at the same cadence.
3. The kick guard does not dedupe in-flight fetches. `polling_handle` tracks the *timer* handle, not the *fetch* handle. After the timer fires and dispatches `fetch_children`, the slot still holds the (now-completed) timer handle until the response callback runs `schedule_next_poll`. An `AppendedExchange` in that window slips past the `is_none()` check and dispatches a second fetch; the first fetch's response is then discarded by the generation guard. Net effect: every qualifying `AppendedExchange` issues a redundant HTTP request whose response is thrown away.
4. The kick is gated on `all_terminal` and so does nothing while children are active — exactly when new children are most likely to appear.
5. No visibility/focus gating: the model polls at the same rate whether the viewer pane is visible or hidden behind another tab.
6. No coalescing across panes: two viewer panes of the same orchestrator each spin their own loop.
7. Full descendant snapshot every cycle — no `since` cursor on the endpoint.
### Other orchestration delivery for reference
- `OrchestrationEventStreamer` (`app/src/ai/blocklist/orchestration_event_streamer.rs`) is SSE-based for the orchestrator-owner side. It currently excludes `is_viewing_shared_session()` and `is_remote_child()` conversations via `is_remote_run_view`, so viewer placeholders are not served. Reconnect, cursor, proactive recycle, and consumer registration are well-tested there.
- `AgentConversationsModel` has a 30s `POLLING_INTERVAL` for the agent-management view, disabled when `AmbientAgentsRTC` is enabled. Under RTC it refreshes via `UpdateManagerEvent::AmbientTaskUpdated` pushed through Warp Drive RTC, throttled at 5s (see `specs/ambient-agent-rtc-refresh-throttle/TECH.md`). The RTC push is owner-scoped on the server (`warp-server/pubsub/subscriber/cloud_drive_subscriber_v2.go:581-596`), so it does not cover viewers of user-owned orchestrations.
## Goals
1. Replace `OrchestrationViewerModel`'s REST polling loop with event-driven delivery.
2. Real-time child discovery and per-child `ConversationStatus` updates.
3. Cross-pane dedupe of the SSE — multiple viewer panes on the same orchestrator share one connection (placeholders remain per-pane).
4. Consolidate orchestration delivery in `OrchestrationEventStreamer` rather than spinning a parallel SSE manager.
5. Auth model covers the same viewers the current REST poll does (any principal with `ViewAction` on the parent task), including teammates of a user-owned orchestrator.
## Non-goals
- Re-platforming the orchestration pill bar UI.
- Changes to how the parent's shared session is delivered (PTY / WebSocket).
- Migrating orchestrator-owner-side delivery to the new ancestor endpoint in this work. The existing per-run-ids SSE serves owner-side correctly today; migrating owner-side to subscribe via `ancestor_run_id` (one SSE instead of one-per-run-ids set, no reconnect-on-new-child) is plausible and probably desirable as a future-extension once viewer-side is stable and the endpoint has bedded down. Out of scope here to keep this PR series focused.
- Solving the external link-shared viewer case (a viewer with neither team membership nor direct `ViewAction`). That gap exists today on the REST path and is out of scope here.
- Transitive descendant scope (i.e. grandchildren). Today the orchestration model is single-level — children are spawned only by the top-level orchestrator. The new design matches that scope. Extending to transitive ancestry is a future extension if/when nested orchestration ships.
## Approach
Server adds `GET /api/v1/agent/events/stream?ancestor_run_id={id}&since={cursor}` that streams events whose `event.run_id`'s `parent_run_id` equals the given run — direct children only, matching the existing REST endpoint's scope on the server. The client extends `OrchestrationEventStreamer` to consume this stream in a new "viewer mode" that emits broadcast events keyed on `{parent_task_id, run_id, …}`. `OrchestrationViewerModel` becomes a thin consumer that subscribes to those events, maintains its own `run_id → conversation_id` map, and applies the resulting status writes locally via `BlocklistAIHistoryModel::update_conversation_status`.
Net steady-state cost for one viewer pane on an orchestrator with N children: one long-lived SSE per orchestrator, one REST snapshot on viewer open (to seed the child set and the SSE cursor), one REST fetch per newly-discovered child after open (for pill metadata), and no periodic polling.
Cross-pane dedupe falls out automatically because the streamer is a singleton: multiple viewer panes on the same `parent_task_id` share the same SSE. Each viewer pane still maintains its own placeholder conversations and `run_id → conversation_id` map — they can't be shared because each pane has its own `BlocklistAIController` and conversation tree.
## Server design
Sketched briefly; a paired `warp-server/specs/...` should land first with the full server-side details.
### Endpoint
`GET /api/v1/agent/events/stream?ancestor_run_id={id}&since={cursor}` served by `warp-server-rtc`, sibling to the existing `?run_ids[]=` stream. The parameter name `ancestor_run_id` mirrors the existing REST endpoint and is kept for naming consistency; under today's single-level orchestration model, the semantic scope is "direct children only" (same as the REST endpoint).
### Filter semantics
- An event is included iff `event.run_id`'s `parent_run_id` equals the given run. **Direct children only.** Matches the existing `?ancestor_run_id=` REST endpoint's scope on the server; no new recursive-descendant infrastructure required.
- The endpoint emits the same event types as the existing per-run stream — lifecycle (`run_in_progress`, `run_succeeded`, `run_failed`, `run_errored`, `run_cancelled`, `run_blocked`) and `new_message` — applied across the child set.
- The first lifecycle event for a previously-unknown child IS the spawn signal. No separate `child_spawned` event type on the wire.
- For new children spawned after the SSE opens, the server fan-out must add them to the active filter without requiring a re-subscription. Simplest implementation: on task insert, look up the parent and push the new run to any open ancestor-stream subscriber for that parent.
### Cursor
- The `since` parameter uses the same monotonic `sequence` field the existing streams use. The cursor is scoped to the parent's child set (i.e. the maximum sequence the client has seen across any direct child of this parent).
- On reconnect, the client sends the last sequence it persisted; the server replays all events with `sequence > since` for the child set.
- The server is the source of truth for sequencing; the client never invents sequence numbers.
### Auth
`ViewAction` on the parent task, evaluated at subscription time. This matches the existing REST `?ancestor_run_id=` semantics. Teammates of a user-owned orchestrator are covered; external link-shared viewers without `ViewAction` are not (same as today).
### Operational
- Cloud Run streaming timeout applies (~15–20m depending on configuration); reuse the existing `DEFAULT_AGENT_EVENT_PROACTIVE_RECONNECT = 14m` recycle which is safely under either bound.
- Same backoff schedule applies on errors (`AgentEventDriverConfig::retry_forever`).
- Server emits the same delivery counter / latency metric scheme as the existing endpoint for parity.
## Client design
Two implementation PRs plus a cleanup PR, all behind a feature flag so the old polling path stays alive until rollout. PR 1 is pure additive wiring (no behaviour change with the flag on or off); PR 2 turns the feature on by adding the SSE consumer and replacing the viewer's polling loop; PR 3 is post-rollout cleanup.
### Streamer ↔ viewer-model contract
The streamer broadcasts events for viewer-mode consumers; viewer models subscribe and translate to local placeholder conversations. Both events are parent-task-id-keyed so they can be delivered to all viewer-pane subscribers of the same orchestrator without per-pane streamer state. Each subscriber filters on `parent_task_id == self.parent_task_id` and maintains its own `run_id → conversation_id` map.
```rust path=null start=null
pub enum OrchestrationEventStreamerEvent {
    // existing variants...
    ChildSpawned {
        parent_task_id: AmbientAgentTaskId,
        run_id: String,
    },
    ChildStatusChanged {
        parent_task_id: AmbientAgentTaskId,
        run_id: String,
        status: ConversationStatus,
    },
}
```
This is the "broadcast" dispatch shape: the streamer holds a minimal known-child set per orchestrator (to dedupe `ChildSpawned`) and otherwise broadcasts events; viewer models do all conversation-level translation. Chosen over the alternative where the streamer holds per-pane placeholder maps and writes statuses directly. It is symmetric to existing `ctx.subscribe_to_model` patterns elsewhere in the codebase (`ActiveAgentViewsModel`, `OrchestrationPillBarModel`).
`new_message` events on the ancestor stream are *dropped* in viewer mode — viewers do not surface inter-agent messages, so the streamer's existing dispatch into `OrchestrationEventService` is suppressed for viewer-mode `parent_task_id`s.
### PR 1 — Streamer viewer-mode plumbing
Pure additive; the new code paths are unreachable until PR 2 wires them up. Shipped as commit `24477f400` on `matthew/orch-polling-pr2`, bundled with PR 2 in the same GitHub PR (`#11408`) since the two pieces interlock too tightly to land in separate review cycles.
- Add a per-orchestrator entry inside `OrchestrationEventStreamer`, keyed on `parent_task_id`, stored on `OrchestrationEventStreamer::viewer_mode_orchestrators: HashMap<AmbientAgentTaskId, OrchestratorStreamState>`. Reference-counted across viewer-mode consumer registrations. Entry point: `register_viewer_mode_consumer(parent_task_id: AmbientAgentTaskId, orchestrator_placeholder_conv_id: AIConversationId, consumer_id: EntityId)`. The existing `register_consumer(conversation_id, consumer_id)` is keyed on conversation; the viewer-mode entry is keyed on `parent_task_id` so multiple panes' orchestrator placeholders can share the same ancestor consumer. Pair with `unregister_viewer_mode_consumer(parent_task_id, consumer_id)` for `Drop`.
- Relax `is_remote_run_view` for `is_viewing_shared_session()` conversations only when the new feature flag is on. Keep `is_remote_child()` excluded — owner-side remote children still receive events through their parent's existing per-run-ids SSE.
- Add a private `lifecycle_event_type_from_wire(&str) -> Option<api::LifecycleEventType>` helper that owns the wire-string mapping (including deprecated legacy variants). Shared by `drain_ancestor_events` and `convert_lifecycle_events` so both paths agree on which event-type strings are recognised. Pair it with a `conversation_status_from_lifecycle_event_type` mapping helper (`LifecycleEventType → ConversationStatus`). Mostly 1:1 with the existing `conversation_status_from_state(AmbientAgentTaskState)` collapsing rules; the only call-out is `Blocked`, which today is emitted without a `blocked_action` payload (we accept empty, matching the REST path).
- Add the `ChildSpawned` / `ChildStatusChanged` event variants on `OrchestrationEventStreamerEvent`.
- Add an `is_known_child(parent_task_id, run_id)` helper inside the streamer's per-orchestrator entry so it can emit `ChildSpawned` once on first observation of a new run_id and only `ChildStatusChanged` thereafter.
### PR 2 — Ancestor SSE consumer + viewer-model migration
This is the PR that actually turns the feature on (still gated by the flag). It combines the streamer-side SSE wiring with the viewer-model rewrite — they were originally split as separate PRs, but without the viewer-model migration the streamer's emitted events have no consumer, and without the streamer the viewer model has no source of events, so the two land together. Shipped as commit `b27d694cb` on `matthew/orch-polling-pr2`, alongside the PR 1 commit in GitHub PR `#11408`.
**Streamer side:**
- Add ancestor SSE driver state on the per-orchestrator entry (`OrchestratorStreamState::sse_connection: Option<AncestorSseConnectionState>`, paired with `AncestorForwardingConsumer` and `AncestorSseStreamItem`), ref-counted across viewer-mode registrations. Opens when the first viewer registers; tears down when the last unregisters.
- Pick an `AgentEventSource` trait shape for the new endpoint. Recommended: extend the trait with a `filter: AgentEventFilter::AncestorRunId(String) | RunIds(Vec<String>)` enum so future endpoints drop in naturally. Alternative: a sibling `ServerApiAgentEventSource::open_ancestor_stream(...)` method.
- The existing `run_agent_event_driver::retry_forever` machinery is reused for reconnect, transient/permanent backoff, and proactive recycle. The driver itself doesn't need substantive new code, though the `AgentEventSource::open_stream(&[String], i64)` call site changes to accept the filter enum if that's the chosen trait shape — a small mechanical change that propagates from the trait.
- Seed the consumer from a one-shot REST `?ancestor_run_id=` snapshot on first open (see "Cold-start seeding" below). The snapshot is used to seed the consumer's known-child set and the SSE's `since` cursor so already-terminal children don't generate spurious `ChildSpawned` events.
- On each lifecycle or `new_message` event whose `run_id` is not yet known to the consumer, emit `ChildSpawned { parent_task_id, run_id }` once, then proceed to status dispatch.
- On every lifecycle event, emit `ChildStatusChanged { parent_task_id, run_id, status }` after applying the mapping.
- Persist the cursor on cursor-advance to the *orchestrator's* viewer placeholder conversation (not the child placeholders) via the existing `persist_event_cursor` machinery, with the server-update side disabled for viewer-mode conversations (see "Cursor consistency" below). One cursor per orchestrator, advanced whenever any child's event sequence advances.
**Viewer-model side:**
- Delete `polling_handle`, `fetch_generation`, `fetch_children`, `schedule_next_poll`, `maybe_kick_polling`, the `AppendedExchange` subscription, and the constants `STATUS_POLL_INTERVAL` / `STATUS_POLL_INTERVAL_IDLE` / `CHILD_DISCOVERY_FETCH_LIMIT` (unless still referenced by the seed fetch).
- On construction, call `OrchestrationEventStreamer::register_viewer_mode_consumer(parent_task_id, orchestrator_placeholder_conv_id, terminal_view_id)` so the streamer can route ancestor events to this pane's orchestrator placeholder.
- Maintain a local `run_id → AIConversationId` map.
- Subscribe to `OrchestrationEventStreamerEvent::ChildSpawned` and `ChildStatusChanged`. Filter both on `parent_task_id == self.parent_task_id`.
  - On `ChildSpawned`: spawn a `get_ambient_agent_task(run_id)` to fetch pill metadata; create the child placeholder conversation (existing `start_new_child_conversation` + `set_viewing_shared_session_for_conversation(true)` + `set_task_id` flow); record `run_id → conversation_id` in the local map; emit `EnsureSharedSessionViewerChildPane` once `session_id` is known.
  - On `ChildStatusChanged`: look up the placeholder via the local map; call `BlocklistAIHistoryModel::update_conversation_status` directly.
- Subscribe to `BlocklistAIHistoryEvent` for `ConversationServerTokenAssigned` and call `maybe_backfill_parent_agent_ids(event, ctx)` from `handle_history_event`. The legacy polling path performed this stamping via the `maybe_kick_polling` subscription pair; the streamer-driven path must keep the parent-agent-id backfill or downstream pill metadata that joins on `parent_agent_id` would silently miss late-arriving server tokens.
- On drop, unregister from the streamer's ref-counted consumer set.
- Fix the misleading comment at `orchestration_viewer_model.rs:243-245` while editing the file ("ancestor_run_id returns every descendant" — it does not).
- Rewrite `orchestration_viewer_model_tests.rs`. The polling-cadence fixtures don't apply; the `apply_children_fetch` semantics map almost directly to "apply spawn/status events." Pure-function tests for `conversation_status_from_state` are unchanged.
After this PR (flag on), the viewer model is a ~100-line event router with no timers and no REST loop. Flag off, the old code path stays.
### PR 3 — Cleanup
After broad rollout:
- Delete the feature flag and the old code path.
- Update `specs/review-sse-connection-strategy/TECH.md`. The "passive views never open their own SSE" invariant becomes "passive viewer-mode views open ancestor-scoped SSEs." Add a debug snapshot for the streamer's viewer-mode entries (mirroring the follow-up that spec already records).
### Cold-start seeding
Replay-from-zero on the SSE is impractical for long-lived orchestrations (hundreds to thousands of events for an active multi-child run). Instead, on first open of an orchestrator viewer:
1. One-shot REST `?ancestor_run_id={parent}` fetch — same endpoint the current poller uses, just once.
2. Read each direct child's `last_event_sequence: Option<i64>` (already on `AmbientAgentTask`, populated by the server today).
3. Compute `seed = max(last_event_sequence across children, locally persisted cursor)`. Locally persisted cursor wins if it's higher (catches the restore-from-disk case). Why `max(across children)` is safe: each child's `last_event_sequence` is the highest sequence the server has recorded for that child, so lower-sequence events for a *different* child don't exist yet at snapshot time. Anything the SSE would skip by starting at `max` has by definition already been incorporated into one of the child snapshots.
4. Open the ancestor SSE with `since=seed`. Only events after that point stream.
5. The snapshot also pre-populates the consumer's known-child set, so already-terminal children don't generate spurious `ChildSpawned` events when their first lifecycle event replays.
This matches the existing owner-side restore flow in `OrchestrationEventStreamer::on_restored_conversations` (`get_ambient_agent_task` + `max(sqlite_cursor, server_seq)`), just with a list-by-parent variant.
### Cursor consistency
The cursor is the only client-side state that has to survive across reconnects and across client restarts. Five points:
1. **Where it lives.** Reuse `AIConversation::last_event_sequence` on the viewer's local orchestrator-placeholder conversation. The streamer's existing `persist_event_cursor` writes to this field; the new viewer-mode code path uses the same plumbing. No new SQLite column needed. Each viewer pane keeps its own local copy — the cursor is per-pane local state, not server-observable.
2. **Server-side update is suppressed.** The existing `persist_event_cursor` ALSO pushes the cursor to the server via `update_event_sequence_on_server` so other clients of the same agent run can resume from the same point. For viewer-mode conversations this push must be suppressed: the viewer does not own the task, and writing to the server's task cursor would interfere with the orchestrator-owner's process resuming its own SSE. Gate the server-update branch on `!is_viewing_shared_session()` and early-return before touching the owner-side `self.streams` map so viewer-mode placeholders never leave a spurious owner-side `ConversationStreamState` entry behind.
3. **Monotonicity enforced at the call site.** `BlocklistAIHistoryModel::update_event_sequence` and the server-side `update_event_sequence_on_server` are both *set-not-max*. `persist_event_cursor` therefore folds every known prior value into one effective sequence at the top of the function: the incoming `sequence`, the in-memory `streams[conversation_id].event_cursor` (read without inserting, so viewer-mode placeholders stay out of the owner-side map), and the persisted `conversation.last_event_sequence()`. The effective sequence is then written to SQLite, written to the in-memory map (owner-side only), and pushed to the server (owner-side only). Without this fold a stale drain delivering an older sequence would clobber a higher persisted value.
4. **`on_restored_conversations` already early-returns** for viewer-side conversations (`is_remote_view` check before any cursor write). So no additional gating is needed there — the changes in (2) and (3) are the only places that needed new gating.
5. **Cursor scope.** The cursor on a viewer-mode conversation represents "highest sequence seen across the parent's direct child set." This is a different scope from the owner-side per-conversation cursor on the same orchestrator (which represents "highest sequence on `watched_run_ids`"). Both numbers come from the same server event log so they're comparable, but they live on different conversation rows (viewer-side placeholder vs. owner-side conversation), so they don't collide. Document this on the field via a doc comment.
### Hotfix candidate
*Obsolete (PR 2 shipped without a separate hotfix).* The kick-guard bug lived on the REST polling path that PR 2 deletes entirely, so patching `maybe_kick_polling` would have been thrown away within the same review cycle. Retained here for design context.
## Rejected alternatives
- **A — tighten the existing poller** (kick guard fix + error backoff + idle backoff + visibility gating). Local fixes inside `OrchestrationViewerModel` only. Doesn't change the fact that we're REST-polling, and most of the logic gets thrown away when the SSE path lands. Worth doing as a hotfix for the kick-guard bug if we cannot ship the new design immediately.
- **B — drive discovery from parent history events** (`AppendedExchange` + safety-net timer). Eliminates idle polling but still REST-fetches on each kick. Obsoleted by the ancestor SSE.
- **C — viewer subscribes to the parent's per-run_id SSE directly**. Discovers via parent-bound `new_message` events; still needs a REST snapshot for child metadata; doesn't catch children that fail before sending the parent anything; introduces a second SSE manager next to `OrchestrationEventStreamer`. Strictly weaker than the chosen design.
- **D — relax the streamer's `is_remote_run_view` exclusion and open one SSE per child placeholder**. Per-child SSEs without parent-scope leave discovery unsolved. The chosen design uses one parent-scoped SSE that delivers all child events together.
- **E (standalone) — client subscribes to the new ancestor endpoint without consolidating in the streamer**. Same wire shape but creates a parallel SSE manager. Worse for reconnect / cursor / proactive-recycle consistency.
- **F.cheap — REST ancestor watcher inside the streamer**. Useful only if the server endpoint can't ship first. Skipped because the server endpoint can ship first.
- **G — use `AgentConversationsModel`'s RTC pushes**. Owner-scoped on the server (`getUsersToNotifyForTask` returns task owner or team members), so teammate viewers of user-owned orchestrators never receive the push. Even if scope were relaxed, the dispatch-path question would still need answering. The chosen design subsumes it.
- **Dispatch via streamer-owned per-pane placeholder maps.** The streamer would hold per-orchestrator `Vec<ViewerRegistration>` and per-viewer `run_id → conversation_id` maps; `handle_event_batch` would iterate registrations and call `update_conversation_status` directly. Picked the broadcast-event option instead: streamer emits parent-task-id-keyed events, viewer models translate to local placeholder conversations. Cleaner separation, symmetric to existing event-subscription patterns, and the "more code per viewer" cost is small.
- **Transitive descendant scope.** Today the orchestration model is single-level. Designing for transitive ancestry now would require either a recursive CTE filter (expensive) or a denormalised `root_orchestrator_id` column (server schema migration). Out of scope; future extension if nested orchestration ships.
## Testing and validation
### Unit tests
- **Streamer ancestor consumer (`PR 1` + `PR 2`).** Lifecycle event for a known child → `ChildStatusChanged` emitted with the mapped status (one test per `LifecycleEventType`). Lifecycle event for an unknown child → `ChildSpawned` emitted exactly once, then `ChildStatusChanged`. New-message events do NOT route into `OrchestrationEventService` for viewer-mode parent_task_ids.
- **Ancestor consumer ref-counting (`PR 2`).** Two viewer-mode registrations for the same `parent_task_id` open one SSE; second unregister tears down; reconnect on flap. Stale cursor on registration is honoured.
- **Cold-start seeding (`PR 2`).** One-shot REST snapshot populates the known-child set; SSE opens with `since=max(child.last_event_sequence, local_cursor)`; replayed events for already-known children do NOT re-emit `ChildSpawned`.
- **Cursor scope (`PR 2`).** Server-update path is suppressed for viewer-mode conversations. Local SQLite cursor advances on each event applied. Restart-from-disk path uses the local cursor and does not double-deliver.
- **`OrchestrationViewerModel` as consumer (`PR 2`).** `ChildSpawned` event → child placeholder conversation created with correct `agent_name` / `set_task_id` / `is_viewing_shared_session=true`. Subsequent `ChildStatusChanged` for the same `run_id` updates the pill via the local map. `EnsureSharedSessionViewerChildPane` is emitted once per child, after `session_id` becomes known. Events for other `parent_task_id`s are ignored.
- **Two-pane fixture (`PR 2`).** Open two viewer panes for the same orchestrator. Confirm both create independent placeholders, both apply status updates locally, neither pane's writes leak into the other's history, and only *one* SSE connection is open across both panes. Closing one pane keeps the SSE alive (refcount=1); closing both tears it down.
- **Test fixture migration.** The existing pure-function tests (`maps_*_to_*`, `unknown_state_maps_to_error`) carry over. The `apply_children_fetch` integration tests are rewritten as streamer-event-driven tests with the same coverage.
### Manual validation
1. Open a shared session viewer for a teammate's orchestrator with active children. Observe the new SSE open in the network panel; no periodic `?ancestor_run_id=` REST requests after the cold-start seed.
2. Watch a new child spawn server-side — verify the pill bar updates within seconds without any REST polling (apart from the one-shot per-child metadata fetch).
3. Switch to a different terminal pane (hide the viewer). With the streamer's visibility gating, the SSE should tear down when the last viewer-mode consumer unregisters. Switch back — SSE re-opens with the persisted cursor; no events are lost.
4. Open the viewer for an orchestrator while the network is down. Observe the driver's backoff schedule. When the network returns, events resume from the persisted cursor.
5. Kill the app while the viewer is open, restart, rejoin the same orchestrator. The cursor is loaded from SQLite; the new SSE includes a `since=` parameter; no duplicate events appear.
6. Open the same orchestrator in two panes / windows. Confirm only one SSE is open (cross-pane dedupe). Close one pane — the SSE stays. Close both — the SSE tears down.
7. Open a viewer on an orchestrator whose children have already terminated. Confirm the cold-start REST snapshot populates the pill bar with their terminal status, and no spurious status flicker occurs as the SSE catches up.
### Server-side spot check
After PR 2 + flag on:
- For one viewer pane on a finished orchestration left open for 30 minutes: the prior baseline is ~60 hits to `/agent/runs?ancestor_run_id=` over that window. Expected post-change: one REST hit on open (the cold-start seed), plus one SSE that recycles every ~14m via the proactive reconnect.
## Risks and mitigations
- **`is_remote_run_view` relaxation scope.** Must apply to `is_viewing_shared_session()` only, NOT `is_remote_child()`. Owner-side remote children continue to receive events through the parent's existing SSE; a second SSE for the placeholder would double-deliver. Mitigation: explicit branch in `is_eligible` plus a test asserting owner-side remote children remain ineligible.
- **Cursor-server-update suppression.** The existing `persist_event_cursor` pushes cursors to the server. Viewer-mode conversations must not touch the server cursor for the underlying task. Mitigation: gate the server-update branch on `!is_viewing_shared_session()` and add a test that exercises the streamer's viewer-mode write path without a corresponding `update_event_sequence_on_server` call.
- **Streamer dispatch regression.** PR 1 changes `handle_event_batch`, which is hot code on the orchestrator-owner side. Mitigation: viewer-mode branch is gated on a per-conversation marker that defaults off for owner-side conversations; existing tests for `OrchestrationEventService::enqueue_event_batch` integration must continue to pass without modification.
- **Double-write during flag transition.** With the flag on, the streamer drives status updates via emitted events; with the flag off, the old polling loop does. While both code paths exist, ensure they cannot both run for the same conversation. Mitigation: the polling loop in `OrchestrationViewerModel::new` is gated on the same feature flag — flag on means no polling loop is created.
- **Cross-pane state divergence.** Two viewer panes of the same orchestrator subscribe to the shared `ChildSpawned` event. Race: one pane creates its placeholder before the other. The placeholders are independent (different `AIConversationId`s) but reference the same server-side run. Mitigation: viewer model uses its own local `run_id → conversation_id` map keyed on the run_id, so each pane resolves the right placeholder regardless of timing. Add a test fixture for the two-pane case.
- **`agent_id_to_conversation_id` collisions (pre-existing).** `BlocklistAIHistoryModel::agent_id_to_conversation_id` (`history_model.rs:238`) is a 1:1 index keyed on `orchestration_agent_id()`. Two viewer panes that create independent placeholders for the same server-side run can collide on this index, particularly on app restart where `restore_conversations` populates the index unconditionally for any conversation with an `agent_id_key()`. This is *not* introduced by the new design — current code already creates viewer placeholders via `set_task_id`. Mitigation: walk the existing `conversation_id_for_agent_id` call sites (`orchestration_events.rs:221,359,738`, `agent_view/orchestration_conversation_links.rs:39,127`, `agent_conversations_model/entry.rs:290`, `conversation_details_panel.rs:227`, `block/view_impl/orchestration.rs:24,95,805`) in PR 1 and confirm none are exercised by viewer-mode placeholders (all the current call sites are owner-side paths). If a viewer-mode use is found, scope the index to non-viewer conversations.
- **Multi-pane SSE refcount race.** First pane opens the SSE; second pane registers (refcount=2); first pane closes (refcount=1, SSE stays). Standard refcount territory but worth a test fixture, especially given the existing `streams` HashMap uses entity drop for cleanup.
- **`Blocked` payload.** The lifecycle event currently emits `Blocked` without `blocked_action`. The viewer's `ConversationStatus::Blocked` accepts an empty string and already does so on the REST path. Mitigation: documented; populate `blocked_action` on the server side as a separate follow-up if the surface gains value.
- **External link-shared viewer.** Users with neither team membership nor direct `ViewAction` on the parent are not covered by either the REST poll or the new SSE endpoint. Mitigation: documented in non-goals; tracked separately.
- **Cloud Run streaming timeout vs 14m proactive recycle.** Same risk as the existing stream. Mitigation: reuse `DEFAULT_AGENT_EVENT_PROACTIVE_RECONNECT` and the existing driver.
- **Server-side live-stream fan-out for new children.** The server must add newly-spawned children to active ancestor-stream subscriptions without requiring a re-subscription. Mitigation: pubsub fan-out on task insert can publish to a per-parent topic; subscribers register interest in the parent. Server team owns this; flagged in the paired server spec.
- **`update_conversation_status` echoing back through `TaskStatusSyncModel`.** `TaskStatusSyncModel::on_conversation_status_updated` (`app/src/ai/blocklist/task_status_sync_model.rs:146-148`) explicitly early-returns for `is_viewing_shared_session()` conversations. Mitigation: existing guard verified; add a test asserting no `update_agent_task` call is made for viewer-side status writes.
- **Race between REST snapshot and SSE open.** During the gap between the cold-start snapshot and the SSE handshake, new children may spawn or existing ones may receive events. Both flows converge correctly: a child spawned during the gap is reported as `ChildSpawned` when its first lifecycle event arrives on the SSE (its `run_id` isn't in the known-child set); a concurrent insert visible in the snapshot pre-populates the known-child set and the first SSE event becomes a `ChildStatusChanged`. Mitigation: documented; add a test that races a child-spawn with the seed.
- **`Drop` refcount race.** `OrchestrationViewerModel`'s `Drop` impl runs while the streamer may still be iterating its ancestor consumer. Standard reference-counting hazard. Mitigation: `unregister_viewer_mode_consumer` is idempotent and safe to call after the streamer has already torn down its entry (e.g., logout flow). Add a test fixture for the drop-during-iteration case.
- **Restore order for orchestrator vs child placeholders.** On app startup, `restore_conversations` loads orchestrator and child placeholders in an unspecified order. If the streamer subscribes for the orchestrator before child placeholders are restored, replayed `ChildSpawned` events would refer to children not yet in the history model. The existing `update_conversation_status` no-ops on missing conversations, so the worst case is a missed status flicker that the next event corrects. Mitigation: add a restored-state test fixture; if the missed-flicker behaviour is unacceptable, defer SSE registration until restore completes.
## Parallelization
The implementation spans two repositories (warp client and warp-server) with no overlapping code, so the bulk of the work can be delegated to parallel agents working on sibling worktrees. Three phases, three agents.
### Phase A — server endpoint and client wiring in parallel
Two agents run concurrently against a frozen wire contract (this spec).
- **Server endpoint agent.** Worktree `/Users/matthew/src/orch-polling/warp-server`, branch `matthew/orch-polling`. Owns: the `GET /api/v1/agent/events/stream?ancestor_run_id=` endpoint plus the paired `warp-server/specs/...` spec. Includes the per-parent pubsub fan-out (so new children appear without a re-subscription). Coordinates only at the wire contract; needs no client knowledge. Outputs a server PR ready for review.
- **Client wiring agent (PR 1).** Worktree `/Users/matthew/src/orch-polling/warp`, branch `matthew/orch-polling`. Owns: streamer viewer-mode plumbing per PR 1 — feature flag, `register_viewer_mode_consumer` / `unregister_viewer_mode_consumer` API, `is_remote_run_view` relaxation, `LifecycleEventType → ConversationStatus` mapping, `ChildSpawned` / `ChildStatusChanged` event variants, `is_known_child` helper. Pure additive; no behaviour change with the flag in either state. Outputs a client PR ready for review.
Both agents work in separate git repositories, so there are no merge conflicts between them. They coordinate only through this spec.
### Phase B — consumer + migration (after Phase A merges)
One agent picks up where Phase A left off, once both Phase A PRs are merged (or at least mergeable) and the server endpoint is reachable from staging.
- **Consumer + migration agent (PR 2).** Worktree `/Users/matthew/src/orch-polling/warp`, same branch (continuing from PR 1). Owns: the `AncestorConsumer`, the SSE driver wiring, the cold-start REST seed, the viewer-model rewrite, the test fixture migration, and the comment fix at `orchestration_viewer_model.rs:243-245`. Validates against staging via the manual checklist before opening the PR. Outputs a client PR ready for review.
This agent benefits from holding both halves of the change in one head: the streamer-side event emission and the viewer-model-side consumption are interlocked enough that splitting them across two agents would create unnecessary coordination overhead.
### Phase C — cleanup (post-rollout)
Manual, not delegated. After the feature flag has been at stable for long enough to confirm no regressions:
- Delete the feature flag and the old polling code path.
- Update `specs/review-sse-connection-strategy/TECH.md` (the "passive views never open their own SSE" invariant).
- Add the debug snapshot for the streamer's viewer-mode entries.
### Pre-launch checklist
Before launching the Phase A agents:
1. Final approval of this spec.
2. Decide the feature flag name (suggested: `FeatureFlag::OrchestrationViewerStreamer`).
3. Confirm the warp-server worktree is on `matthew/orch-polling` and clean.
4. Confirm the warp worktree is on `matthew/orch-polling` and clean.
5. Draft the agent prompts: each prompt should include this spec's section references and the explicit scope above.
### Coordination model
- All three agents share this spec as their source of truth. No agent-to-agent messaging required for normal operation.
- If the wire contract needs to change mid-work, the spec is updated first, then the agents are re-prompted from the new spec. Avoid out-of-band coordination.
- Each agent is expected to surface uncertainty by stopping and asking, not by deciding unilaterally. The orchestrator monitors for blocked status.
## Environment
- Client worktree: `/Users/matthew/src/orch-polling/warp`, branch `matthew/orch-polling`.
- Server worktree (sibling): `/Users/matthew/src/orch-polling/warp-server`, branch to match.
- Touched files on the client (Phase 1):
  - `app/src/ai/blocklist/orchestration_event_streamer.rs` — viewer-mode plumbing, ancestor consumer, broadcast events.
  - `app/src/ai/blocklist/orchestration_event_streamer_tests.rs` — new tests.
  - `app/src/terminal/shared_session/viewer/orchestration_viewer_model.rs` — consumer migration, local `run_id → conversation_id` map.
  - `app/src/terminal/shared_session/viewer/orchestration_viewer_model_tests.rs` — fixture rewrite.
  - `app/src/server/server_api.rs` — new ancestor-scoped stream helper alongside `stream_agent_events`.
  - `app/src/ai/agent_events/driver.rs` — extend the `AgentEventSource` trait with a filter enum (or add a sibling method for the ancestor endpoint).
  - `crates/warp_core/src/features.rs` — new feature flag.
- Touched files on the server: TBD pending the paired server spec.
- No other client files should need to change.
