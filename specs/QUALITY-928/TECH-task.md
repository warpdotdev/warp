# Task-Driven Orchestration Restore

Supplemental spec for QUALITY-928. Proposes replacing the DB-persistence
approach in `TECH-parent-restore.md` (Fix 1: persist the owner's parent
conversation; Fix 2: seed children from `task.children`) with a restore path
driven entirely by server task data. Fix 1 is reverted outright. Fix 2 is kept
but re-pointed at a different server query. The M1 discovery/streaming stack
(`OrchestrationChildTracker`, `FamilyDrainMode`, `drain_family_events`) is
unaffected.

## Problem

Fix 1 makes the owner's cloud agent parent conversation survive a restart by
persisting it to SQLite despite `is_viewing_shared_session`. This trades one
bug for a new failure class:

- **BUG-2.** On a second restart, the persisted parent conversation can
  restore with zero tasks. `AIConversation::new_restored_synthesizing_on_empty`
  (`app/src/ai/agent/conversation.rs:439-457`) treats an empty `agent_tasks`
  set as "child waiting on its first server response" and silently synthesizes
  a fresh optimistic root instead of failing — that heuristic is correct for
  a child conversation created moments ago, but wrong for a parent that should
  have a full transcript. Because a local row now exists,
  `resolve_entry_open_action` (`app/src/ai/agent_conversations_model.rs:1443`)
  resolves the pane to `RestoreOrNavigateToConversation` (a local-conversation
  action) rather than `OpenConversationTranscriptViewer`, so the transcript
  fetch that would otherwise repair an empty/stale local row never runs. The
  pane renders an empty transcript instead of the real one.
- **Stable-ID plumbing became load-bearing.** Once a client conversation id is
  persisted, it must stay stable forever, or the children's persisted
  `parent_conversation_id` orphans. `ensure_remote_child_conversation`
  (`app/src/ai/blocklist/history_model.rs:577-632`) carries a stale-parent
  reconciliation branch for exactly the case where a live-session rejoin mints
  a new parent id while DB rows still point at the old one — a class of bug
  that only exists because a parent id is expected to survive across
  sessions.
- **Two independent code paths do the same job for two different reasons.**
  DB persistence (native) and `task.children` seeding (web, and any native
  restore where the local row is missing or stale) both rebuild
  `children_by_parent`. They're individually reasoned about as
  order-independent and idempotent (`TECH.md` §9.5), but that's two restore
  mechanisms to keep synchronized rather than one.
- **The web client was already proof that persistence isn't required.** Fix 2
  exists because WASM has no SQLite (`app/build.rs` gates `local_fs` /
  `local_tty` on `target_family != "wasm"`) and restores entirely from server
  data. If server data is sufficient there, it's sufficient everywhere, and
  running two mechanisms in parallel on native is the part that's optional.

## Proposed design

The pane snapshot already stores the one piece of data that's stable across
restarts and across clients: `task_id` (`AmbientAgentTaskId`, the server-side
run id). Everything else needed to rebuild a restored parent's pill bar and
transcript is available from the server through APIs that are already wired:

1. `GET /agent/runs/{task_id}` — `get_or_async_fetch_task_data` (already used
   by `ambient_pane_restoration.rs` and `child_agent/restoration.rs`) resolves
   the parent task: title, state, `conversation_id`, ownership.
2. `GET /agent/runs?ancestor_run_id={task_id}` — `TaskListFilter::ancestor_run_id`
   (`app/src/server/server_api/ai.rs:701`) + `list_ambient_agent_tasks`
   (`app/src/server/server_api/ai.rs:1248-1252`) resolves the direct children.
   This is the same query `OrchestrationChildTracker`'s Observer side already
   uses for its cold-start seed (`spawn_ancestor_seed_fetch` /
   `finish_ancestor_seed_fetch`, `app/src/ai/blocklist/orchestration_event_streamer.rs:1324-1411`).
3. `task.conversation_id()` (`app/src/ai/ambient_agents/task.rs:270-272`)
   gives the parent's server conversation token, which is what
   `restore_pane_with_transcript` / `fetch_and_load_transcript`
   (`app/src/pane_group/ambient_pane_restoration.rs:185-268`) already use to
   load a transcript.

None of the three needs a persisted local row. Restore always mints a fresh
`AIConversationId` for the parent (as it already does for every other
in-memory-only conversation) and fully repopulates it — transcript, pill bar,
child links — from these three calls. No server change is required; only the
client-side wiring of *which* data restore reads changes.

```mermaid
flowchart LR
  TID["pane snapshot: task_id"] --> A["GET /agent/runs/{task_id}"]
  TID --> B["GET /agent/runs?ancestor_run_id={task_id}"]
  A --> C["task.conversation_id()"]
  C --> D["fetch_and_load_transcript<br/>(existing)"]
  B --> E["ensure_remote_child_conversation<br/>per child (existing)"]
  D --> F["restored parent pane,<br/>fresh AIConversationId"]
  E --> F
```

## Changes to M1

Unchanged: `OrchestrationChildTracker`, `FamilyDrainMode`,
`drain_family_events`, the family SSE stream, and everything in `TECH.md`
§4-§8. None of that discovers children via local persistence — it already
works from live server events and REST fetches — so it is orthogonal to this
change.

One M1 mechanism is reverted:

- **`ensure_remote_child_conversation`'s stale-parent reconciliation**
  (`app/src/ai/blocklist/history_model.rs:588-608`, the
  `if current_parent != Some(parent_conversation_id) { ... re-index ... }`
  branch). This branch exists solely to repair a persisted child row whose
  `parent_conversation_id` points at a parent id from a *previous* session
  that a live-session rejoin has since replaced. Once no parent id is ever
  loaded from a persisted row — every restore mints children fresh, under
  the current session's freshly-minted `parent_conversation_id` — the
  mismatch this branch handles cannot occur: `current_parent` is either
  `None` (first creation) or already equal to `parent_conversation_id`
  (idempotent re-run). Revert to the pre-fix, unconditional
  `return conversation_id` when `conversation_id_for_agent_id(run_id)`
  already resolves.

## Changes to M2

**Reverted (Fix 1):**

- `is_owned_cloud_agent_conversation` (`app/src/ai/agent/conversation.rs:3486-3506`)
  is removed. Nothing needs to know "does the current user own this run" for
  persistence purposes anymore.
- `write_updated_conversation_state`'s conditional early-return
  (`app/src/ai/agent/conversation.rs:3512-3516`) reverts to its original,
  unconditional form: any conversation with `is_viewing_shared_session` is
  never written to `agent_conversations`, full stop. A cloud agent parent
  pane is always rebuilt from `task_id` on restore, so there's nothing to
  persist and no ownership carve-out to reason about.

**Kept, modified (Fix 2):** `seed_child_conversations_from_task`
(`app/src/pane_group/child_agent/restoration.rs:79-182`) keeps its role as
the single function that rebuilds `children_by_parent` for a restored parent,
its idempotency guarantees (routes every child through
`ensure_remote_child_conversation`, which is a no-op for an already-linked
child), and its pending/retry shape. What changes is where the child list
comes from:

- Before: read `task.children` off the parent task fetched via
  `get_or_async_fetch_task_data(&parent_task_id)`. In practice this list is
  frequently empty — `GET /agent/runs/{id}` does not reliably populate
  `children` for the run's own record — which is why Fix 2 needed a fallback
  onto whatever the local index already held.
- After: issue `GET /agent/runs?ancestor_run_id={parent_task_id}` directly
  (mirroring `spawn_ancestor_seed_fetch`), and use that response's tasks as
  the child list. This is the same query path the Observer side already
  relies on for exactly this purpose, so it's a proven source rather than a
  best-effort field. The function no longer needs to fetch the parent task
  itself for this purpose (fetching `task.children` is dropped); it still
  needs `parent_conversation_id` (from its caller) to link children under and
  each child's own task data (already fetched per-child, unchanged) to name
  and place them.

**Kept, unchanged:**

- `process_pending_parent_child_seeds` / `pending_parent_child_seeds`
  (`app/src/pane_group/child_agent/restoration.rs:186-204`, field declared on
  `PaneGroup`): the ancestor-list fetch can still be in flight when restore
  first runs, so the same pending/re-drive-on-`TasksUpdated` shape is needed
  regardless of which server call populates the list.
- `load_data_into_restored_ambient_cloud_mode_view`'s `Option<AIConversationId>`
  return and both call sites that invoke the seed immediately after
  (`app/src/pane_group/mod.rs:3732-3741` and `:5377-5405`). The parent still
  needs a resolvable conversation id to seed children under, and the
  re-entrancy constraint that motivated returning it instead of reaching back
  into `PaneGroup` from an associated fn is unchanged.
- `restore_pane_with_transcript` / `fetch_and_load_transcript`
  (`app/src/pane_group/ambient_pane_restoration.rs:185-268`): this is now the
  *only* transcript-loading path for a restored completed parent (see below),
  so it stays exactly as it is.

**Removed:**

- The `RestoreOrNavigateToConversation` arm in
  `process_pending_ambient_restorations`
  (`app/src/pane_group/ambient_pane_restoration.rs:141-174`). This arm exists
  only to handle the case where `resolve_entry_open_action` sees a persisted
  local row for the parent and routes restore through a local-conversation
  navigation action instead of `OpenConversationTranscriptViewer`. With
  nothing ever persisted for the parent, `entry.identity.local_conversation_id`
  is never populated for a cross-restart pane, so
  `resolve_entry_open_action` (`app/src/ai/agent_conversations_model.rs:1443-1534`)
  always falls through to its last arm and returns
  `OpenConversationTranscriptViewer`. The arm — and the debug instrumentation
  added around it while chasing BUG-2 — is dead code once Fix 1 is reverted.

## Invariants preserved

- **Flag-OFF path.** Unaffected. All of the above is gated behind
  `FeatureFlag::OrchestrationUnifiedStack` already; flag-off restore keeps
  using the pre-M1/M2 baseline exactly as today.
- **Live session restore (case A).** Unchanged. A parent whose run is still
  `InProgress` with an attachable session resolves through
  `OpenOrAttachAmbientAgentConversation`
  (`app/src/pane_group/ambient_pane_restoration.rs:99-119`), rejoins the
  shared session, and `OrchestrationViewerModel`'s ancestor seed
  (`spawn_ancestor_seed_fetch`) rediscovers children exactly as it does today.
  This path never touched local persistence and needs no change.
- **Completed session restore (case B).** Same action
  (`OpenConversationTranscriptViewer`) and the same transcript-loading
  machinery (`restore_pane_with_transcript`,
  `load_data_into_restored_ambient_cloud_mode_view`, the seed call
  immediately after). The only thing that changes underneath is what feeds
  `seed_child_conversations_from_task`'s child list, per above.

## What's removed / simplified

- `is_owned_cloud_agent_conversation` (conversation.rs).
- The ownership carve-out in `write_updated_conversation_state`
  (conversation.rs) — reverts to a plain, unconditional early return.
- The `RestoreOrNavigateToConversation` arm and its `[ORCH-D:restore]` debug
  logging in `process_pending_ambient_restorations`
  (ambient_pane_restoration.rs).
- The `[ORCH-D:restore]` / `[ORCH-D:history]` debug logging added while
  diagnosing BUG-2 in `fetch_and_load_transcript`
  (ambient_pane_restoration.rs), `load_data_into_transcript_viewer` (mod.rs),
  and `index_child_conversation` / `ensure_remote_child_conversation`
  (history_model.rs) — once the underlying persistence path is gone, the
  conditions those log lines were added to observe no longer exist.
- The stale-parent reconciliation branch in `ensure_remote_child_conversation`
  (history_model.rs).
- `task.children`-reading in `seed_child_conversations_from_task`
  (child_agent/restoration.rs) — replaced by the ancestor-list fetch. Note
  `AmbientAgentTask.children` itself is not removed: it's still read by the
  legacy per-conversation restore-fetch path (`apply_task_children`,
  `orchestration_event_streamer.rs:2085-2099`), which is unrelated to this
  change.

Net effect: the parent conversation's identity is no longer expected to
survive a restart at all. There is exactly one restore mechanism (server task
data), not two order-independent ones, and it is the same mechanism on native
and web.

## Tradeoffs

- **Startup network dependency.** Restoring a cloud agent parent always
  requires at least the `GET /agent/runs/{task_id}` and, once there are
  children, the ancestor-list fetch. This is not new cost — both were already
  required for a correct restore under Fix 1 + Fix 2 (the parent task fetch
  to resolve `conversation_id()`, and either the DB index or `task.children`
  for the child list). What's new is that there's no persistence fallback: if
  the client is fully offline at restart, the pane cannot show a pill bar or
  transcript until connectivity returns. Given this is exclusively a
  cloud-agent (server-backed) feature, that's consistent with every other
  cloud-agent affordance already requiring connectivity.
- **No offline pill bar.** A restart with no network shows a loading state
  rather than a stale-but-present pill bar. Acceptable for the same reason:
  the underlying run only exists and only advances server-side.
- **Simpler restore surface.** No stable-conversation-id invariant to
  maintain, no DB-sync class of bug (BUG-2's failure mode cannot recur — an
  empty local row for the parent can't exist because no row is ever written),
  and no reconciliation pass. One less thing that must stay correct across
  every future change to the persistence layer.

## Pill bar notification gap on restore

Because children are never persisted, `children_by_parent` is always empty
the instant a restored parent pane is created — the only source of children
is the ancestor-list fetch kicked off by `seed_child_conversations_from_task`
(`restoration.rs:87-125`), which is async. `OrchestrationPillBar` is
constructed unconditionally in `TerminalView::new()`
(`app/src/terminal/view.rs:4211-4213`), i.e. before that fetch can possibly
resolve, and `pill_specs` (`orchestration_pill_bar.rs:600-624`) returns `None`
whenever the orchestrator has zero children. So the pill bar's first render is
always empty; whether it ever becomes non-empty depends entirely on the pill
bar receiving a re-render notification once `finish_seed_child_conversations_from_task`
(`restoration.rs:131-218`) finishes linking children.

**Traced notification chain.** `finish_seed_child_conversations_from_task`
routes every discovered child through `ensure_remote_child_conversation`
(`history_model.rs:591-639`). For a child never seen before in this session
(true for every child on a cold, unpersisted restart) this takes the `else`
branch: `start_new_child_conversation` → `start_new_conversation`, which emits
`BlocklistAIHistoryEvent::StartedNewConversation` (`history_model.rs:1345-1372`),
followed by `assign_run_id_for_conversation`, which emits
`ConversationServerTokenAssigned` (`history_model.rs:1487-1530`; per its own
doc comment this exists so `StartAgentExecutor` can resolve a pending
`start_agent` tool call, not for pill bar refresh).
`OrchestrationPillBar::new` subscribes directly to the `BlocklistAIHistoryModel`
singleton (`orchestration_pill_bar.rs:352-384`) and matches
`StartedNewConversation` (alongside `UpdatedConversationStatus`,
`AppendedExchange`, `SetActiveConversation`) to call `ctx.notify()`;
`ConversationServerTokenAssigned` falls through the wildcard `_ => {}` arm and
is dropped. Separately, `TerminalView::handle_ai_history_model_event`
(`view.rs:6044-6576`) calls `ctx.notify()` unconditionally at the end of
handling any event addressed to it, including `ConversationServerTokenAssigned`,
but that only re-renders the parent's own `TerminalView`, not `OrchestrationPillBar`
directly.

Net effect: today, `StartedNewConversation` incidentally does notify the pill
bar for the exact restore scenario (every child is genuinely new), so the pill
bar likely *does* converge once children are created. But this is fragile,
incidental coupling, not an intentional signal — nothing in
`orchestration_pill_bar.rs` documents that pill bar correctness depends on
`start_new_conversation` always being called for every newly-linked child.
Any future change that discovers/attaches a child through a path other than
`start_new_child_conversation` (e.g. a pre-created shell conversation, or a
different SSE/hydration path that calls `assign_run_id_for_conversation`
directly without minting a fresh conversation first) would silently reproduce
a permanently-empty pill bar, because the one event that's semantically
"this child now belongs under this parent" (`ConversationServerTokenAssigned`,
or the `set_parent_for_conversation` / `index_child_conversation` call inside
`ensure_remote_child_conversation`) is never observed by the pill bar.

**Recommended fix.** Make the notification explicit and independent of
incidental conversation-creation side effects, rather than relying on
alternative 4(a)'s fetch-timing change or 4(c)'s pre-population scheme:

1. Add `ConversationServerTokenAssigned` to the set of events
   `OrchestrationPillBar::new`'s subscription matches (`orchestration_pill_bar.rs:352-359`),
   calling `ensure_mouse_states` + `ctx.notify()` exactly as the existing four
   variants do. This event already fires for every remote child exactly once
   per `ensure_remote_child_conversation` call that actually links a child
   (new creation today; any future direct-attach path tomorrow), so this
   closes the gap without depending on `StartedNewConversation` continuing to
   fire incidentally.
2. Treat 4(a) (start the ancestor-list fetch in parallel with the transcript
   fetch in `process_pending_ambient_restorations`, `ambient_pane_restoration.rs:81-138`,
   caching the result for `seed_child_conversations_from_task` to consume
   synchronously) as a follow-on latency improvement, not a correctness
   requirement — it shrinks or eliminates the empty-pill-bar flash on restore,
   but (1) is what guarantees the pill bar eventually renders correctly even
   if the ancestor fetch is slow or retried.
3. Do not pursue 4(c) (pre-populating `children_by_parent` before the parent's
   `AIConversationId` exists): `children_by_parent` is keyed by the local
   parent id, which doesn't exist until the transcript loads, so this would
   need a second, task-id-keyed staging index that gets reconciled once the
   parent id is minted — reintroducing exactly the "two mechanisms to keep
   synchronized" complexity this spec sets out to remove (see Problem, bullet
   3).

**Estimated LOC.** Fix (1) is a one-arm addition to an existing match plus a
comment explaining why (~5-10 LOC) in `orchestration_pill_bar.rs`, no new
event variant or model changes required. The optional follow-on (2) is
larger: a new per-task cache field on `PaneGroup`, a fetch kicked off earlier
in `process_pending_ambient_restorations`, and a cache-check branch in
`seed_child_conversations_from_task` before falling back to today's fetch
(~60-100 LOC including the fallback path and a test covering both orderings
of transcript-fetch-first vs. ancestor-fetch-first completion).

## No-Persistence Dependencies

This section documents every place that currently depends on remote child
conversations (`is_remote_child=true` rows) being persisted to the local
`agent_conversations` table, discovered while investigating a regression from
a trial change that stopped persisting them. The trial change broke child
restore for a scenario this spec's Problem/Design sections do not cover:
a **local, non-viewer parent** (a regular local agent-mode conversation that
calls the `run_agents` tool, not `/cloud-agent`) with remote children. That
parent is a normal local conversation — `is_viewing_shared_session=false` —
so it is persisted today regardless of this spec, and would keep being
persisted under a broader no-child-persistence change. Only its children
(`is_remote_child=true`) would stop being persisted.

### 1. Where persisted remote-child data is read at startup

`BlocklistAIHistoryModel::new` (`history_model.rs:331-366`) runs
`initialize_historical_conversations` (`conversation_loader.rs:489-608`)
synchronously, in the constructor, over `multi_agent_conversations` — *every*
locally persisted `agent_conversations` row, loaded eagerly and fully by
`persistence::initialize` before any pane exists
(`app/src/lib.rs:1506-1519`, `persistence/mod.rs:297`). For each row it
deserializes `AgentConversationData` and calls
`resolved_parent_conversation_id_from_persisted_data`
(`history_model.rs:497-509`), which reads the persisted `parent_conversation_id`
/ `parent_agent_id` fields (`crates/persistence/src/model.rs:1176-1192`). If a
parent resolves, the row is treated as a child agent conversation and:

- `index_child_conversation(conversation_id, parent_id)` is called
  (`conversation_loader.rs:575`), populating `children_by_parent` —
  synchronously, at boot, before any UI exists.
- The full `AIConversation` is eagerly loaded (via `load_conversation_from_db`
  or direct conversion) into `conversations_by_id`
  (`conversation_loader.rs:593-608`), including its persisted `run_id` →
  `task_id` (`conversation.rs:636`, `:1210` in the persistence model).
- The row is excluded from `all_conversations_metadata` (`return None` at
  `conversation_loader.rs:609`), so it never appears in conversation
  history/sidebar lists — this exclusion is intentional and pre-existing,
  independent of this spec.

This is the **only** mechanism that populates `children_by_parent` and
resolves child `task_id`s for a local (non-ambient) parent today. There is no
lazy or async equivalent wired into the local-conversation restore path —
unlike the ambient/viewer path, which now uses the ancestor-list seed
(`seed_child_conversations_from_task`). Removing child persistence removes
this mechanism's only data source with nothing to replace it for local
parents.

### 2. `child_conversation_ids_of` callers

`child_conversation_ids_of` (`history_model.rs:658-663`) reads directly from
`children_by_parent` with no fallback. Callers:

- `restore_missing_child_agent_panes_for_parent` (`restoration.rs:44-46`) —
  the pane-materialization entry point (see §3).
- `descendant_conversation_ids_in_spawn_order` /
  `descendant_conversations_in_pill_order` (`orchestration_topology.rs:160-181,
  224-265`) — walked by the pill bar (`orchestration_pill_bar.rs:616`) and
  keyboard navigation (`adjacent_orchestration_child_conversation_id`).
- `orchestration_aware_conversation_status` / `has_local_orchestrated_children`
  (`orchestration_topology.rs:196-207, 412-424`) — status aggregation and
  cloud-handoff eligibility checks.

All of these are synchronous, read-only queries against whatever
`children_by_parent` currently holds. None of them know how to trigger a
fetch when the index is empty — they are consumers, not the seed. If
`children_by_parent` was never populated (no persistence, no seed run for
this parent), every one of these silently reports "no children" forever, not
just once.

### 3. `restore_missing_child_agent_panes_for_parent` / `create_hidden_child_agent_pane`

`restore_missing_child_agent_panes_for_parent` (`restoration.rs:38-75`) is
called from **two independent trigger groups**:

- Ambient/viewer restore, from `finish_seed_child_conversations_from_task`
  (`restoration.rs:211-215`) — runs (and re-runs on retry) after the
  ancestor-list seed links each child, so it's inherently async-tolerant.
- Generic pane lifecycle, via
  `restore_missing_child_agent_panes_for_terminal_pane_if_needed`
  (`restoration.rs:245-273`), called from `replace_pane` (`mod.rs:5001`),
  `restore_closed_pane` (`mod.rs:5566`), `add_pane_with_options`
  (`mod.rs:6770`), and `reattach_panes` (`mod.rs:7731`); **and** directly from
  `TerminalPane::attach`'s subscription to `AgentViewControllerEvent::EnteredAgentView`
  when `display_mode.is_fullscreen()` (`pane/terminal_pane.rs:333-347`).

The `EnteredAgentView` trigger is the one that fires for a restored **local**
agent-view conversation: session restore re-enters fullscreen agent view for
a conversation that was in that state when the session was saved, firing this
subscription once, synchronously, with **no pending/retry mechanism** — unlike
the ambient path's `pending_parent_child_seeds`. If `children_by_parent` is
empty at that moment (no persistence, nothing async has populated it yet),
this call finds nothing and is never invoked again for this parent+pane
combination in this session, short of another full pane-lifecycle event
(close/reopen, split, etc.).

Downstream, `create_hidden_child_agent_pane` (`restoration.rs:345-440`) reads
the child conversation object passed to it by the caller (already resolved
via `child_conversation_ids_of` + `conversation()`/`RestoredAgentConversations`,
see §5) — it does not touch the DB itself. With `OrchestrationUnifiedStack` on
and `child_conversation.is_remote_child()` true, it delegates to
`materialize_child_pane` (`child_agent/hydration.rs:115-156`), which
requires `child_conversation.task_id()` to already be `Some` — if not, it
logs a warning and returns without creating a pane
(`hydration.rs:128-131`). Without persistence, `task_id` is only ever
populated via `ensure_remote_child_conversation` → `assign_run_id_for_conversation`
(the ancestor-list seed path), which today is never invoked for local parents.

### 4. Local-parent + remote-children scenario, traced end to end

With today's persistence (before the trial removal): a user runs `run_agents`
from a local agent-mode conversation. The parent is a normal, persisted local
conversation; each remote child is created via `ensure_remote_child_conversation`
(from the live orchestration event stream) and persisted normally (no
`is_viewing_shared_session`/`is_remote_child` gate exists on
`write_updated_conversation_state` today — see `conversation.rs:3476-3569`).
On restart: `initialize_historical_conversations` bulk-loads every child row,
populates `children_by_parent` and `conversations_by_id` (with `task_id`
resolved from the persisted `run_id`) — all before any pane is constructed.
When the parent's terminal pane is reconstructed and re-enters fullscreen
agent view, `EnteredAgentView` fires, `restore_missing_child_agent_panes_for_parent`
finds the children already indexed, and `create_hidden_child_agent_pane` →
`materialize_child_pane` succeeds immediately because `task_id` is already
set. Pill bar and hidden panes both work.

With remote-child persistence removed and no other change: `children_by_parent`
is empty at boot (§1). `EnteredAgentView` fires as before, but
`restore_missing_child_agent_panes_for_parent` finds no children and returns
(§2, §3) — permanently, since nothing re-drives this call for a local parent.
Unlike the ambient/viewer path, **no ancestor-list seed is ever kicked off**
for a local parent today — `seed_child_conversations_from_task` is only
called from the ambient-pane restore call sites
(`load_data_into_transcript_viewer` and
`replace_loading_pane_with_restored_ambient_cloud_mode_pane_inner`,
`pane_group/mod.rs:3737-3739, 5401-5403`). This is the actual regression: not
just an empty pill bar (recoverable once children eventually get linked, per
the prior section), but **no hidden child panes ever materialize at all**, so
clicking a (non-existent) pill or otherwise trying to reach a child is
impossible — the children are functionally gone from the client until the
user interacts with the run again in a way that re-triggers live discovery
(e.g. the child sends new SSE events while the app is open).

### 5. `RestoredAgentConversations`

`RestoredAgentConversations` (`ai/restored_conversations.rs`) is a singleton
read-through cache keyed by an *already-known* `AIConversationId`
(`get_conversation` / `take_conversation`, lines 111-138). On a cache miss it
calls `load_from_db`, a direct `read_agent_conversation_by_id` lookup
(lines 83-107) — i.e. it is a **lazy hydration** helper for an ID the caller
already learned about some other way, not a discovery mechanism. Its only
caller in this flow is `restore_missing_child_agent_panes_for_parent`'s
fallback (`restoration.rs:64-67`), reached only for IDs `child_conversation_ids_of`
already returned — which itself requires `children_by_parent` to already have
an entry (§1-§2). Since `initialize_historical_conversations` already eagerly
loads every indexed child straight into `conversations_by_id`
(`conversation_loader.rs:593-608`), this fallback is rarely exercised today
and, like everything else in this list, depends on the same persisted rows —
it provides no independent resilience against removing them.

### 6. Conversation list / `AgentConversationEntry`

No new problem here. Child agent conversations are already excluded from the
conversation list/sidebar by design, independent of persistence:
`initialize_historical_conversations` never inserts a row that resolves to a
parent into `all_conversations_metadata` (`conversation_loader.rs:566-610`),
and `merge_cloud_conversation_metadata` separately filters out any server
token belonging to a conversation with `is_child_agent_conversation()`
(`conversation_loader.rs:387-405`). Removing DB persistence of children
doesn't change this — there was never a list entry to lose.

### 7. What must change for a clean no-persistence solution

The only path missing async-tolerant discovery is the **local-parent**
restore path. The ambient/viewer path already has everything it needs
(`seed_child_conversations_from_task`, `pending_parent_child_seeds`, the
`finish_seed_child_conversations_from_task` → `restore_missing_child_agent_panes_for_parent`
call chain); it just needs the pill bar notification fix from the prior
section. For the local-parent path:

1. **Call `seed_child_conversations_from_task` from the local-parent restore
   trigger, not just the ambient one.** In the `EnteredAgentView`
   (`display_mode.is_fullscreen()`) handler (`pane/terminal_pane.rs:333-347`)
   — or equivalently, at the top of `restore_missing_child_agent_panes_for_parent`
   itself, so every caller benefits — check `child_conversation_ids_of` is
   empty and the conversation has a resolvable `task_id()`
   (`conversation.rs:1037`; populated from the *parent's own* persisted
   `run_id`, which is unaffected by removing *child* persistence), then call
   `seed_child_conversations_from_task(conversation_id, task_id, ctx)`
   exactly as the ambient path does. `seed_child_conversations_from_task` and
   `finish_seed_child_conversations_from_task` are already generic — they
   only need a `parent_conversation_id` and `parent_task_id`, and
   `terminal_surface_id_for_conversation` / `pane_id_for_owned_conversation`
   resolve identically for local and ambient panes — so no change is needed
   inside `restoration.rs` itself. This one change also finishes the "single
   restore mechanism" goal from this spec's Design section: local and viewer
   parents converge on the same ancestor-list seed instead of local parents
   keeping a DB-only path.
2. **Accept a one-time, harmless ancestor-list fetch for local conversations
   that never spawned children.** Every local conversation restored into
   fullscreen agent view would now issue a `GET /agent/runs?ancestor_run_id=`
   call; for the common case of a plain single-agent conversation this comes
   back empty and `finish_seed_child_conversations_from_task` already
   no-ops cleanly on an empty list (`restoration.rs:166-178, 197-203`). This
   only fires on the user-driven "re-enter this agent's fullscreen view"
   event, not a background scan over all history, so the added load is
   proportional to how many agent conversations a user actually revisits.
   If this proves too costly in practice, a follow-on optimization is to
   persist a cheap boolean hint (e.g. `has_spawned_children`) on the *parent's*
   own `AgentConversationData` — set once, the first time
   `ensure_remote_child_conversation` links a child under it — and skip the
   fetch when the hint is absent. This is a hint on a row that is already
   persisted regardless of this change (the local parent), not a
   reintroduction of child persistence, so it doesn't reopen the stable-ID
   problem this spec removes. Start without the hint; add it only if restore
   network volume is measured to be a problem.
3. **No change needed to `create_hidden_child_agent_pane` /
   `materialize_child_pane`.** Once (1) makes `task_id` and
   `children_by_parent` populate the same way for local and ambient parents,
   these already work unmodified — they only ever depended on the
   `AIConversation` object having `task_id` set, not on how it got there.
4. **No change needed for `RestoredAgentConversations` or the conversation
   list.** Per §5-§6, neither depends on anything this change removes beyond
   what's already covered by (1).

**Estimated LOC.** The core fix (1) is small: a guard clause plus one call to
an already-generic function, added either in `pane/terminal_pane.rs`'s
`EnteredAgentView` handler or inside `restore_missing_child_agent_panes_for_parent`
(~15-25 LOC, plus tests exercising local-parent restore with 0 and N remote
children, and confirming the fetch does not run when `task_id()` is `None`).
The optional hint-based follow-on (2) is larger: a new `AgentConversationData`
field, a write site in `ensure_remote_child_conversation`, and a read/guard at
the call site from (1) (~30-50 LOC) — do not build this until (1) ships and
network volume is actually measured.

## Validation

Manual (dogfood, flag on, server emits deployed):

1. Start a cloud agent via `/cloud-agent`, have it run 2+ remote children to
   completion, then restart the client. Verify the parent restores as a
   transcript view and the pill bar shows one pill per child with the correct
   final status badge.
2. Restart a **second** time (the BUG-2 repro) and confirm the transcript and
   pill bar are identical to the first restore — no empty transcript, no
   empty pill bar.
3. Restart a **third** time for good measure; nothing should degrade with
   repetition, since nothing is accumulating in local state.
4. Open the same completed run's Oz session link in the web client. The pill
   bar and children must appear — this exercises the ancestor-list fetch on
   the client that has no SQLite at all, so it validates the primary
   mechanism rather than a fallback.
5. Restart while a cloud agent is still running: the pane must rejoin the
   live session and show pills (case A, unaffected by this change).
6. Kill and restart the client immediately after a run completes (tight
   race with any in-flight fetch) and confirm restore still converges once
   the fetches complete, without a partial or empty pane being left behind.
7. Inspect the `agent_conversations` SQLite table after a completed
   `/cloud-agent` run and confirm no row exists for the parent conversation
   (confirming Fix 1 is fully reverted, not just inert).
8. With the flag off, repeat scenario 1 and confirm prior (pre-M1) behavior
   is unchanged.
9. Run `./script/presubmit` — must pass cleanly.
