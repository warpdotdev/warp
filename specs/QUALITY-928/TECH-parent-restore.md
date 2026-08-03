# Cloud Agent Parent Conversation Restore

Supplemental spec for QUALITY-928. Describes the fix for the pill bar and child
panes disappearing after a client restart following a completed cloud agent run
with remote children.

## Environment

**Worktree:** `/Users/matthew/src/child-agent-started-events/warp`
**Branch:** `matthew/orch-unified-m2` (add commits on top; do not squash into
the existing M2 commit)
**Flag:** `OrchestrationUnifiedStack` — all new code is gated behind it

## Problem

After restarting the client, a restored cloud agent parent pane shows its
transcript correctly but the orchestration pill bar is empty and child panes are
gone. Pill-bar clicks and keyboard orchestration navigation are dead.

## Root Cause

### RC1: Parent conversation is never persisted

When the client joins a cloud agent's shared session the parent conversation is
flagged `is_viewing_shared_session = true`. `write_updated_conversation_state`
(app/src/ai/agent/conversation.rs:3476) early-returns immediately for any
`is_viewing_shared_session` conversation, so the parent is never written to the
`agent_conversations` SQLite table. The pane snapshot stores only `task_id`.

This flag exists to prevent third-party viewers from persisting the host's
conversation. For a `/cloud-agent` run the user IS the owner of the run; the
flag is applied correctly in form (they joined via shared-session protocol) but
incorrectly in effect (they should persist as owner).

### RC2: Parent conversation ID changes on every restart

Because the parent is not persisted, `get_or_set_canonical_conversation_id_for_server_token`
(app/src/ai/blocklist/history_model.rs:2655) finds no local row on restore and
mints a fresh `AIConversationId`. The child conversations ARE persisted and store
`parent_conversation_id = OLD_ID`, but the pill bar reads
`children_by_parent[NEW_ID]` and finds nothing. The children's
`parent_agent_id` field (= parent's server `run_id`) is stable across restarts
but is not used by the startup re-indexing pass.

### Why the live-session path works

When the parent run is still in progress at restart time (`InProgress` +
`is_sandbox_running` + valid `session_id`), restore joins the live shared
session. `NetworkEvent::JoinedSuccessfully` constructs an
`OrchestrationViewerModel`, which issues an ancestor REST seed that
re-discovers all children and fires `ChildSpawned` for each. This path never
runs for completed runs.

## Fix 1: Persist owned cloud agent conversations

**One function change in `write_updated_conversation_state`.**

Conditionalize the `is_viewing_shared_session` early-return: skip persistence
only when the conversation is NOT owned by the current user.

### Implementation

#### 1. Add imports to `app/src/ai/agent/conversation.rs`

Two singletons are needed that are not yet imported:

```rust
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::auth::AuthStateProvider;
```

`FeatureFlag` and `AmbientAgentTaskId` are already imported.

#### 2. Change `write_updated_conversation_state` (around line 3476)

Current code:

```rust
pub(crate) fn write_updated_conversation_state(
    &mut self,
    ctx: &mut ModelContext<BlocklistAIHistoryModel>,
) {
    // We should not persist non-local conversations (e.g. shared sessions).
    if self.is_viewing_shared_session {
        return;
    }
    ...
```

After the change:

```rust
pub(crate) fn write_updated_conversation_state(
    &mut self,
    ctx: &mut ModelContext<BlocklistAIHistoryModel>,
) {
    if self.is_viewing_shared_session && !self.is_owned_cloud_agent_conversation(ctx) {
        return;
    }
    ...
```

#### 3. Add `is_owned_cloud_agent_conversation` helper

Add immediately above `write_updated_conversation_state`:

```rust
/// Returns true when this conversation is the owner's view of their own
/// cloud agent run. Such conversations are persisted despite
/// `is_viewing_shared_session` being set: the client joins via the
/// shared-session protocol but the run belongs to the current user, so
/// the local conversation should survive restarts.
///
/// Returns false when the task data is not yet cached — the caller
/// retries on the next update, which arrives after the cache is warm.
fn is_owned_cloud_agent_conversation(
    &self,
    ctx: &ModelContext<BlocklistAIHistoryModel>,
) -> bool {
    if !FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
        return false;
    }
    let Some(task_id) = self.task_id else {
        return false;
    };
    let Some(current_uid) = AuthStateProvider::as_ref(ctx)
        .get()
        .user_id()
        .map(|uid| uid.as_string())
    else {
        return false;
    };
    AgentConversationsModel::as_ref(ctx)
        .get_task_data(&task_id)
        .is_some_and(|task| {
            task.creator
                .as_ref()
                .is_some_and(|c| c.uid == current_uid)
        })
}
```

### Timing note

`write_updated_conversation_state` is called whenever conversation state
changes. Early calls (before the task is in the ACM cache) will get
`get_task_data = None` → `is_owned_cloud_agent_conversation = false` →
conversation not persisted on that call. This is safe: the task is fetched
before the session is joined and before any content arrives, so by the time
exchanges are being streamed the task is already cached. Any intermediate
update that is not persisted is superseded by the next write.

### `is_viewing_shared_session` remains set — that is correct

**No other code path is broken by this change.** A thorough audit of all
usages confirms each gated behavior is correct for the cloud agent owner:

- **Navigation exclusion** (`should_exclude_from_navigation`, conversation.rs:1422):
  cloud agent conversations should not appear in the regular conversation list;
  they are shown via the pane. Correct.
- **Timing derivation** (conversation.rs:2334, 2465, 2532, 2554): exchange
  start times and time-to-first-token are derived from server message
  timestamps for shared-session viewers. Correct for the owner — the agent
  runs on the server.
- **Subtask/message reconstruction** (conversation.rs:2701, 2723, 3036):
  user inputs are reconstructed from server messages because the original
  prompt was not sent locally. Correct for the owner.
- **Tool call cleanup skip** (conversation.rs:2931): temp dirs from search
  subagents are not cleaned up for viewers. Correct — the cloud run is
  server-side.
- **OVM restore skip** (orchestration_event_streamer.rs:1918):
  `on_restored_conversations` skips `is_viewing_shared_session` conversations.
  Acceptable: for completed runs the pill bar is rebuilt from the DB index
  (which Fix 1 makes correct) and from the `task.children` seed added by
  Fix 2; for live runs the OVM is created at `JoinedSuccessfully` and is
  unaffected.

### Companion regression fix in the loading-pane path

Persisting the parent has a side effect on restore: an owned cloud agent
conversation that now exists locally resolves through
`AgentConversationsModel::resolve_open_action` to
`WorkspaceAction::RestoreOrNavigateToConversation` (a local-conversation
navigation action) instead of `OpenConversationTranscriptViewer`. That action
cannot be applied to an ambient loading pane, so the pane fell through to the
`_` arm and was replaced with an empty new cloud conversation.

`app/src/pane_group/ambient_pane_restoration.rs` handles this by extracting
`restore_pane_with_transcript` (ambient_pane_restoration.rs:167) and adding a
`RestoreOrNavigateToConversation` arm (ambient_pane_restoration.rs:138) that
loads the transcript from the server token carried on the task
(`task.conversation_id()`), falling back to a new cloud conversation only when
the task has no conversation id.

## Fix 2: Seed children from `task.children` at restore time

### Why Fix 1 is not sufficient on its own

Fix 1 works only where a durable, per-user SQLite database survives across
sessions and already contains a row for the parent. Two paths do not satisfy
that:

1. **The WASM web client.** It shares this Rust codebase, but `app/build.rs`
   (add_features, app/build.rs:224) only sets the `local_fs` / `local_tty`
   cargo features when `target_family != "wasm"`, and `crates/persistence`
   gates its SQLite implementation behind `local_fs`
   (crates/persistence/src/lib.rs:4). There is no cross-session
   `agent_conversations` table on web, so
   `get_or_set_canonical_conversation_id_for_server_token` can never find a
   prior row and `initialize_historical_conversations` has nothing to index.
   For an Oz session link opened on web, seeding from `task.children` is the
   *only* mechanism that can populate the pill bar.

This case is solved by rebuilding the parent→child relationship from server
data (`AmbientAgentTask.children`, task.rs:190 — the `Vec<String>` of direct
child `run_id`s added in M2) instead of from local DB state.

### Where the seeding runs

`load_data_into_restored_ambient_cloud_mode_view` (pane_group/mod.rs:5384) is
the single funnel for restoring a cloud agent parent into a cloud-mode pane.
Both entry points reach it:

- Native session restore: `replace_loading_pane_with_restored_ambient_cloud_mode_pane_inner`
  (mod.rs:5357).
- Web / deep-link transcript load: `load_data_into_transcript_viewer`
  (mod.rs:3722).

At that point the following are in scope:

- `task_id: AmbientAgentTaskId` — the parent run id (parameter).
- The just-minted parent `AIConversationId`, already computed in the local
  `conversation_id` variable inside the `CloudConversationData::Oz` arm
  (mod.rs:5424).
- A fetch for the parent task is already kicked off at the top of the function
  (mod.rs:5398), so `task.children` is usually one `TasksUpdated` away.

The function is an associated fn (no `&mut self`) and runs inside an active
`PaneGroup` update, so it cannot re-enter the view to touch `PaneGroup` state.
**Change it to return `Option<AIConversationId>`** (the value it already
computes) and have each of the two `&mut self` call sites invoke the new
seeding entry point immediately afterwards. Rejected alternative: taking a
weak handle and calling `update` from inside the associated fn — that is a
re-entrant view update.

### New state and entry point

Add one field to `PaneGroup` (mod.rs, near `pending_child_hydrations`,
mod.rs:957) and initialize it in `new_internal` (mod.rs:3172):

```rust
/// Restored cloud agent parents whose `task.children` have not yet been
/// fully materialized as local child conversations, keyed by the parent's
/// run id. Re-driven from the shared `TasksUpdated` subscription until
/// every child in the server-reported list has a local conversation.
pending_parent_child_seeds: HashMap<AmbientAgentTaskId, AIConversationId>,
```

Drain it from the existing shared subscription by adding a call to
`process_pending_parent_child_seeds(ctx)` in
`handle_pending_ambient_restoration_event` (mod.rs:3302), alongside the
existing `process_pending_*` calls.

Implement both functions in `app/src/pane_group/child_agent/restoration.rs`,
which already owns the `impl PaneGroup` child-restoration helpers.

```rust
/// Rebuilds the parent→child conversation index for a restored cloud agent
/// parent from the server-reported `task.children` list. This is the only
/// pill-bar source on clients without cross-session SQLite (web) and on the
/// first restore of a run whose parent was never persisted.
pub(in crate::pane_group) fn seed_child_conversations_from_task(
    &mut self,
    parent_conversation_id: AIConversationId,
    parent_task_id: AmbientAgentTaskId,
    ctx: &mut ViewContext<Self>,
) {
    if !FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
        return;
    }
    let task = AgentConversationsModel::handle(ctx)
        .update(ctx, |model, ctx| {
            model.get_or_async_fetch_task_data(&parent_task_id, ctx)
        });
    let Some(task) = task else {
        // Fetch in flight: re-drive on the next `TasksUpdated`.
        self.pending_parent_child_seeds
            .insert(parent_task_id, parent_conversation_id);
        self.ensure_pending_ambient_restoration_subscription(ctx);
        return;
    };
    // Older servers don't populate `children`; fall back to whatever the
    // local index already holds (Fix 1).
    if task.children.is_empty() {
        self.pending_parent_child_seeds.remove(&parent_task_id);
        return;
    }
    ...
}
```

### Per-child work

For each `child_run_id` in `task.children`:

1. `child_run_id.parse::<AmbientAgentTaskId>()`; `log::warn!` and skip a
   malformed id (mirrors `ensure_remote_child_placeholder`,
   orchestration_event_streamer.rs:709).
2. Fetch the child's task data:
   `AgentConversationsModel::get_or_async_fetch_task_data(&child_task_id, ctx)`.
   `None` means the fetch is in flight — leave the parent entry in
   `pending_parent_child_seeds` and continue to the next child. The ACM's
   per-task in-flight dedupe and failure cooldowns
   (agent_conversations_model.rs:1806) bound the retry rate, so re-driving the
   whole list on each `TasksUpdated` is safe.
3. Resolve the parent's terminal surface:
   `BlocklistAIHistoryModel::terminal_surface_id_for_conversation(&parent_conversation_id)`.
   Warn and skip if absent (same guard as
   `finish_remote_child_placeholder`, orchestration_event_streamer.rs:762).
4. Create or look up the child placeholder:

```rust
let child_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
    history.ensure_remote_child_conversation(
        terminal_surface_id,
        parent_conversation_id,
        child_run_id.clone(),
        child_task.task_id,
        child_task.display_name().to_string(),
        child_task.title.trim().to_string(),
        agent_task_harness(&child_task),
        ctx,
    )
});
```

`ensure_remote_child_conversation` (history_model.rs:576) is idempotent: it
returns the existing conversation when `conversation_id_for_agent_id(run_id)`
already resolves, so re-running the seed (or racing the SSE family drain,
which calls the same function) creates nothing extra. On the create path it
goes through `start_new_child_conversation` →
`set_parent_for_conversation`, which both stamps `parent_conversation_id` and
inserts into `children_by_parent` — exactly what the pill bar reads
(`descendant_conversation_ids_in_spawn_order`, orchestration_topology.rs:160).

`agent_task_harness` (orchestration_event_streamer.rs:2763) is currently a
private free fn; widen it to `pub(crate)` and import it rather than
duplicating the derivation.

### Parent-link reconciliation

No reconciliation step is needed. Fix 1 and Fix 2 ship together before any
users have run an orchestration session under `OrchestrationUnifiedStack`.
Every run will have had Fix 1 active since it was created, so the parent
always keeps the same `AIConversationId` across restarts and the persisted
children's `parent_conversation_id` already matches.

`ensure_remote_child_conversation` (history_model.rs:576) is idempotent: if
the child already exists under the correct parent the call is a no-op.

### Completion and pane materialization

After the loop:

- Remove the `pending_parent_child_seeds` entry once every id in
  `task.children` resolves through `conversation_id_for_agent_id`; otherwise
  leave it for the next `TasksUpdated`.
- Materialize hidden child panes if the parent's pane is resolvable:
  `self.pane_id_for_owned_conversation(parent_conversation_id, ctx)` →
  `restore_missing_child_agent_panes_for_parent(...)` (restoration.rs:29),
  which skips children that already have a live pane. Pills themselves do not
  require panes — `ensure_hidden_child_agent_pane_for_conversation`
  (restoration.rs:107) materializes lazily on click — so a missing parent pane
  is not an error; just `ctx.notify()` so the pill bar re-renders.

`process_pending_parent_child_seeds` simply re-invokes
`seed_child_conversations_from_task` for each `(parent_task_id,
parent_conversation_id)` snapshot, which is idempotent by construction.

### Graceful degradation

- `task.children` empty (older server, or a run with no children): no-op; the
  pill bar shows whatever the local index holds, i.e. current behavior.
- Parent or child task fetch fails: ACM records the failure and applies its
  cooldown; the pending entry survives and retries on later `TasksUpdated`
  events. No pane is replaced or destroyed on this path.
- `OrchestrationUnifiedStack` disabled: the seeding entry point returns
  immediately.

## What changes at restore time

With the parent conversation persisted (Fix 1):

1. `get_or_set_canonical_conversation_id_for_server_token` finds the existing
   row and returns the **same** `AIConversationId` as the previous session.
2. The child conversations' persisted `parent_conversation_id` still matches.
3. `initialize_historical_conversations`
   (app/src/ai/blocklist/history_model/conversation_loader.rs:571) rebuilds
   `children_by_parent[stable_parent_id] = [child_ids]` automatically.
4. The pill bar reads the populated index and renders the child pills. ✓
5. `restore_missing_child_agent_panes_for_parent` (restoration.rs) finds the
   children and drives child pane hydration, which fetches each child's task
   data and updates the pill badge status through the normal
   `process_pending_child_hydrations` / `hydrate_child_transcript` path.

Independently, seeding from `task.children` (Fix 2) fires on the same restore
and converges on the same `children_by_parent` state from server data. That
makes the pill bar work in the case where steps 1–3 cannot succeed:

- The **web client**, which has no SQLite at all, so steps 1–3 are unavailable.

The two fixes are order-independent and idempotent: whichever populates the
index first, the other one's work collapses to a no-op.

## Files to Change

| File | Change |
|---|---|
| `app/src/ai/agent/conversation.rs` | Fix 1: add 2 imports; add `is_owned_cloud_agent_conversation` helper; change early-return condition in `write_updated_conversation_state` |
| `app/src/pane_group/ambient_pane_restoration.rs` | Fix 1 companion: extract `restore_pane_with_transcript`; add the `RestoreOrNavigateToConversation` arm so an owned, now-persisted cloud run still restores its transcript instead of an empty pane |
| `app/src/pane_group/mod.rs` | Fix 2: return `Option<AIConversationId>` from `load_data_into_restored_ambient_cloud_mode_view`; call the seeding entry point from both call sites; add the `pending_parent_child_seeds` field and drain it from `handle_pending_ambient_restoration_event` |
| `app/src/pane_group/child_agent/restoration.rs` | Fix 2: add `seed_child_conversations_from_task` and `process_pending_parent_child_seeds` |
| `app/src/ai/blocklist/orchestration_event_streamer.rs` | Fix 2: widen `agent_task_harness` to `pub(crate)` for reuse |

No server changes are required: `AmbientAgentTask.children` is already
populated by `GET /agent/runs/{run_id}` and already consumed by
`apply_task_children` (orchestration_event_streamer.rs:2084).

## Validation

1. Start a cloud agent via `/cloud-agent`. Ask it to call `run_agents` with
   2+ remote children. Wait for all agents to complete.
2. Quit and restart Warp.
3. Verify the parent pane restores as a transcript view.
4. Verify the pill bar shows one pill per child with the correct final status
   badge (success/error/cancelled).
5. Click each pill — verify it reveals the child pane with its transcript.
6. Quit and restart Warp a second time — verify the pills still appear
   (confirming the parent was written to the DB on first restore, not just
   synthesized from the live session).
7. Test via the Oz web session link for the same completed run — open the
   run's session link in the browser client and verify the pill bar appears
   and the children are shown. This exercises Fix 2 in isolation, since the
   web build has no SQLite (`local_fs` is off for wasm) and therefore no
   stable-parent-id path.
8. Test the live-session path: start a cloud agent, restart before it
   finishes. Verify the pane rejoins the live session and pills appear
   (this path was already working; confirm no regression).
9. Verify no duplicate pills: a run whose children are both persisted locally
    and reported in `task.children` must render exactly one pill per child.
10. Disable `OrchestrationUnifiedStack`: restart after a completed run —
    verify the old behavior (no pills) is unchanged.
11. Run `./script/presubmit` — must pass cleanly.

## Sequencing

New commits on top of `matthew/orch-unified-m2`. No server changes required.

Fix 1 has landed on the branch:

- `b388b3ae8` — persist owned cloud agent conversations
  (`app/src/ai/agent/conversation.rs`).
- `b26b8aaaf` — loading-pane restore regression fix
  (`app/src/pane_group/ambient_pane_restoration.rs`).

Fix 2 is not implemented yet and can land as a single follow-up commit; it is
independent of Fix 1 and does not modify it.
