# Hydration Path Simplification

Supplemental spec for QUALITY-928. Describes planned simplifications to
`app/src/pane_group/child_agent/hydration.rs` after M2 merges.

## Already Applied (M1/M2)

The following simplifications have already landed:

- **`OrchestrationChildTracker` is mode-agnostic** — `new()` takes only
  `parent_task_id`. `OrchestrationEventConsumer` has been removed from the
  tracker entirely; Primary vs Observer is handled at the drain level only
  via `FamilyDrainMode`. The tracker treats every child the same regardless
  of which side of the family stream is consuming it.
- **`TrackedChild.conversation_id` removed** — the stand-in conversation ID
  (which carried a race risk when multiple children briefly shared the same
  placeholder ID) is gone. The tracker no longer holds any conversation
  reference; all history-model lookups go through `run_id` directly.
- **`ChildSignal::Registered` is a unit variant** — no `conversation_id`
  payload; `stamp_conversation_id_for_run` is gone.
- **`finish_remote_child_placeholder` no longer stamps the tracker** — follows
  from removing `TrackedChild.conversation_id`.
- **`FamilyDrainMode` is the sole mode concept** — `OrchestrationEventConsumer`
  is no longer used in code (only referenced in an old module doc comment).
  `FamilyDrainMode` in the streamer is the right granularity: it controls
  cursor authority and parent-self event delivery, nothing more.
- **`is_remote_placeholder` unification** — a module-level TODO documents the
  intent to merge `is_remote_child` and `is_viewing_shared_session` into a
  single persisted flag, making `is_durable_observer_parent` (M3) unnecessary.
- **`EnsureSharedSessionViewerChildPane` flag-ON branch is dead** —
  `OrchestrationViewerModel` emits `EnsureUnifiedViewerChildPane` (not
  `EnsureSharedSessionViewerChildPane`) when the flag is on. The flag-ON
  branch remains in `terminal_pane.rs` and is removed by Step 12 of this
  cleanup (it still compiles today because `attach_child_session` still
  exists; it breaks compilation only after Step 11 deletes that function).

## Problem

M2 introduced parallel owner/viewer implementations for every stage of child
pane materialization: two materialization arms, two transcript functions, two
pending maps, and two loading-placeholder wrappers. The duplication arose from
treating `ChildPaneOrigin` as the primary capability determinant. Capability
is actually determined by `TaskOwnership`, and both paths already run the same
ownership check — so the parallel structure expresses a distinction that doesn't
exist in the code.

Current function count in `hydration.rs` (flag-ON path): ~18 functions, ~1100 lines.
Target after simplification: ~11 functions, ~800 lines.

## Core Insight

**`ChildPaneOrigin` is a pane construction hint, not a capability gate.**

`completed_child_conversation_access` (the ownership check) is already called
identically in both `hydrate_owner_child_transcript` and
`hydrate_viewer_child_transcript_in_place`, and both route to the same
`replace_child_loading_with_continuation_pane` or
`restore_child_passive_transcript`. The `AttachLive` path does have a real
construction difference today — not just the pane type, but the
`TerminalManager` constructor:
- Owner uses `TerminalManager::new(is_ambient_agent=true,
  orchestration_child_conversation_id=None)`
- Viewer uses `TerminalManager::new_for_orchestration_child(conversation_id=Some(...))`
  with `is_ambient_agent=false`

The `orchestration_child_conversation_id` field is load-bearing: it routes
`FailedToJoin` events through `OrchestrationChildSharedSessionJoinFailed` →
`recover_viewer_child_join_failure` instead of showing a generic toast. The
current owner path lacks this routing (a gap). Change 2 resolves both by
introducing a single constructor that combines both properties.

## Proposed Changes

### 1. Unify the transcript functions

`hydrate_owner_child_transcript` and `hydrate_viewer_child_transcript_in_place`
are structurally nearly identical:

- Load the server transcript
- Call `completed_child_conversation_access` (same ownership check)
- Call `completed_child_presentation` (same branching)
- Route to `replace_child_loading_with_continuation_pane` or
  `restore_child_passive_transcript`

Two differences exist. First, the viewer path has a 6-line active-conversation
staleness guard (bail if the pane's active conversation no longer matches the
child — a race between async transcript fetch and pane supersession). Second,
the owner path uses a single `let Some(CloudConversationData::Oz(cloud)) = ...
else { warn; return }` to reject non-Oz responses, while the viewer path
handles `CLIAgent` and `None` with separate explicit arms and distinct warn
messages.

**Replace both with:**

```rust
fn hydrate_child_transcript(
    pane_id: PaneId,
    child_id: AIConversationId,
    task_id: AmbientAgentTaskId,
    server_token: ServerConversationToken,
    ctx: &mut ViewContext<Self>,
)
```

The staleness guard is applied unconditionally — it is harmless on the owner
path since the pane is already canonical. The unified function adopts the
viewer's explicit per-variant error handling for non-Oz responses (`CLIAgent`,
`None`) to produce distinct warn messages for each failure mode.

**Pros:** ~85 lines saved, single source of truth for transcript logic.
**Cons:** None significant.

### 2. Universal ambient pane for all live children

Currently `materialize_viewer_child_pane`'s `AttachLive` arm calls
`attach_viewer_child_session`, producing a plain shared-session viewer pane
(`is_ambient_agent = false`) with no ambient model. This means:
- An owner observing their own cloud run via a shared link gets a read-only
  pane for live children with no ambient controls
- A collaborator gets the same plain pane
- Both paths lack `FailedToJoin` recovery (owner path never had it; viewer path
  has it via `orchestration_child_conversation_id` but that field disappears
  if we naively replace with `attach_owner_child_session`)

**Change:** Introduce `attach_ambient_orchestration_child_session` — a new
unified attach function that replaces `attach_child_session`,
`attach_owner_child_session`, and `attach_viewer_child_session` for all child
live-session panes.

This requires a new `TerminalManager` constructor,
`new_for_ambient_orchestration_child(session_id, conversation_id, ...)`, in
`terminal/shared_session/viewer/terminal_manager.rs`. It combines:
- `is_ambient_agent = true` — creates ambient model upfront, wires
  `wire_ambient_agent_session_events`, uses
  `TerminalModel::new_for_cloud_mode_shared_session_viewer`
- `orchestration_child_conversation_id = Some(conversation_id)` — routes
  `FailedToJoin` through `OrchestrationChildSharedSessionJoinFailed` →
  `recover_viewer_child_join_failure` for all child panes (fixes the owner gap)

`new_for_orchestration_child` (flag-OFF path) is unchanged.

A matching `create_ambient_orchestration_child_pane` helper in `mod.rs`
replaces `create_orchestration_child_shared_session_viewer` for the flag-ON
path (~22 lines, net zero change in mod.rs).

**Session-end behavior:** With `is_ambient_agent = true` universal,
`handle_viewer_session_end` routes all child pane session ends through
`end_current_ambient_session`. That function checks `owned_ambient_agent_task_id`
and sets `NotShared` (editable, follow-up input visible) for owners and
`FinishedViewer` (read-only) for collaborators. This is the correct behavior:
owners may legitimately continue a child task as a standalone cloud conversation.

**`FailedToJoin` recovery:** `recover_viewer_child_join_failure` now handles
all child panes. The recovery is bounded: each retry requires a
`evict_and_refetch_task` round trip; the stale-session guard (`failed_viewer_child_sessions`)
prevents re-attaching to the same dead session; and once the task moves to a
terminal state (`LoadTranscript`) the entry is removed from pending. No busy loop.

**`enter_viewing_existing_session` audit:** This function issues a
`get_ambient_agent_task` fetch per child pane (one extra round trip per
collaborator, acceptable at 2–8 children). It calls
`apply_viewed_task_config_snapshot`, which writes the child task's model
preference to `LLMPreferences` scoped to this `terminal_view_id`. This write
is not global; it already happens for owner child panes today. No new risk.
It does not emit `ExecutionSessionReady` and does no server write-back.

Stale-session guard (`failed_viewer_child_sessions`) becomes universal: applied
unconditionally in the unified `AttachLive` arm for all child origins, including
in `process_pending_child_hydrations`. Making it universal eliminates the
current gap where an owner-origin child could get stuck on a dead session_id.

**Pros:**
- Single `attach_ambient_orchestration_child_session` replaces three functions
- `FailedToJoin` recovery now works for owner child panes (fixes existing gap)
- Owners always get ambient controls on live child panes in viewer contexts
- Collaborators see informative ambient UI (environment, harness)
- `EnsureSharedSessionViewerChildPane` handler and
  `restoration.rs::ensure_shared_session_viewer_child_pane` become confirmed
  dead code and can be deleted (~120 lines)

**Cons:**
- One extra `get_ambient_agent_task` round trip per collaborator per child pane
  to resolve harness/environment for the ambient UI. Acceptable at 2–8 children.
- Owner child panes can show follow-up input after session end — desired
  behavior, but a product change worth noting.
- The harness/environment selectors have no ownership gate at the widget level,
  so collaborators can click them. However `set_harness`/`set_environment_id`
  only update local state; without follow-up input enabled (ownership-blocked
  via `resolve_cloud_conversation_continuation_ui_state`) there is no path for
  that selection to be acted on. The existing owner child pane path already
  puts the same controls in the same `AgentRunning` state via
  `enter_viewing_existing_session`, so no new exposure is introduced.

**Lines saved in `hydration.rs`:** ~120 (removing ~230 lines across three
attach functions, adding ~80 for the unified function; `new_for_ambient_orchestration_child`
adds ~30 lines in `terminal_manager.rs`).

### 3. Collapse the two materialization arms

With changes 1 and 2, `materialize_owner_child_pane` and
`materialize_viewer_child_pane` become structurally identical:

- `AttachLive` → `attach_ambient_orchestration_child_session` (universal,
  stale-session guard applied unconditionally)
- `LoadTranscript` → `hydrate_child_transcript` (same function). The unified
  function adopts the viewer's `LoadTranscript` behavior: reuse an existing
  registered pane before creating a new loading placeholder, rather than
  always creating a new one as the owner path does.
- `Pending` → register in unified pending map

**Outer dispatch** (`materialize_child_placeholder_pane`, renamed
`materialize_child_pane`) is unchanged in substance — it fetches task data via
`get_or_async_fetch_task_data` and then calls the inner unified arm. The
cached hit is synchronous so this adds no network cost.

**Inner dispatch** is extracted as
`apply_child_pane_materialization(child_conversation, task, ctx)`. This is what
`materialize_viewer_child_pane_from_task` calls directly with its pre-fetched
task, keeping that function as a thin adapter that also provides the
`child_conversation.task_id().or(Some(task.task_id))` fallback.

**Pros:** ~65 lines saved, removes the owner/viewer split at the dispatch level.
**Cons:** None significant.

### 4. Unified pending map

Replace two separate maps and two processing functions:

```rust
// Before
pending_remote_child_hydrations: HashMap<AmbientAgentTaskId, AIConversationId>
pending_viewer_child_hydrations: HashMap<AmbientAgentTaskId, AIConversationId>

// After
pending_child_hydrations: HashMap<AmbientAgentTaskId, AIConversationId>
```

`ChildPaneOrigin` is dropped from the map entirely. After change 2, origin has
no effect on any materialization arm: `AttachLive` uses
`attach_ambient_orchestration_child_session` for all origins, `LoadTranscript`
uses `hydrate_child_transcript` for all origins, and `Pending` re-inserts
`child_id` with no origin metadata needed. One
`process_pending_child_hydrations` function replaces both processors;
the stale-session re-queue from `process_pending_viewer_child_hydrations`
(skip re-attach when the same session_id is still in `failed_viewer_child_sessions`)
is preserved and now applies to all child panes.

**Pros:** ~30 lines saved, conceptually cleaner, no spurious origin tracking.
**Cons:** None.

### 5. Inline loading placeholder wrappers

`create_owner_loading_child_placeholder` and
`create_viewer_loading_child_placeholder` are trivial one-line wrappers around
`create_child_loading_placeholder` with different `AgentViewEntryOrigin`
values. Remove them and call `create_child_loading_placeholder` directly with
`AgentViewEntryOrigin::CloudAgent` throughout — all child panes are cloud agent
children regardless of which side of the family stream discovered them.

**Pros:** ~20 lines saved, removes indirection.
**Cons:** None.

### 6. Drop `TaskOwnership`/`TaskScope`

**Decision: drop in this PR; restore in a follow-on if needed.**

The flag-OFF path used a simple `creator.uid == current_user_uid` check.
`TaskScope`/`TaskOwnership` was introduced to handle team-owned runs where the
creator is a service account UID. `blocks_cloud_followups` returns `true` only
for `GitHubAction` and `GitHubWebhook`; Linear/Slack/CLI runs are not blocked.
The team service account scenario (Linear run where `creator.uid` is a service
account) is real but unconfirmed in production. Drop the complexity now and
restore if a concrete gap surfaces.

**Savings:** ~50 lines (`TaskScope` deserialization, `resolve_ownership`,
`ownership_for_current_principal`, test coverage).

## Summary

| Change | Lines saved | Scope | Priority |
|---|---|---|---|
| Unified transcript function | ~85 | `hydration.rs` | High |
| Universal ambient pane + new constructor | ~120 | `hydration.rs` (−230, +80); +30 in `terminal_manager.rs` | High |
| Dead code deletion (`restoration.rs`, `terminal_pane.rs`) | ~120 | `restoration.rs`, `terminal_pane.rs` | High |
| Collapse materialization arms | ~65 | `hydration.rs` | High |
| Unified pending map (drop `ChildPaneOrigin`) | ~30 | `hydration.rs` | Medium |
| Inline placeholder wrappers (`CloudAgent` origin) | ~20 | `hydration.rs` | Low |
| Drop `TaskOwnership`/`TaskScope` | ~50 | `hydration.rs`, `task.rs` | Approved |
| **Total** | **~490** | | |

Function count in `hydration.rs` (flag-ON): 18 → 11.
Functions removed (10): `materialize_owner_child_pane`,
`materialize_viewer_child_pane`, `attach_child_session`,
`attach_owner_child_session`, `attach_viewer_child_session`,
`hydrate_owner_child_transcript`, `hydrate_viewer_child_transcript_in_place`,
`create_owner_loading_child_placeholder`,
`create_viewer_loading_child_placeholder`,
`process_pending_viewer_child_hydrations`.
Functions added (4): `apply_child_pane_materialization`,
`attach_ambient_orchestration_child_session`, `hydrate_child_transcript`,
`process_pending_child_hydrations`.
Function renamed (1): `materialize_child_placeholder_pane` →
`materialize_child_pane`.
Function simplified (1): `process_pending_remote_child_hydrations` — flag-ON
branch removed, flag-OFF branch retained; function is NOT renamed.
New in `terminal_manager.rs` (1): `new_for_ambient_orchestration_child`.

## Invariants Preserved

- Flag-OFF path is completely untouched; all changes are within
  `OrchestrationUnifiedStack` flag-ON code. `new_for_orchestration_child` in
  `terminal_manager.rs` is unchanged.
- `recover_viewer_child_join_failure` is unchanged in signature and logic; it
  now handles all child panes (owner and viewer) via the universal
  `orchestration_child_conversation_id` set by
  `new_for_ambient_orchestration_child`.
- Follow-up input remains ownership-gated via `cloud_conversation_continuation_ui_state`
  → `owned_ambient_agent_task_id`; collaborators see ambient affordances but
  not the follow-up input.
- Stale-session guard (`failed_viewer_child_sessions`) is preserved and applied
  unconditionally in the unified `AttachLive` arm and
  `process_pending_child_hydrations` for all child origins.
- `materialize_viewer_child_pane_from_task` is preserved as a thin adapter;
  the `task_id` fallback (`child_conversation.task_id().or(Some(task.task_id))`)
  is retained to handle the race where a child conversation has not yet been
  stamped with its task_id.

## Environment

**Branch:** `matthew/orch-unified-m2`
**Directory:** `/Users/matthew/src/child-agent-started-events/warp`
**Commit strategy:** New commits on top of the existing M2 commits. Do not
squash into existing commits. Commit changes 1–5 + dead-code fix as a single
commit, then commit change 6 (`TaskOwnership`/`TaskScope` drop) separately.

**Files to modify:**
- `app/src/terminal/shared_session/viewer/terminal_manager.rs` — add constructor
- `app/src/pane_group/mod.rs` — add helper, update struct fields + initializer
  + `remove_child_agent_panes` + `discard_child_agent_pane_for_conversation`
  + `handle_pending_ambient_restoration_event` caller
- `app/src/pane_group/child_agent/hydration.rs` — primary refactor
- `app/src/pane_group/child_agent/restoration.rs` — update
  `create_hidden_child_agent_pane` call site (Step 6)
- `app/src/pane_group/pane/terminal_pane.rs` — simplify
  `EnsureSharedSessionViewerChildPane` handler (Step 12)
- `app/src/pane_group/child_agent/materialization.rs` — update doc comment
  that references `attach_child_session` (Step 11)
- `app/src/ai/ambient_agents/task.rs` — change 6 only
- `app/src/terminal/view/shared_session/cloud_conversation_continuation.rs`
  — change 6 only (`task_ownership_access` uses `TaskOwnership`)
- `app/src/terminal/view/shared_session/cloud_conversation_continuation_tests.rs`
  — change 6 only
- Any other test files that reference removed functions or renamed fields
  (grep for `pending_viewer_child_hydrations`, `TaskOwnership`, `TaskScope`)

## Implementation

Implement in order. The build should compile after all steps are applied.

### Step 1 — Add `new_for_ambient_orchestration_child` to `terminal_manager.rs`

Model on `new_for_orchestration_child` (search for it in the file). The only
differences from that constructor:
- Pass `is_ambient_agent = true` to `new_internal` (instead of `false`)
- Pass `orchestration_child_conversation_id = Some(conversation_id)` (same as
  `new_for_orchestration_child`)

The `wire_ambient_agent_session_events` call is **not** made here — it is the
caller's responsibility (done in `create_ambient_orchestration_child_pane`).
The `connect_session(session_id, ReplaceFromSessionScrollback, ctx)` call is
retained.

### Step 2 — Add `create_ambient_orchestration_child_pane` to `mod.rs`

Model on `create_orchestration_child_shared_session_viewer` (search for it).
Replace:
```rust
TerminalManager::new_for_orchestration_child(session_id, conversation_id, resources, initial_size, ctx.window_id(), ctx)
```
With:
```rust
TerminalManager::new_for_ambient_orchestration_child(session_id, conversation_id, resources, initial_size, ctx.window_id(), ctx)
```
Then wire the ambient session events. Because `is_ambient_agent=true`, the
ambient model exists at construction time. After adding the pane to the model:
```rust
if let Some(view_model) = terminal_view.as_ref(ctx).ambient_agent_view_model().cloned() {
    crate::terminal::view::ambient_agent::wire_ambient_agent_session_events(&terminal_manager, &view_model, ctx);
}
```

### Step 3 — Add `hydrate_child_transcript` to `hydration.rs`

Model on `hydrate_viewer_child_transcript_in_place` (the more complete version).
It already has both staleness guards and explicit per-variant non-Oz handling.
The function body is nearly identical — the difference from the owner path is:
1. The 6-line active-conversation staleness guard is retained (harmless on owner path)
2. Non-Oz cases (`CLIAgent`, `None`) produce separate warn messages instead of a
   single combined bail-out

The signature is:
```rust
fn hydrate_child_transcript(
    &mut self,
    pane_id: PaneId,
    child_id: AIConversationId,
    task_id: AmbientAgentTaskId,
    server_token: ServerConversationToken,
    ctx: &mut ViewContext<Self>,
)
```

### Step 4 — Add `attach_ambient_orchestration_child_session` to `hydration.rs`

Model on `attach_owner_child_session`. Replace the
`create_shared_session_viewer(session_id, ..., false, true, ctx)` call with
`create_ambient_orchestration_child_pane(session_id, child_id, resources, view_size, ctx)`.
The function takes no `ChildPaneOrigin` parameter. Everything else (anchor
tracking, conversation restore, `enter_agent_view`, `enter_viewing_existing_session`,
`set_live_execution_session`, `child_agent_panes.insert`) is identical to the
owner path.

Signature:
```rust
fn attach_ambient_orchestration_child_session(
    &mut self,
    child_id: AIConversationId,
    session_id: SessionId,
    ctx: &mut ViewContext<Self>,
)
```

### Step 5 — Add `apply_child_pane_materialization` to `hydration.rs`

This is the inner dispatch. Called by both `materialize_child_pane` (step 6)
and `materialize_viewer_child_pane_from_task` (step 7). Signature:
```rust
fn apply_child_pane_materialization(
    &mut self,
    child_conversation: AIConversation,
    task: AmbientAgentTask,
    ctx: &mut ViewContext<Self>,
)
```

Dispatch on `decide_child_pane_materialization(&task)`:

**`AttachLive { session_id }` arm:**
```rust
let child_id = child_conversation.id();
if self.failed_viewer_child_sessions.get(&child_id) == Some(&session_id) {
    // Stale-session guard: same dead session, queue for retry
    if let Some(task_id) = child_conversation.task_id() {
        self.pending_child_hydrations.insert(task_id, child_id);
        self.ensure_pending_ambient_restoration_subscription(ctx);
    }
    if let Some(pane_id) = self.child_agent_panes.get(&child_id).copied()
        && let Some(view) = self.terminal_view_from_pane_id(pane_id, ctx)
    {
        view.update(ctx, |view, ctx| {
            view.set_orchestration_child_live_unavailable(true, ctx);
        });
    }
    return;
}
self.failed_viewer_child_sessions.remove(&child_id);
if let Some(task_id) = child_conversation.task_id() {
    self.pending_child_hydrations.remove(&task_id);
}
self.attach_ambient_orchestration_child_session(child_id, session_id, ctx);
```

**`LoadTranscript { server_token }` arm:**
Reuse an existing registered pane if one is present (model on the viewer path's
pane-reuse logic — check `child_agent_panes.get(&child_id)` first, then create
a new loading placeholder with `create_child_loading_placeholder(child_conversation,
AgentViewEntryOrigin::CloudAgent, ctx)` as fallback). Remove from
`pending_child_hydrations` and `failed_viewer_child_sessions`, then call
`hydrate_child_transcript`.

**`Pending` arm:**
Reuse or create loading placeholder (same pattern). Insert into
`pending_child_hydrations`, call `ensure_pending_ambient_restoration_subscription`.

### Step 6 — Rename `materialize_child_placeholder_pane` → `materialize_child_pane`

The outer dispatch is largely unchanged. After it calls
`get_or_async_fetch_task_data` and has `Some(task)`, it calls
`apply_child_pane_materialization(child_conversation, task, ctx)` instead of
branching on `origin`. The `origin` parameter can be removed from the
signature.

When task data is `None` (not yet cached): show loading placeholder
(`create_child_loading_placeholder(child_conversation, AgentViewEntryOrigin::CloudAgent, ctx)`),
insert `task_id` into `pending_child_hydrations`, call
`ensure_pending_ambient_restoration_subscription`. This replaces the
`Some(ChildPaneMaterialization::Pending) | None` arm in the old owner path.

**Also update `restoration.rs::create_hidden_child_agent_pane`** (around line
172–224). This function computes a `pane_origin: Option<ChildPaneOrigin>` via
a branch on conversation flags, then calls
`self.materialize_child_placeholder_pane(child_conversation, origin, ctx)`.
After this step:
- Rename the call to `self.materialize_child_pane(child_conversation, ctx)`
  (drop the `origin` argument)
- Remove the `pane_origin` computation block above it (now dead)
- Remove any `ChildPaneOrigin` imports from `restoration.rs` that become unused

Do not remove `ensure_shared_session_viewer_child_pane` from `restoration.rs`
— it is still used by the flag-OFF path.

### Step 7 — Update `materialize_viewer_child_pane_from_task`

This function has a subtlety: the caller (OVM) may provide a task whose
`task_id` is not yet stamped on the local conversation object (timing race).
The existing `child_conversation.task_id().or(Some(task.task_id))` fallback
must be preserved. Simplify to:
```rust
// Idempotency guard (same as current — check child_agent_panes)
if let Some(existing_pane_id) = self.child_agent_panes.get(&child_id).copied()
    && self.has_pane_id(existing_pane_id)
{
    return;
}
self.apply_child_pane_materialization(child_conversation, task, ctx);
```
The `task_id` fallback is now unnecessary because `apply_child_pane_materialization`
always uses `task.task_id` directly from the task object.

### Step 8 — Add `process_pending_child_hydrations` to `hydration.rs`

This replaces `process_pending_viewer_child_hydrations` and the flag-ON branch
of `process_pending_remote_child_hydrations`. Guard on
`OrchestrationUnifiedStack` enabled and `pending_child_hydrations` non-empty.

For each task in `pending_child_hydrations` with available task data:
- `AttachLive { session_id }` where `failed_viewer_child_sessions.get(&child_id) == Some(&session_id)` → re-insert (stale; wait for next cycle)
- `AttachLive { session_id }` (fresh) → remove from `failed_viewer_child_sessions`, call `attach_ambient_orchestration_child_session`
- `LoadTranscript { server_token }` → remove from `failed_viewer_child_sessions`, get pane_id from `child_agent_panes`, call `hydrate_child_transcript`
- `Pending` → re-insert into `pending_child_hydrations`

Model on `process_pending_viewer_child_hydrations` which has all these arms.

### Step 9 — Update `PaneGroup` struct and constructor in `mod.rs`

In the struct definition (around line 951):
- Keep `pending_remote_child_hydrations` (flag-OFF path only — do not remove)
- Replace `pending_viewer_child_hydrations: HashMap<AmbientAgentTaskId, AIConversationId>` with `pending_child_hydrations: HashMap<AmbientAgentTaskId, AIConversationId>`

In the constructor (around line 3170):
- Keep `pending_remote_child_hydrations: HashMap::new()`
- Replace `pending_viewer_child_hydrations: HashMap::new()` with `pending_child_hydrations: HashMap::new()`

Update these methods that reference the old field:
- `remove_child_agent_panes` (around line 4637): replace `pending_viewer_child_hydrations` with `pending_child_hydrations`
- `discard_child_agent_pane_for_conversation` (around line 4652): same replacement
- The doc comment on `ensure_pending_ambient_restoration_subscription` (around
  line 3284) doesn’t currently mention `pending_viewer_child_hydrations`, so
  no substitution is needed; extend it to mention `pending_child_hydrations`
  alongside `pending_remote_child_hydrations`.

### Step 10 — Update `handle_pending_ambient_restoration_event` in `mod.rs`

The caller at lines 3314–3316 currently calls both
`process_pending_remote_child_hydrations` and
`process_pending_viewer_child_hydrations`. Replace with:
```rust
self.process_pending_ambient_restorations(ctx);
self.process_pending_remote_child_hydrations(ctx); // flag-OFF path (no-op under flag-ON)
self.process_pending_child_hydrations(ctx);         // flag-ON unified path
```

### Step 11 — Remove deleted functions

Delete from `hydration.rs`:
- `materialize_owner_child_pane`
- `materialize_viewer_child_pane`
- `attach_child_session`
- `attach_owner_child_session`
- `attach_viewer_child_session`
- `hydrate_owner_child_transcript`
- `hydrate_viewer_child_transcript_in_place`
- `create_owner_loading_child_placeholder`
- `create_viewer_loading_child_placeholder`
- `process_pending_viewer_child_hydrations`

Remove the flag-ON branch from `process_pending_remote_child_hydrations`
(the branch that drove owner children via `pending_remote_child_hydrations`).
Keep the flag-OFF branch (`attempt_remote_child_hydration`) and the function
itself unchanged — it is NOT renamed.

Update `recover_viewer_child_join_failure` to use `pending_child_hydrations`
instead of `pending_viewer_child_hydrations`.

Remove the unused `ChildPaneOrigin` import from `hydration.rs` (the type
becomes fully unused after the above deletions).

In `materialization.rs`, find the doc comment on `ChildPaneOrigin` that
references `PaneGroup::attach_child_session` and update it to reflect that
the type is now a hint used only in `create_hidden_child_agent_pane` for
the flag-OFF path.

### Step 12 — Fix `EnsureSharedSessionViewerChildPane` handler in `terminal_pane.rs`

Find the handler for `Event::EnsureSharedSessionViewerChildPane`. It currently
has a flag-ON branch calling `group.attach_child_session(...)`. Since
`attach_child_session` is deleted and this branch is dead code under the unified
stack, simplify the handler to unconditionally call the flag-OFF path:
```rust
Event::EnsureSharedSessionViewerChildPane { conversation_id, session_id } => {
    group.ensure_shared_session_viewer_child_pane(
        *conversation_id,
        *session_id,
        ctx,
    );
}
```
`restoration.rs::ensure_shared_session_viewer_child_pane` is **not** deleted —
it is still used by the flag-OFF path.

### Step 13 — Commit hydration simplification (changes 1–5 + step 12)

Run presubmit checks and fix any issues before committing:
```
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
Commit:
```
git add -A
git commit -m "Simplify hydration: unified child pane materialization path

Replace the parallel owner/viewer materialization arms in hydration.rs
with a single unified path under OrchestrationUnifiedStack:

- Add new_for_ambient_orchestration_child TerminalManager constructor
  (is_ambient_agent=true + orchestration_child_conversation_id=Some)
- Introduce attach_ambient_orchestration_child_session replacing three
  attach functions; all child panes now get FailedToJoin recovery
- Unify hydrate_owner/viewer_child_transcript into hydrate_child_transcript
- Collapse materialize_owner/viewer_child_pane into apply_child_pane_materialization
- Unify pending_viewer_child_hydrations into pending_child_hydrations;
  stale-session guard now applies to all child panes
- Simplify EnsureSharedSessionViewerChildPane handler (dead flag-ON branch)

Flag-OFF path is completely untouched.

Co-Authored-By: Oz <oz-agent@warp.dev>"
```

### Step 14 — Drop `TaskOwnership`/`TaskScope` (separate commit)

In `app/src/ai/ambient_agents/task.rs`:
- Remove `TaskScope` struct/enum and its `Deserialize` impl
- Remove `TaskOwnership` enum
- Remove `resolve_ownership` and `ownership_for_current_principal` from `AmbientAgentTask`
- Remove the `scope: Option<TaskScope>` field from `AmbientAgentTask`

In `terminal/view/shared_session/view_impl.rs` (around line 192):
- Update the `OrchestrationUnifiedStack` branch of `owned_ambient_agent_task_id`
  to use the simple `creator.uid == current_user_uid` check instead of
  `ownership_for_current_principal`

In `terminal/view/shared_session/cloud_conversation_continuation.rs`:
- Find `task_ownership_access` (imports `TaskOwnership`). Update the
  `OrchestrationUnifiedStack` flag-ON branch to use the simple creator-uid
  check, matching what `owned_ambient_agent_task_id` does after the above change.
- Remove the `TaskOwnership` import.

In `cloud_conversation_continuation_tests.rs`:
- Update or remove tests that set `task.scope = Some(TaskScope::User { ... })`
  or `TaskScope::Team { ... }` to grant Edit/ViewOnly access. With the scope
  field removed, these tests need to use the creator-uid mechanism instead.

In `mod_tests.rs` (pane_group tests):
- Find the test (around line 1194-1295) that sets
  `task.scope = Some(TaskScope::User { uid: "other-user" })` specifically to
  grant Edit access despite a non-matching creator uid. This test's premise
  is testing the `TaskScope` path that is being deleted. The test's assertions
  (`Continuation`/edit-access) may no longer hold under the simple creator-uid
  check. Review whether to update assertions or remove the test.

Grep for all usages of `TaskOwnership`, `TaskScope`, `ownership_for_current_principal`,
and `resolve_ownership` to catch every call site before and after changes.

Commit:
```
git commit -m "Drop TaskOwnership/TaskScope from hydration path

Falls back to simple creator.uid == current_user_uid check for the
OrchestrationUnifiedStack flag-ON path. Restore in a follow-on if
team service-account ownership gaps surface in production.

Co-Authored-By: Oz <oz-agent@warp.dev>"
```

### Step 15 — Push

```
git push origin matthew/orch-unified-m2
```

## Validation

1. `./script/presubmit` — must pass (format, clippy, tests)
2. Build and run against a local warp-server with `OrchestrationUnifiedStack`
   enabled
3. Start an orchestration run with 2+ children and verify:
   - Each child pane has ambient controls (environment, harness indicator)
   - Follow-up input appears for owned child panes after session ends
   - Collaborator viewing the shared parent session sees child panes with
     ambient UI but no follow-up input
   - If a child session_id is stale/expired, `FailedToJoin` triggers
     recovery (pane retries on next task update) rather than a toast
4. Verify flag-OFF path: disable `OrchestrationUnifiedStack`, start a
   shared session with children, and confirm the old behavior is unchanged

## Sequencing

New commits on top of `matthew/orch-unified-m2`. No server changes required.
Behavioral changes:
- All child panes gain `FailedToJoin` recovery (owner panes previously showed
  a toast with no retry).
- Owner child panes in viewer contexts gain ambient controls and
  follow-up input after session end — a product improvement.
- Collaborator child panes gain ambient affordances (environment, harness) —
  informational, no new actionable risk (controls are already gated by
  `resolve_cloud_conversation_continuation_ui_state`).
