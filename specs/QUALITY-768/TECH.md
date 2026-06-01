# QUALITY-768: Restore orchestration session state

## Context

Restarting Warp left orchestration sessions in four distinct broken states. Local-local orchestration came back without the pill bar, and `send_message_to_agent` / `messages_received` rows rendered the child as `"Unknown agent"`. Local-remote orchestration restored the placeholder child pane as the empty `"New agent conversation"` shell instead of the cloud transcript that was actually streaming server-side. A meaningful share (~50% in some traces) of local-no-harness Oz child conversations were silently dropped at read time entirely. And a cloud-parent orchestration session (a shared-session viewer pane locally) survived the first restart correctly but lost its cloud-mode shape on the next snapshot — the second restart re-materialised the parent as a plain local terminal with the cloud session's env-setup blocks replayed and no orchestration UI. Underneath all four symptoms, the on-disk eviction policy was splitting orchestration trees across restarts, so even a healthy read path would re-encounter partial state on the next boot.

The subsystem-level explanation of restoration ↔ orchestration lives in the architecture report at `/Users/matthew/src/orch-restore/restoration-and-orchestration.md`. The PR body summarises the user-visible behaviour. This spec is the implementation-level account of the four code-level decisions that ship in this PR; an earlier read-time optimistic-stub filter was superseded by upstream PR #11814 and dropped during the rebase — see change 3 below.

### Relevant code

- `crates/persistence/src/schema.rs:11` — `agent_conversations` table.
- `crates/persistence/src/schema.rs:20` — `agent_tasks` table.
- `crates/persistence/src/model.rs (935-1006)` — `AgentConversation::is_restorable` (the multi-root acceptance rules introduced by upstream PR #11814 cover the optimistic-stub case).
- `app/src/persistence/agent.rs (38-233)` — disk-side prune entry point and `select_conversations_to_evict`.
- `app/src/ai/restored_conversations.rs:17` — `RestoredAgentConversations`, the consume-once startup store.
- `app/src/ai/blocklist/history_model.rs:55-68` — `MAX_HISTORICAL_CONVERSATIONS`.
- `app/src/ai/blocklist/history_model/conversation_loader.rs:64` — `convert_persisted_conversation_to_ai_conversation_with_metadata`, the single conversion entry point used by both startup consumers.
- `app/src/ai/blocklist/history_model/conversation_loader.rs:472` — `initialize_historical_conversations`.
- `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs:624` — `OrchestrationPillBar::pill_specs`, reads `conversations_by_id`.
- `app/src/ai/blocklist/block/view_impl/orchestration.rs:87` — `participant_for_agent_id`, reads `conversations_by_id`.
- `app/src/pane_group/mod.rs:927` — `child_agent_panes` index (`HashMap<AIConversationId, PaneId>`).
- `app/src/pane_group/mod.rs (1078-1151)` — `PendingRemoteChildHydration` struct, `RemoteChildHydrationAction` enum, `decide_remote_child_hydration_action`.
- `app/src/pane_group/mod.rs:3899` — `hydrate_task_backed_hidden_child_pane`.
- `app/src/pane_group/mod.rs:4007` — `attempt_remote_child_hydration`.
- `app/src/pane_group/mod.rs:4114` — `hydrate_remote_child_transcript_in_place`.
- `app/src/pane_group/mod.rs:4228` — `attach_ambient_session_and_maybe_tombstone`.
- `app/src/ai/blocklist/history_model.rs:2353` — `merge_cloud_tasks_into_existing_conversation`.
- `app/src/pane_group/mod.rs` — `create_shared_session_viewer` (the `is_cloud_mode` parameter routes restored parent panes through the cloud-mode constructor).
- `app/src/pane_group/pane/terminal_pane.rs` — `TerminalPane::snapshot`, whose shared-session-viewer branch chooses between `LeafContents::AmbientAgent { task_id }` and a fallback empty `LeafContents::Terminal` based on `view.ambient_agent_view_model()`.
- `app/src/terminal/shared_session/viewer/terminal_manager.rs` — `viewer::TerminalManager::new` (non-deferred constructor; `is_cloud_mode` is plumbed through `new_internal`).
- `app/src/terminal/view.rs` — `TerminalView::new`, where `ambient_agent_view_model` is only constructed when `is_cloud_mode == true`.

## Proposed changes

The implementation now breaks into four self-contained changes that share the read/write path described in the architecture report. A fifth change in the original scope (a read-time optimistic-stub filter in `crates/persistence/src/model.rs`) has been superseded by upstream PR #11814 "Fix optimistic-root persistence breaking conversation restore (QUALITY-774)" and is no longer carried by this PR; the rationale lives below in change 3.

### 1. Eager orchestration-child hydration in `initialize_historical_conversations`

`initialize_historical_conversations` (`app/src/ai/blocklist/history_model/conversation_loader.rs:472`) previously only indexed orchestration children in `children_by_parent` and `agent_id_to_conversation_id`; insertion into `conversations_by_id` was deferred until the parent's hidden child pane materialised. After the change, the loader detects orchestration children via `resolved_parent_conversation_id_from_persisted_data` and eagerly inserts the fully-deserialised `AIConversation` into `conversations_by_id` (`conversation_loader.rs:558-573`), reusing the existing `convert_persisted_conversation_to_ai_conversation_with_metadata` conversion. No `RestoredConversations` event is emitted, no `live_conversation_ids_for_terminal_view` entry is registered, and no AI blocks are constructed. The later `restore_conversations` call from lazy pane materialisation overwrites the eager entry idempotently.

Tradeoff. The plan considered emitting a `RestoredConversations` event from the eager path so downstream consumers could subscribe symmetrically. Rejected: it would force every `RestoredConversations` subscriber to handle the "no terminal view" case, and the surfaces that actually need the child (pill bar, name resolver) already read from `conversations_by_id` directly. The eager insert is also gated to orchestration children only; non-orchestration historical conversations stay on the lazy path.

### 2. Direct `AmbientAgentTask` inspection for remote-child transcript hydration

The plan originally routed restored remote-child hydration through `AgentConversationsModel::resolve_open_action`. Once `conversations_by_id` carries the placeholder eagerly (change 1), the resolver returns `RestoreOrNavigateToConversation`, a variant that collapses "navigate to the local conversation" and "hydrate the cloud transcript onto the local placeholder" into a single outcome. Widening the resolver would force every navigation site to handle a new variant; the remote-child path is the only site that wants the cloud-transcript outcome.

Instead the hidden-pane hydration inspects the `AmbientAgentTask` directly. The dispatch is extracted into a pure function:

```rust path=null start=null
enum RemoteChildHydrationAction {
    LiveAttach,
    LoadTranscript { server_token: ServerConversationToken, task_is_terminal: bool },
    Fallback { task_is_terminal: bool },
}

fn decide_remote_child_hydration_action(task: &AmbientAgentTask) -> RemoteChildHydrationAction { ... }
```

`RemoteChildHydrationAction` and `decide_remote_child_hydration_action` live in `app/src/pane_group/mod.rs`. The function filters empty/whitespace `conversation_id` tokens via `task.conversation_id().map(str::trim).filter(|t| !t.is_empty())` before treating the value as a usable `LoadTranscript` target, and computes `task_is_terminal = matches!(live_session_state, Inactive)` once so both `LoadTranscript` and `Fallback` carry the same gate.

`hydrate_task_backed_hidden_child_pane` creates the hidden pane up front, registers it under the placeholder's local id in `child_agent_panes`, and either calls `attempt_remote_child_hydration` synchronously when task data is cached, or installs an entry in the named `pending_remote_child_hydrations: HashMap<AmbientAgentTaskId, PendingRemoteChildHydration>` map and retries when `AgentConversationsModelEvent::TasksUpdated` fires. An inner idempotency guard returns whenever a live tracked pane already exists for the placeholder, so a second mid-hydration call from `restore_missing_child_agent_panes_for_parent` — even before the initial transcript merge has populated any exchanges — cannot insert a duplicate hidden pane and orphan the first one. The `LoadTranscript` branch routes through `hydrate_remote_child_transcript_in_place`, which fetches the cloud transcript via `BlocklistAIHistoryModel::load_conversation_by_server_token` and merges it onto the placeholder via `BlocklistAIHistoryModel::merge_cloud_tasks_into_existing_conversation` (`app/src/ai/blocklist/history_model.rs`). The merge preserves the placeholder's local `AIConversationId`, parent linkage, agent name, run id, and `is_remote_child` flag, and returns `anyhow::Error` if the placeholder has been evicted from `conversations_by_id`. The caller already handles that error path by falling back to the live-attach + tombstone branch.

The post-match step is centralised in `attach_ambient_session_and_maybe_tombstone`, which calls `apply_existing_ambient_task_to_pane` and then inserts the conversation-ended tombstone iff `task_is_terminal == true`. All four arms (`LoadTranscript` Ok merge, `LoadTranscript` Err merge, `LoadTranscript` non-Oz / fetch-failure fallback, and `Fallback`) route through this helper so the gate stays uniform — an `ActiveUnattachable` task whose transcript fetch errors out, returns a non-Oz payload, or never had a server token in the first place no longer gets a misleading "conversation ended" tombstone.

The async continuation guard inside `ctx.spawn` requires both `child_agent_panes[child_id] == pane_id` and the pane's terminal view's `active_conversation_id == Some(child_id)` before mutating UI state, so a racing nav or competing hydration cannot clobber a stale target.

Tradeoff. The plan considered widening `resolve_open_action` to expose a `HydrateRemoteChildPlaceholder` variant. Rejected for the reason above. The plan also considered keeping the dispatch inline inside `attempt_remote_child_hydration`; extracted into a free function so the decision (Attachable, ActiveUnattachable + token, Inactive + token, Inactive + no token, ActiveUnattachable + no token, plus the empty-token-filter case) is unit-testable without standing up a `PaneGroup`.

### 3. Optimistic-stub handling (superseded by upstream PR #11814)

Local-no-harness Oz children sometimes persisted two root-shaped tasks: a 38-byte zero-payload optimistic stub created at child-spawn time, and a real upgraded root carrying the actual messages. `AgentConversation::is_restorable` saw two root-shaped tasks and rejected the row, silently dropping the conversation at the entry to `initialize_historical_conversations`.

Upstream PR #11814 ("Fix optimistic-root persistence breaking conversation restore", QUALITY-774) addresses the same symptom on two coordinated layers: `Task::source_for_persistence` returns `None` for `Optimistic(Root)` so no new stub rows are written, and `AIConversation::new_restored` deduplicates multi-root payloads by preferring the parentless task with non-empty `messages`. `AgentConversation::is_restorable` was relaxed to accept the multi-root [stub + real] shape so legacy DB rows still load through the restore path. The local-DB conversion entry point `convert_persisted_conversation_to_ai_conversation_with_metadata` now calls `AIConversation::new_restored_synthesizing_on_empty`, which synthesizes a fresh optimistic root when the persisted task list is empty.

This PR's earlier read-time filter (`task_is_root_shaped`, `optimistic_stub_task_id`, `tasks_for_restore`, `into_tasks_for_restore`) and the seven unit tests pinning it have been removed during the rebase onto upstream. Upstream's restore-side dedupe (with a 50-iteration HashMap-order regression test in `app/src/ai/agent/conversation_tests.rs`) plus its relaxed `is_restorable` cover the same ground at the same layer with less surface area, so a parallel filter is redundant.

### 4. Tree-aware persisted-conversation prune

`select_conversations_to_evict` (`app/src/persistence/agent.rs:143`) replaces the previous per-row FIFO LRU prune in `upsert_agent_conversation` (`agent.rs:60-118`). Each persisted row is grouped into its orchestration tree by walking `parent_conversation_id` to a root (parse failures are treated as their own root; orphan references where the declared parent is missing from the row set are likewise treated as roots). Trees are sorted freshest-first by `max(member.last_modified_at)` with ties broken by `root_id` ascending. The greedy keep loop always retains the freshest tree intact — even if it alone exceeds `MAX_PERSISTED_CONVERSATION_COUNT` — and then keeps each subsequent tree atomically while the cumulative kept count is within the cap. Hard-stop semantics: once any tree exceeds the budget, every older tree is also evicted.

The retention cap moved from 100 to 200 (`MAX_PERSISTED_CONVERSATION_COUNT`, `app/src/persistence/agent.rs:49`). The mirrored read-side cap `MAX_HISTORICAL_CONVERSATIONS` (`app/src/ai/blocklist/history_model.rs:68`) was bumped to the same value with an inline comment noting that the read-side cap is currently moot because the disk-side prune keeps the persisted set inside the same window.

The iteration uses an `iter.next()` pattern to unconditionally keep the freshest tree, then a `for` loop over the remainder, avoiding the `first: bool` flag that would otherwise sit inside the loop body.

Tradeoffs.

- **Freshest-tree exception.** The plan considered a strict cap that evicts even from the freshest tree. Rejected: a strict cap could split an active orchestration session in half on disk, regressing into the same "broken half-tree" failure mode that motivated this change. The unbounded freshest-tree case is documented as a known limitation below.
- **Sharing the constant.** Two `const usize` values in two files is a soft drift hazard, but `crates/persistence` is upstream of `warp` in the workspace graph and cannot import from it. The reverse import would pull persistence-only code into the read-side path. Documented in `MAX_HISTORICAL_CONVERSATIONS`'s comment that the read cap is moot only as long as it stays ≥ the disk cap.
- **Parse failure handling.** Rows whose `conversation_data` fails JSON parsing are treated as their own root rather than being silently linked into another tree. The disk row is untouched and the eviction algorithm just refuses to chain a malformed row into a tree.

### 5. Cloud-mode shared-session viewer snapshot/restore loop

Changes 1–4 fix restoration for orchestration *children*. A separate latent bug affected the *parent* of an orchestration tree when the parent itself was a cloud agent: its local pane is a shared-session viewer attached to the cloud session, and the restoration → snapshot → restoration loop lost the cloud-mode shape on the second cycle.

`restore_pane_leaf`'s `LeafContents::AmbientAgent → AmbientRestoreKind::SharedSession` arm called `create_shared_session_viewer(session_id, …)`, which used `shared_session::viewer::TerminalManager::new(…)`. The non-deferred constructor hardcoded `is_cloud_mode = false` when delegating to `new_internal`, so the resulting `TerminalView` had no `ambient_agent_view_model` (`TerminalView::new` only constructs the model when `is_cloud_mode == true`). The pill bar and transcript still rendered on restart 1 because the `JoinedSuccessfully` viewer-network handler spins up an `OrchestrationViewerModel` regardless of `is_cloud_mode`. But the snapshot path in `app/src/pane_group/pane/terminal_pane.rs` checks `view.ambient_agent_view_model()` to decide between `LeafContents::AmbientAgent { task_id }` and the fallback empty `LeafContents::Terminal`. With `ambient_agent_view_model = None`, every shutdown after the first restoration emitted the empty `Terminal` shape and the `task_id` was lost. On restart 2 the pane re-materialised as a fresh local terminal pane sharing the original UUID, replayed the cloud session's env-setup blocks from the persisted block list, and surfaced a local shell prompt with no orchestration UI or pill bar.

The fix threads an explicit `is_cloud_mode: bool` parameter through `create_shared_session_viewer` → `viewer::TerminalManager::new` → `new_internal`. The two ambient-agent restoration call sites (`restore_pane_leaf`'s `SharedSession` arm and `process_pending_ambient_restorations`'s `OpenOrAttachAmbientAgentConversation` arm) now pass `is_cloud_mode: true`, so the restored view has an `ambient_agent_view_model`, the existing `JoinedSuccessfully` handler's `enter_viewing_existing_session(task_id)` path fires, and the next snapshot correctly emits `LeafContents::AmbientAgent` with the `task_id`. The two other callers stay `false`: `ensure_shared_session_viewer_child_pane` (per-child viewer; hidden child panes are excluded from the snapshot tree by design) and `new_for_shared_session_viewer` (navigation-to-shared-session entry point — has the same latent issue but is outside this PR's scope; see Follow-ups).

Tradeoff. The plan considered a snapshot-only fallback that would read the `task_id` from the live `OrchestrationViewerModel` when `ambient_agent_view_model` is absent. Rejected: it would leave the runtime shape mismatch in place. A freshly created cloud-mode pane has an `ambient_agent_view_model`; a restored one would still not, and any future feature reading from `ambient_agent_view_model` on a viewer pane would silently misbehave only after a restart. Routing the restoration through the cloud-mode constructor makes the restored pane's shape match the freshly created shape.

## End-to-end flow

Tracing one boot through the four changes (with upstream PR #11814's stub handling sitting upstream of them) makes the causal chain visible. The diagram below covers changes 1–4; change 5 is orthogonal (it concerns the parent pane's snapshot shape rather than the child-restore data flow).

```mermaid
flowchart TD
    A[SQLite agent_conversations + agent_tasks] -->|read_agent_conversations| B[Vec<AgentConversation>]
    B -->|new_restored_synthesizing_on_empty<br/>+ multi-root dedupe<br/>(upstream PR #11814)| C[restored AIConversation]
    C --> D[RestoredAgentConversations]
    C --> E[initialize_historical_conversations]
    E -->|orchestration children| F[(conversations_by_id)]
    F --> G[OrchestrationPillBar::pill_specs]
    F --> H[participant_for_agent_id]
    F --> I[hydrate_task_backed_hidden_child_pane]
    I -->|decide_remote_child_hydration_action| J{Action}
    J --> K[LiveAttach: apply_existing_ambient_task_to_pane]
    J --> L[LoadTranscript: merge_cloud_tasks_into_existing_conversation]
    J --> M[Fallback: attach in place]
    L --> N[attach_ambient_session_and_maybe_tombstone]
    K --> N
    M --> N
    A -->|upsert_agent_conversation| O[select_conversations_to_evict]
    O -.tree-aware.- A
```

Upstream PR #11814 sits upstream of every other change: it determines which rows survive restore and therefore which rows enter `RestoredAgentConversations` and `initialize_historical_conversations`. The eager-child hydration is what surfaces children into `conversations_by_id` early enough for the pill bar / name resolver to see them and what creates the resolver-collision case that motivates direct `AmbientAgentTask` inspection in the hidden-pane hydration path. The tree-aware prune feeds back into the next boot: if the prune splits trees, the read path's invariants are violated regardless of how well the in-memory side is wired up.

## Testing and validation

Unit coverage lives alongside each module:

- Tree-aware eviction (`app/src/persistence/agent.rs (348-566)`, eight cases): `prune_is_no_op_when_under_limit`, `keeps_fresh_tree_atomically_and_evicts_older_singletons`, `child_kept_drags_parent_along`, `parent_kept_drags_child_along`, `orphan_with_missing_parent_is_its_own_tree`, `single_tree_larger_than_limit_is_kept_in_full`, `parse_failure_row_is_treated_as_root_and_can_be_referenced_by_others`, `eviction_is_deterministic`.
- Stub / multi-root restore coverage now lives in upstream PR #11814's tests: `is_restorable_*` in `crates/persistence/src/model.rs` (five cases covering single-root, multi-root with one real root, multi-root with multiple real roots, multi-root with no real root, empty/single-task) and the dedupe loop in `test_new_restored_prefers_parentless_task_with_messages_over_empty_stub` (`app/src/ai/agent/conversation_tests.rs`).
- Eager orchestration-child hydration (`app/src/ai/blocklist/history_model_tests.rs`): `test_initialize_historical_conversations_eagerly_hydrates_orchestration_children` (line 332), plus `test_initialize_historical_conversations_resolves_parent_agent_id_children_via_seeded_run_ids` (line 258) covers the parent-resolution side path. The eager-hydration test asserts the child is in `conversations_by_id`, that the parent is NOT loaded eagerly, that child run-ids resolve via `conversation_id_for_agent_id`, and that child metadata is excluded from navigation.
- Remote-child hydration dispatch (`app/src/pane_group/mod_tests.rs`): `decide_remote_child_hydration_action` covered with six cases — `LiveAttach` for `Attachable`; `LoadTranscript` for `Inactive` + token (`task_is_terminal: true`); `LoadTranscript` for `ActiveUnattachable` + token (`task_is_terminal: false`); `Fallback` for `Inactive` + no token (`task_is_terminal: true`); `Fallback` for `ActiveUnattachable` + no token (`task_is_terminal: false`, asserting the in-progress run is not visually marked ended); and `decide_remote_child_hydration_empty_token_falls_back` for the empty/whitespace filter added during review.
- LoadTranscript → merge integration coverage (`app/src/ai/blocklist/history_model_tests.rs`): `merge_cloud_tasks_into_existing_conversation_preserves_placeholder_identity`. Builds a placeholder remote-child conversation with `parent_conversation_id` + `agent_name` + `run_id` + `is_remote_child`, drives `merge_cloud_tasks_into_existing_conversation` with a cloud transcript carrying a non-empty title and one user-query exchange, and asserts the merged conversation retains the placeholder's local id and orchestration linkage while surfacing the cloud transcript content. A second assertion exercises the precondition guard by calling merge against an unknown placeholder and asserting `Err`.
- Cloud-mode viewer snapshot contract (`app/src/pane_group/mod_tests.rs`): `create_shared_session_viewer_with_cloud_mode_populates_ambient_agent_view_model` asserts the restored view's `ambient_agent_view_model().is_some()` so the snapshot path's `LeafContents::AmbientAgent` branch is reachable on the next shutdown. A companion test, `create_shared_session_viewer_without_cloud_mode_does_not_populate_ambient_agent_view_model`, pins the non-cloud-mode behaviour so a future flip of the default would be loud.

Manual validation matrix (covers each change end-to-end on a restart):

1. Local-local restart: pill bar renders with the correct agent names (no "Unknown agent" fallback); the multi-root stub shape is silently healed by upstream restore dedupe; conversation list UI is intact.
2. Tree-aware prune: orchestration trees stay together on disk across the cap.
3. Local-remote restart: the cloud transcript merges onto the local placeholder; the "New agent conversation" placeholder bug is gone; live runs continue to attach.
4. Cloud-parent restart-restart: a remote orchestration parent restores correctly across two consecutive Warp restarts; no fallback to a stray local terminal with replayed env-setup blocks.
5. Baseline single-conversation restore: no regressions.

Validation commands: `cargo fmt -p warp -p persistence`, `cargo clippy -p warp --all-targets --features local_fs -- -D warnings`, `cargo clippy -p persistence --tests --all-features -- -D warnings`, and `cargo nextest run` over `pane_group::`, `persistence::agent::tests`, `ai::blocklist::history_model::tests`, and `persistence::model::tests`.

Deferred coverage:

- A property test that asserts `select_conversations_to_evict` never produces an evict-list that splits an orchestration tree (currently inferred from the case tests).
- An end-to-end PaneGroup restart test that exercises `hydrate_remote_child_transcript_in_place` against a mock cloud fetch (the existing test covers the smaller seam directly).

## Risks and mitigations

- **Unbounded freshest tree.** `select_conversations_to_evict` always retains the freshest tree intact, so a single very large orchestration session can push the on-disk row count above 200. Mitigation: the next session's prune evicts everything older, so the steady-state row count stays within the cap unless every session spawns a hundred-plus children. If this becomes a real problem, the freshest-tree exception can be replaced with an "evict the oldest tree members first" within-tree policy, but that re-introduces the half-tree failure mode and is deliberately deferred.
- **Soft drift between `MAX_PERSISTED_CONVERSATION_COUNT` and `MAX_HISTORICAL_CONVERSATIONS`.** The two constants live in different files and crates. They happen to agree today, and the read-side cap's comment documents the invariant: the read cap is moot only as long as the disk cap is `≤` it. Mitigation: the comment, the symmetry of the test coverage, and a future single-const refactor (called out under Follow-ups).
- **Eager-hydration sort-window asymmetry.** `initialize_historical_conversations` walks rows sorted freshest-first by `last_modified_at`, capped at `MAX_HISTORICAL_CONVERSATIONS`. In principle a fresh child could land inside the window without its (stale) parent. Mitigation: tree-aware disk eviction keeps parents fresh enough that this case hasn't been observed; the parent-resolution path tolerates parents that aren't in `conversations_by_id` by falling back to `children_by_parent`.
- **Shared `AgentConversationsModel` subscription for two pending maps.** `ensure_pending_ambient_restoration_subscription` (`app/src/pane_group/mod.rs:3725`) drives both `pending_ambient_agent_conversation_restorations` and `pending_remote_child_hydrations`. Mitigation: maps are kept distinct so the visible-tree `replace_pane` flow does not accidentally swap a hidden child pane; the cost is small and bounded by the number of restored ambient panes.

## Follow-ups

- Share the retention constant between `crates/persistence` and `warp` once the read cap might plausibly be raised independently (for example, if a future history view wants to surface more than the disk cap can hold). Today the read cap is moot, so a separate constant is acceptable; tomorrow it may need to be a single source of truth.
- Add a property test that asserts `select_conversations_to_evict` never returns an evict-list that splits an orchestration tree across the kept/evicted boundary. The existing case tests cover the substantive shapes; a property test would shore them up under generated inputs.
- Tighten `decide_remote_child_hydration_action` against `AmbientAgentLiveSessionState` variants added in future schema bumps. The function currently matches `Attachable` and `Inactive` explicitly; new variants fall into the `LoadTranscript` / `Fallback` decision based on whether the task has a server token. A `cfg`-gated exhaustiveness assertion would catch a new variant at compile time.
- `new_for_shared_session_viewer` has the same latent snapshot/restore loop as the ambient-agent restoration paths fixed by §5. It's a separate entry point used when the user opens a shared session from the conversation list, not the orchestration restore flow, so it's deferred. The shape of the fix is identical: pass `is_cloud_mode: true` from that call site once we confirm the cloud-mode behaviour is desired for non-restore opens.
