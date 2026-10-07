*Spec: Restore child agents with TUI conversations*

== PRODUCT ==

*Summary:* Restoring an orchestration conversation in Warp Agent CLI currently restores only the selected parent conversation. Its child-agent conversations remain absent from the retained TUI session registry, so the existing orchestration tab bar cannot display or navigate to them. Restore every locally known, TUI-displayable descendant when the parent is restored through either `--resume` or the conversations menu, while preserving the current GUI and TUI presentation.

*Key design choices:*
- Reuse the existing frontend-neutral history loaders, `AIConversation` metadata methods, topology walker, and block-restoration plan directly. Add only the missing TUI session materializer; do not introduce a new cross-frontend child descriptor/resolver or coordinator.
- Eagerly materialize all supported descendant sessions in the TUI, in topology order, and eagerly restore local Oz transcripts. Remote children use the existing lightweight cloud-run session and persisted run metadata rather than gaining a new transcript UI.
- Match the GUI's locally known restoration scope. Do not synthesize UI children from `AmbientAgentTask.children` when no local child conversation/topology record exists; that list is currently sufficient for event watching, but not for reconstructing conversation identity, transcript, or all presentation metadata.

*Behavior*:
1. Restoring a parent orchestration conversation with `warp --resume <server-token>` restores the parent as the focused session and eagerly creates background TUI sessions for every supported, locally known child and nested descendant.
2. Selecting the same parent from the TUI conversations menu produces the same restored session tree as `--resume`; the entry point changes neither descendant selection nor hydration behavior.
3. Restored descendants appear in the existing orchestration tab bar in the same recursive spawn/status order used by live TUI orchestration. The parent remains focused after restoration, and existing keyboard navigation switches to each restored child session without a new UI affordance.
4. A local Oz descendant is restored into a retained local terminal session with its persisted transcript, name, status, parent linkage, task/run identity, action results, and working directory metadata. Restoration does not send its original prompt again or create a new server task.
5. A remote/cloud descendant is restored into the existing lightweight cloud-run session. Its state starts as restored/spawned—not dispatching—and the existing view shows the authoritative remote task status and run link. Restoration fetches the task status once, then existing orchestration events keep the status current. This applies to task-backed remote children regardless of the remote harness because the TUI cloud view is harness-agnostic.
6. Descendants are restored transitively, not only one level deep. A grandchild remains linked to its actual parent conversation, while all descendants use the root session's window as their TUI host.
7. Restoring the same tree more than once never creates duplicate sessions or duplicate orchestration tabs. Replacing a parent conversation removes retained child-session projections belonging to the replaced tree without cancelling or deleting the underlying cloud tasks.
8. A parent with no children restores exactly as it does today: one focused parent session, no tab bar, and no additional network or session work.
9. A missing/corrupt child record, a remote child without stable task/run identity, a shared-session-viewer child, or a local non-Oz harness that the TUI cannot display does not fail the parent restore. That child is skipped with diagnostic logging; other supported descendants still restore.
10. The GUI's restoration code, timing, hidden-pane ownership checks, shared-session behavior, hydration decisions, focus, and rendered behavior do not change.
11. Child discovery uses the existing history index and loaders. A server-only `--resume` whose client has no child topology records restores only the parent; deriving UI children solely from server `children` run IDs is out of scope.

== TECH ==

*Context:* All references are pinned to `fd16dceb3f2a9e5e106f19010aa964265ac4f02c`.

- `crates/warp_tui/src/session.rs:256-303` creates one focused bootstrap terminal session and forwards a `--resume` token to `TuiTerminalSessionView::restore_conversation`.
- `crates/warp_tui/src/terminal_session_view.rs:2490-2645` loads one target, validates it, clears the current parent surface, restores only `vec![conversation]`, restores that transcript, and selects it. The conversations menu and startup origins converge here, so this is the common TUI restore boundary.
- `crates/warp_tui/src/orchestration_model.rs:160-220` walks all descendants for the tab snapshot, but filters each one through `TuiSessions::session_ids_by_conversation`. The existing test `snapshot_is_shared_across_tree_and_filters_conversations_without_sessions` confirms this condition: topology without retained sessions is intentionally invisible.
- `crates/warp_tui/src/session_registry.rs:139-239` owns full local terminal sessions and lightweight cloud sessions. Live local Oz children retain a terminal manager/view; live remote children retain `TuiCloudRunView` plus `TuiCloudRunState`.
- `crates/warp_tui/src/orchestration_model.rs:539-662` initializes live local and remote child conversations, sends a local child's first prompt, and records `child_session_by_conversation`. Restore must reuse the materialized session shapes but must not reuse launch initialization or resend work.
- `crates/warp_tui/src/cloud_run.rs:12-99` initializes every cloud view as `Dispatching` and only reaches `Spawned` through launch-time `set_spawned`. A restore-specific constructor/state transition is required.
- `app/src/pane_group/child_agent/restoration.rs:20-245` is the GUI prior art. It gets direct child IDs, skips already-owned panes, resolves each child from history or `RestoredAgentConversations`, and then branches into shared-session viewer, task-backed remote, or local hidden-pane materialization.
- `app/src/pane_group/child_agent/hydration.rs:18-310` keeps remote GUI hydration UI-specific (`LiveAttach`, `LoadTranscript`, or `Fallback`). Those pane/view operations do not belong in the TUI or the shared core.
- `app/src/pane_group/pane/terminal_pane.rs:322-352` subscribes every attached terminal pane to fullscreen `EnteredAgentView`. Because hidden child panes are attached before their restored conversation enters agent view (`app/src/pane_group/mod.rs:4118-4160`), the GUI's direct-child helper is re-entered for each materialized child and therefore restores locally known nested descendants recursively.
- `app/src/ai/blocklist/history_model/conversation_loader.rs:526-620` eagerly indexes parent-to-child topology and hydrates orchestration children into `conversations_by_id` during local startup. Other historical conversations stay lazy.
- `app/src/ai/blocklist/orchestration_topology.rs:142-181` already provides the recursive spawn-order walker used by TUI navigation and lifecycle operations.
- `app/src/ai/blocklist/history_model/conversation_loader.rs:235-310` loads one server-token conversation. It does not reconstruct child UI conversations. `app/src/ai/blocklist/orchestration_event_streamer.rs:1348-1558` consumes `AmbientAgentTask.children` to restore event-watching run IDs, not UI sessions.
- `ServerApiProvider::get_ai_client` exposes the existing `get_ambient_agent_task` request used to read authoritative task state. Remote TUI restoration can issue that request once after session materialization and guard its completion with the retained conversation/session/task mapping, without adding lifecycle state to local persistence or persistent restoration bookkeeping.

*Root cause and reproduction:* The defect is a missing TUI materialization step, not missing tab rendering. Both restore entry points insert only the requested parent into one surface. The orchestration snapshot correctly omits descendants that have no retained session. A real end-to-end reproduction could not be run in the spec environment because no seeded parent token with local and cloud children was available. The equivalent code-level reproduction was run with:

`cargo nextest run --manifest-path /workspace/warp/Cargo.toml -p warp_tui snapshot_is_shared_across_tree_and_filters_conversations_without_sessions`

It passed (`1 passed, 757 skipped`) and proves the failing precondition: a known child without a session is filtered out of the TUI snapshot. The implementation regression test below must first demonstrate that current parent restore leaves such children unmaterialized, then pass after the fix.

*Design alternatives*:

- *Alternative A — reuse the existing shared primitives directly; add only a TUI materializer (selected).*
  - The TUI reads recursive descendant IDs with `descendant_conversation_ids_in_spawn_order`, obtains each `AIConversation` from the already-hydrated history model or the existing `load_conversation_data` API, and classifies it with existing methods such as `is_remote_child`, `is_viewing_shared_session`, `task_id`, and orchestration-harness accessors.
  - The GUI remains unchanged. Its direct-child loop continues to resolve from history / `RestoredAgentConversations`, enforce pane ownership, and invoke the existing hidden-pane materializers.
  - Pros: uses the same kind of data-oriented shared seam as base conversation restoration, introduces no abstraction that has only two callers, minimizes GUI risk, and keeps the implementation focused on the missing TUI lifecycle.
  - Cons: the GUI's history/store fallback and the TUI's history/loader lookup remain expressed at their call sites. The small amount of repeated control flow is deliberate because their availability and ownership semantics differ.

- *Alternative B — add a frontend-neutral child descriptor/resolver (rejected as unnecessary for this change).*
  - Introduce a type owning a resolved `AIConversation`, parent ID, and semantic local/remote/shared-viewer kind, plus a resolver that combines history and `RestoredAgentConversations`.
  - GUI and TUI would both consume that type before entering their own materializers.
  - Pros: gives child lookup/classification a named contract and centralizes the history/store precedence.
  - Cons: it mostly wraps fields and methods already present on `AIConversation`; `RestoredAgentConversations` has GUI take-once semantics that the TUI does not need; and it would require changing/exporting `app` APIs plus touching the GUI to remove only a few lines of lookup. It over-formalizes data that the existing shared loader seam already exposes.
  - Decision: do not add this type or resolver unless implementation uncovers a concrete invariant that cannot be expressed with the existing APIs.

- *Alternative C — shared restoration coordinator with GUI/TUI adapter callbacks (rejected).*
  - Add a coordinator in `app` that owns traversal, deduplication, async child hydration, error aggregation, and lifecycle ordering. Define an adapter trait/callback set such as `has_materialized_child`, `materialize_local`, `materialize_remote`, `materialize_shared_viewer`, and `on_failure`; call it from both `PaneGroup` and `TuiOrchestrationModel`.
  - The GUI adapter would wrap `child_agent_panes`, pane ownership checks, off-tree attachment, fullscreen entry, task-fetch subscriptions, and transcript/live-attach fallback.
  - The TUI adapter would wrap `TuiSessions`, terminal managers, cloud-run state, focus, event consumers, and tab refreshes.
  - Pros: one place could own traversal and deduplication, and future restoration policy changes would have a single coordinator.
  - Cons: the apparent commonality ends after discovery. GUI restore is lazy per fullscreen pane and includes hidden/off-tree/shared-session semantics; TUI restore is eager, recursive, and creates heterogeneous retained sessions. A coordinator would either expose UI lifecycle details in a large trait or hide important behavior in callbacks, making ordering and failure handling harder to reason about. It would also force a GUI control-flow rewrite despite the explicit no-GUI-behavior-change constraint.
  - Decision: reject this alternative for APP-5038. Existing shared primitives should continue to describe/load data, not orchestrate UI objects.

- *Eager sessions versus lazy sessions.*
  - Eager session materialization is selected because the existing TUI tab snapshot intentionally requires a session before showing a child. Lazy sessions would require changing tab/navigability semantics, adding loading/error tabs, and splitting identity from session availability—a TUI UI behavior change.
  - Eager local transcript restoration is selected because current TUI conversation restore already constructs the complete block-restoration plan before rendering. Historical child rows are already eagerly hydrated specifically for orchestration.
  - Remote children are eagerly represented by lightweight cloud sessions, but no new remote transcript renderer is added. Stable persisted task/run identity creates the session immediately; a one-time task request then reconciles lifecycle status from the authoritative remote task, and existing orchestration events keep it current.

- *Direct children versus recursive descendants.*
  - Recursive pre-order restoration is selected. It matches the GUI's effective recursive behavior, matches the TUI snapshot and kill-descendants prior art, and avoids a visible but non-navigable grandchild. A single TUI traversal is preferred over relying on child-view re-entry callbacks because it is deterministic and does not duplicate parent restore logic.

- *Server task-child discovery versus local topology parity.*
  - Local topology parity is selected. `AmbientAgentTask.children` contains direct run IDs and is authoritative for event filtering, but cannot by itself restore the local `AIConversationId`, transcript, parent-conversation identity across all records, or shared/local presentation kind. Synthesizing UI records would exceed GUI prior art and requires a separate product decision.

*Proposed changes:*

1. *Reuse the existing child data seams without changing the GUI.*
   - Keep `PaneGroup::restore_missing_child_agent_panes_for_parent` and `create_hidden_child_agent_pane` unchanged.
   - In the TUI restore path, get descendant IDs from `descendant_conversation_ids_in_spawn_order`.
   - Resolve an ID by cloning `BlocklistAIHistoryModel::conversation` when already hydrated. If an indexed child is not in memory, use `BlocklistAIHistoryModel::load_conversation_data`; do not add a second persistence resolver.
   - Classify the returned `AIConversation` with its existing metadata methods. Treat a local child with no explicit harness as legacy Oz; skip explicit local non-Oz and shared-session-viewer records because the TUI has no matching retained view.
   - Do not add a new `app` module, public export, descriptor type, or GUI call-site refactor.

2. *Add a restore-only TUI orchestration materialization API.*
   - After `replace_conversation_surface` restores and selects the parent, call a new `TuiOrchestrationModel` restore entry point with the parent conversation ID and root `TuiSessionId`. Both `TuiConversationRestoreOrigin::Startup` and `ConversationList` must use this same call.
   - Walk descendants in recursive spawn order and resolve every child conversation before/while materializing. Use the root session's window for all new child sessions; preserve each conversation's actual parent ID in history.
   - Before creating a child, consult both `TuiSessions::session_ids_by_conversation` and `child_session_by_conversation`. Existing live materializations are reused, not duplicated.
   - Register every restored supported child in `child_session_by_conversation`. Register event consumers using the same ownership rules as live children: local child sessions may consume their own stream; remote placeholders remain passive, while the root parent remains responsible for descendant status watching.
   - A child failure is isolated: log the child ID and reason, continue the traversal, and finish the parent restore in `Idle`.

3. *Materialize restored local Oz children without launching them.*
   - Add a `TuiSessions` path that creates an unfocused local terminal session and passes its view/manager back to the orchestration restore API.
   - Add a restore-only `TuiTerminalSessionView` method for a fresh child surface. It must reuse `prepare_conversation_block_restoration`, action-result restoration, `BlocklistAIHistoryModel::restore_conversations`, transcript restoration, active-conversation selection, and exit-summary refresh.
   - It must not call `prepare_local_oz_child_launch`, `start_new_child_conversation`, `record_new_conversation_request_complete`, `initialize_orchestrated_child_conversation`, or `start_orchestrated_child`.
   - Restore the persisted working-directory metadata as the terminal session's startup directory where available. Do not attempt to restart a terminated local process.

4. *Materialize restored remote/cloud children in the current cloud view.*
   - Add `TuiCloudRunState::new_restored` (or an equivalent explicit constructor) requiring conversation ID, task ID, run ID, and run URL. It starts in `Spawned`; it must never render “Starting cloud run…” for a restored child.
   - Add a restore-only `TuiSessions` cloud-session constructor that accepts the restored state, creates an unfocused `TuiCloudRunView`, restores the child conversation onto that surface, marks it active there, and registers the conversation/session mapping.
   - Derive the current Oz run URL with the existing `oz_run_url` helper. Issue one `get_ambient_agent_task` request after materialization, map its `AgentRunDisplayStatus` to `ConversationStatus`, and update the restored placeholder through `BlocklistAIHistoryModel`; subsequent orchestration events continue using the existing live status path.
   - After applying a fetched status change, emit a restoration-specific `TuiOrchestrationEvent`. Only agent blocks that render received-agent messages subscribe; a block whose sender matches the restored child calls `ctx.notify()` so its cached glyph redraws without invalidating transcript layout or subscribing every block to general history updates.
   - The request completion must verify that the conversation still maps to the same retained session, remains a remote child, and retains the same task identity. A late completion for a replaced tree becomes a no-op; the model does not retain a separate set of restored conversations.
   - If stable task/run identity is missing or invalid, skip the remote child. Do not invent a new task or leave a permanently dispatching placeholder.

5. *Clean up replaced tree projections and async races.*
   - Before replacing an already selected parent, collect that parent's materialized descendant session IDs. Remove those retained session projections and unregister their event consumers without calling `kill_child_agent` or the cloud cancellation API.
   - Guard any async/background enrichment with the existing restore request ID or with a root/child mapping check, so a stale completion cannot attach a child to a later restored parent.
   - If the parent restore is repeated, the session lookup guards make the operation idempotent. If a different parent replaces it, stale child sessions from the prior tree do not remain visible.

*Affected files (expected):*
- `crates/warp_tui/src/terminal_session_view.rs`
- `crates/warp_tui/src/terminal_session_view_tests.rs`
- `crates/warp_tui/src/orchestration_model.rs`
- `crates/warp_tui/src/orchestration_model_tests.rs`
- `crates/warp_tui/src/session_registry.rs`
- `crates/warp_tui/src/session_registry_tests.rs`
- `crates/warp_tui/src/cloud_run.rs`
- `crates/warp_tui/src/cloud_run_view_tests.rs`

*Open questions resolved:*
- *Which descendants?* All locally known descendants, recursively. This matches the GUI's effective recursion and the existing TUI topology model.
- *When are tabs/sessions created?* Eagerly during parent restore. Lazy session creation would require a new tab loading contract and is outside the no-UI-change scope.
- *When is heavy state hydrated?* Local Oz transcripts are eager. Remote children eagerly create only the lightweight cloud session from persisted identity, then reconcile status with one remote task request; the current TUI does not display their transcript.
- *Which child kinds?* Local Oz (including legacy unspecified-harness local rows) and task-backed cloud children are supported. Explicit local non-Oz and shared-session-viewer children are skipped because the TUI has no corresponding retained view.
- *What is shared?* The existing history loaders/index, `AIConversation` metadata, topology walker, and block-restoration plan. No new child-specific shared abstraction is required; UI lifecycle orchestration stays per frontend.
- *Do server-only run IDs create tabs?* No. The existing GUI does not reconstruct UI children solely from `AmbientAgentTask.children`, and those IDs are not a complete conversation restoration record.
- *Does restoration restart children?* No. It restores navigable history/status projections only and never relaunches prompts, local processes, or cloud tasks.
- *Does switching parents kill children?* No. It removes stale TUI projections and event-consumer registrations but does not invoke child cancellation/deletion.
- *What remains out of scope?* New TUI visuals, new remote transcript UI, local third-party harness support, shared-session viewer support, server-side child reconstruction, changes to GUI behavior, and changing how live child agents are launched.

*Risks / blast radius:*
- The selected design intentionally leaves GUI code untouched, reducing GUI blast radius. Existing GUI restore tests remain a no-regression gate.
- Session cleanup can accidentally cancel work if it uses live kill paths. The restore path must remove projections only and explicitly test that no cancel/delete method is invoked.
- Restoring full local transcripts in several background terminal sessions can increase startup memory/latency for large trees. The eager decision is required by current tab semantics; record timings in manual verification and keep remote children lightweight.
- Legacy or malformed records may lack harness/task/run metadata. Fail per child, retain the parent, and verify supported siblings still restore.
- Recursive data may contain cycles. Reuse the existing topology walker only after adding/retaining cycle protection or validate that the history index cannot recurse indefinitely; a malformed-cycle test must fail closed.
- Restore races can orphan sessions when users select another conversation before background work completes. Request/root mapping checks must gate late updates.
- Remote task status hydration can fail while offline. The lightweight session and run link remain available, and later orchestration events can still correct the placeholder status; restoration does not fail the parent or write remote lifecycle state into local persistence.

*Validation & verification criteria* (must ALL pass before merge):

1. *Code-level bug reproduction and primary regression* — Add a TUI integration-style unit test, e.g. `restoring_parent_materializes_supported_descendant_sessions`, that seeds a parent plus local Oz child, cloud child, and nested grandchild in history/restored storage, invokes the common parent restore boundary, and asserts:
   - before the implementation the existing snapshot filters the children because no sessions exist;
   - after the implementation the session count is parent plus all supported descendants;
   - the snapshot lists the descendants in recursive pill order and tab navigation resolves each session.
   Run with `cargo nextest run -p warp_tui restoring_parent_materializes_supported_descendant_sessions`. Verifies behavior 1, 3, 4, 5, and 6.

2. *Both restore entry points* — Parameterize the parent restore regression over `TuiConversationRestoreOrigin::Startup` with a server-token target and `TuiConversationRestoreOrigin::ConversationList` with a local target. Assert identical descendant IDs/kinds/order, parent focus, and final `ConversationRestoreState::Idle`. Run the named test(s) with `cargo nextest run -p warp_tui <test-filter>`. Verifies behavior 1 and 2.

3. *Local Oz hydration* — Add a test that restores a local child with at least one exchange, action result, agent name, task/run ID, status, parent ID, and working directory. Assert the child has a full terminal session, the transcript renders the exchange, restored action results exist, metadata is preserved, and no launch/start-agent executor receives a request. Run with `cargo nextest run -p warp_tui restored_local_oz_child_hydrates_transcript_without_relaunch`. Verifies behavior 4.

4. *Remote/cloud hydration* — Add tests for at least one remote Oz child and one remote non-Oz harness child. Assert both use `TuiSessionView::Cloud`, begin in `TuiCloudRunStartup::Spawned`, retain conversation/task/run IDs, reconcile the authoritative terminal task status through the one-time task request, render that status and run link, and never display “Starting cloud run…”. Include a regression where the restored local placeholder begins `InProgress` while the fetched remote task is terminal. Run with `cargo nextest run -p warp_tui restored_remote_child`. Verifies behavior 5.

5. *Nested descendants* — Seed parent → child → grandchild with mixed local/cloud kinds. Assert all supported descendants get sessions, keep their immediate parent linkage, appear in recursive spawn order, and remain navigable from parent and child snapshots. Run with `cargo nextest run -p warp_tui restored_nested_descendants`. Verifies behavior 6.

6. *Idempotency and replacement cleanup* — Restore the same tree twice and assert one session per conversation and no duplicate tabs/consumers. Then restore a different parent and assert prior descendant session projections/consumers are removed, underlying history records remain persisted, and no cloud cancel/delete request occurs. Include a stale-completion case that switches parents before enrichment completes. Run with `cargo nextest run -p warp_tui restore_tree_is_idempotent restore_replacement_discards_stale_child_sessions`. Verifies behavior 7.

7. *No-child no-op* — Restore a parent with no child index. Assert the session count stays one, focus stays on the parent, no cloud/task fetch is triggered by child restoration, and the tab snapshot remains `None`. Run with `cargo nextest run -p warp_tui restoring_parent_without_children_is_noop`. Verifies behavior 8.

8. *Unsupported and malformed children degrade independently* — Seed supported siblings around a missing child record, explicit local non-Oz child, shared-session-viewer child, remote child without task/run identity, and a malformed topology cycle. Assert the parent and supported siblings restore, unsupported records create no session, no infinite traversal occurs, and diagnostics identify each skipped child. Run with `cargo nextest run -p warp_tui restore_skips_unsupported_or_malformed_children`. Verifies behavior 9.

9. *Existing shared-seam coverage* — Add/extend TUI tests proving an already-hydrated child and an indexed-but-not-loaded child both flow through the existing history APIs into the same materializer, while classification preserves parent identity and does not create a session for unsupported kinds. Run with `cargo nextest run -p warp_tui restored_child_history_loading`. Supports behavior 4-11.

10. *GUI no-regression* — Run the existing GUI tests for lazy topology/pane restoration and remote child hydration, including `test_pane_group_restore_loop_keeps_orchestration_topology_and_materializes_child_pane`, the restored remote-child tests in `pane_group::mod_tests`, and `hydrate_remote_child_placeholder_with_cloud_transcript_preserves_placeholder_identity`. No GUI production or test changes are expected. Run with `cargo nextest run -p warp pane_group_restore child_agent_hydration hydrate_remote_child_placeholder`. Verifies behavior 10.

11. *Server-only scope guard* — Test a parent loaded by server token with no local child topology while task metadata exposes `children` run IDs. Assert event-watching state may record those run IDs through existing streamer behavior, but the TUI creates no synthetic child conversation/session. Run with `cargo nextest run -p warp_tui server_only_resume_does_not_synthesize_child_sessions` and the relevant `orchestration_event_streamer` restore tests. Verifies behavior 11.

12. *Existing TUI orchestration regressions* — Run `cargo nextest run -p warp_tui orchestration_model session_registry cloud_run terminal_session_view` and confirm live local/cloud launch, tab navigation, nested kill, `/new`, focus, and cloud status/link tests still pass.

13. *Repository checks* — From the repository root, run:
   - `./script/format --check`
   - `./script/check_no_inline_test_modules`
   - `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings`
   - `cargo clippy -p warp --all-targets --tests -- -D warnings`
   - `cargo clippy -p warp_completer --all-targets --tests -- -D warnings`
   - `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`
   - `cargo nextest run -p warp_completer --features v2`
   - `cargo test --doc`
   These are the documented L/cross-cutting presubmit gates (`./script/presubmit` may be used to run the combined sequence).

14. *Running TUI visual verification* — Build successfully, then run `./script/run-tui` against a seeded account/database containing a parent with a local Oz child, cloud child, and nested descendant. Using computer use:
   - restore once with `--resume`, capture a screenshot showing the existing tab bar with all descendants and the parent focused;
   - navigate to the local child and capture its restored transcript;
   - navigate to the cloud child and capture its restored status/run link without a dispatching state;
   - repeat restoration from the conversations menu and capture the equivalent restored tab tree;
   - capture a parent-only restoration showing no tab bar.
   Attach the screenshots to the implementation PR. Verifies behavior 1-9 without introducing new UI.
