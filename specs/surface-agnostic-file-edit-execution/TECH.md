# Surface-Agnostic File-Edit Execution TECH

## Context

This branch builds on the TUI agent tool-calling work (now on `master`), which renders `RequestFileEdits` in the transcript like any other tool call but leaves it unexecuted on non-GUI surfaces: `RequestFileEditsExecutor::execute` required a registered GUI `CodeDiffView` and returned `NotReady` otherwise, blocking the conversation (`app/src/ai/blocklist/action_model/execute/request_file_edits.rs`).

On master the GUI view owns the whole persistence flow: `CodeDiffView::accept_and_save` drives per-file editor-buffer saves through `InlineDiffView`, tracks completion in `SavingDiffs`, and assembles the `RequestFileEditsResult` in `try_emit_diffs_saved`, which the executor consumes via a `SavedAcceptedDiffs` event subscription. That flow is well proven, but it is expressed entirely in terms of one concrete GUI view, so no other surface can execute file edits.

`RequestFileEdits` has a two-phase lifecycle: `preprocess_action` resolves the LLM's edits into concrete diffs (async file reads via `ApplyDiffModel::apply_diffs`) and `execute` runs later to persist them. State computed in preprocess must survive an arbitrary user-interaction gap; the review surface's buffers are that survivor.

## Goal

Make file-edit tool calls executable on any surface (GUI, and the up-stack TUI) by keeping master's surface-owned persistence flow but expressing its shared parts as a trait, so every surface runs the same save-completion and result-assembly code. Every surface must register its diff storage with the executor before the action's diffs resolve — the same contract master had with the GUI view — so the executor's flow stays exactly master's, just behind a surface-agnostic interface.

## Flow

Old flow (master) — executor is bound to the concrete GUI view, and result assembly is split between the view (event payload) and the executor (subscription handler):

```text
output complete ──> AIBlock creates CodeDiffView
                    └─ register_requested_edits(ViewHandle<CodeDiffView>)
preprocess ──> on_diffs_applied ──> diff_views[id].set_candidate_diffs()

User Accept ──> TryAccept ──> executor::execute(id)
  ├─ subscribe_to_view(diff_view)  ◄──────────────────┐ SavedAcceptedDiffs /
  └─ diff_view.accept_and_save()                     │ Rejected events
        │  state = Accepted(Some(SavingDiffs))       │
        ▼                                            │
     per file: InlineDiffView save + diff compute    │
     (tracked inside CodeDiffState)                  │
        │                                            │
     all done => emit SavedAcceptedDiffs{diff,       │
     updated_files, file_contents, ...} ─────────────┘
        │
        ▼
executor's event handler assembles
RequestFileEditsResult + telemetry ──> LLM
```

New flow — identical shape, but the executor talks to the surface-agnostic interface, and persistence + result assembly live entirely in `diff_storage.rs`:

```text
output complete ──> surface creates its review storage
                    └─ register_requested_edits(Box<dyn RegisteredDiffStorage>)
preprocess ──> on_diffs_applied ──> diff_storages[id].set_candidate_diffs()

User Accept ──> TryAccept ──> executor::execute(id)
  └─ diff_storages[id].accept_and_save()      [RegisteredDiffStorage]
        │  (handle upgrade/update)
        ▼
     DiffStorageHelper::accept_and_save    [shared blanket impl]
       ├─ save_state.begin(count) ──> returns result future ──┐
       └─ start_saving()          [surface-specific:          │
            GUI: editor buffers / InlineDiffView;              │
            TUI (up-stack): FileModel write dispatch]          │
        │                                                     │
     callbacks: handle_file_saved /                           │
                handle_diff_computed  [DiffStorageHelper]     │
        │                                                     │
     all complete ──> assemble_result(progress,               │
                      pending_file_state()) ──> send ────────┘
        │
        ▼
ActionExecution::new_async(future) ──> telemetry ──> LLM
```

## Design

### The `DiffStorage` / `DiffStorageHelper` traits (`app/src/ai/blocklist/diff_storage.rs`)

The surface contract and the shared flow follow the `AIBlockModel` / `AIBlockModelHelper` convention (`app/src/ai/blocklist/block/model/helper.rs`): a required-methods-only trait that surfaces implement, and a `Helper` trait whose methods are defined once in a blanket impl so implementations cannot errantly override them:

- `DiffStorage` — implemented by every surface that stores pending diffs. All methods required: state accessors — the fields live on each impl, since traits cannot hold state — plus the surface-specific write kickoff: `save_state_mut` (the in-flight accept's [`DiffSaveState`]), `pending_diff_count`, `pending_file_state` (per-file report state: reported paths, changed lines, final contents, user-edit flags), and `start_saving` (the write kickoff — the hook `accept_and_save` invokes, never called by callers directly).
- `DiffStorageHelper` — the shared flow, blanket-implemented for every `DiffStorage`: `accept_and_save` (the sole entry point: begins tracking, calls `start_saving`, returns a `BoxFuture<RequestFileEditsResult>`) and `handle_file_saved` / `handle_diff_computed` (record per-file completion; surfaces call these from their save-completion events). Each flow method ends with the completion check: when every file is saved and its result diff computed, assemble the result and send it.

`DiffSaveState` encapsulates the in-flight accept: per-file progress (`SavingDiffs`) plus the result-delivery oneshot, with private fields so surfaces cannot reach into the channel. Each surface stores one and exposes it via `save_state_mut`; only `is_saving` is public (revert guards).

The completion check + `assemble_result` is master's `try_emit_diffs_saved` relocated to shared code: it combines the per-file `DiffResult`s into the unified diff, builds updated/deleted file state from `pending_file_state` (`updated_file_contexts_from_content_map`), and maps save errors to `DiffApplicationFailed`. Delivery through the stored oneshot replaces master's `SavedAcceptedDiffs` event + executor subscription. Dropping a surface mid-save drops the sender, resolving the future with `Cancelled`.

`SavingDiffs` (per-file save status + computed result diff, complete when every file has both) moves from `code_diff_view.rs` to `diff_storage.rs` unchanged in behavior.

### The `RegisteredDiffStorage` trait

The executor-facing handle over a registered surface. GUI `ViewHandle`s and model `ModelHandle`s share no common handle type, so each surface's handle type implements this directly, delegating to its entity's `DiffStorage`:

- `set_candidate_diffs(diffs, session_type, app)` — preprocess pushes resolved diffs into the surface.
- `accept_and_save(app)` — persists everything, resolving with the result for the LLM.

The GUI impl is on `WeakViewHandle<CodeDiffView>` (`code_diff_view.rs`): it upgrades and delegates, so the executor never keeps a dead review view alive; a dead view at execute time resolves `DiffApplicationFailed` recoverably. It flips the view to `Accepted` (`mark_accepted_for_save`) as persistence kicks off, then runs `DiffStorageHelper::accept_and_save`.

### Executor (`request_file_edits.rs`)

Per-action state keeps master's two-field shape:

- `diff_storages: HashMap<AIAgentActionId, Box<dyn RegisteredDiffStorage>>` — master's `diff_views`, with the storage interface in place of the concrete GUI view handle.
- `diff_application_failures: HashMap<AIAgentActionId, Vec1<DiffApplicationError>>` — unchanged.

- `register_requested_edits(action_id, storage)` is a plain insert and MUST be called before `preprocess_action` or `execute` (master's contract, now stated for every surface).
- `on_diffs_applied` seeds the registered storage with the resolved diffs; with none registered it warns and drops them (master behavior). Failures insert into `diff_application_failures`.
- `execute`: failures → `DiffApplicationFailed`; otherwise `storage.accept_and_save(ctx)` wrapped in `ActionExecution::new_async` with the `EditResolved` accept telemetry; no entry → `NotReady`.
- Cleanup: every terminal outcome funnels through the action model's `handle_action_result` → `discard_action_state` → `discard_pending`, which drops both maps' entries, so prepared content never outlives its action.
- `should_autoexecute` allows continue-on-failure for failed diff application, unchanged.

### GUI (`code_diff_view.rs`, `block.rs`, `inline_diff.rs`)

`CodeDiffView` implements `DiffStorage`; its core save flow is master's, untouched:

- `start_saving` drives `InlineDiffView::accept_and_save_diff` per file (compute result diff + save editor content through `FileModel`); completions arrive via the per-file `FileSaved`/`FailedToSave`/`DiffAccepted` subscriptions, which forward into `handle_file_saved`/`handle_diff_computed` by index. The GUI's result diff stays editor-computed, as on master; save failures surface master's per-file toasts.
- `pending_file_state` is master's result extraction behind the accessor: final content from the editor buffers (possibly user-edited), changed lines from editor state, rename/delete bookkeeping.
- `save_state: DiffSaveState` is a view field; `CodeDiffState::Accepted` is payloadless. Revert requires a settled accept (`Accepted` with no in-flight save).
- `block.rs` registers `Box::new(view.downgrade())` with the executor at view creation (`handle_requested_edit_complete`), upholding the register-before-preprocess contract exactly as master did. On `TryAccept` the block emits malformed-line telemetry and calls `execute_action`, as before. View-only sessions still populate payload diffs directly and never register — a spectator's view must never become the surface that writes edits to disk.

### Passive path (`terminal/view.rs`)

`on_maa_code_diff_generated`'s `TryAccept` handler calls `DiffStorageHelper::accept_and_save` on the view directly — passive diffs are not executor actions, so the view is the sole owner. The result is not surfaced to the LLM; failed writes surface the per-file toasts.

### TUI surface (up-stack)

The TUI surface registers its own storage through `register_requested_edits` for every `RequestFileEdits` action — including auto-approved edits with no review UI — upholding the same registration contract as the GUI. Its headless persistence (final-content derivation from diff deltas, `FileModel` write dispatch, `similar`-based result diffs) lives up-stack with that surface. `FileDiff::line_stats` (in `diff_types.rs`) exists for its summary rendering.

## Boundaries

- The review surface's state is the only resident copy of diff content while under review; the executor holds only erased storage handles (weak, for the GUI).
- The resolve side (`ApplyDiffModel`) is unchanged.
- Revert stays GUI-local via `InlineDiffView::restore_diff_base`.
- A surface that fails to register before diffs resolve leaves the action unexecutable (`NotReady`), as on master; the registration contract is upheld by each surface at action-arrival time.

## Testing and validation

- Shared-flow tests exercise the `DiffStorageHelper` flow through a minimal test surface (`app/src/ai/blocklist/diff_storage_tests.rs`): result assembly on success, save failure → `DiffApplicationFailed`, deleted-file reporting, and the context-fragment extraction.
- Executor tests cover the registry lifecycle (`request_file_edits_tests.rs`): preprocess seeding of the registered storage, execution through the registered storage, preprocess failure reporting, `NotReady` without a registered storage, and `discard_pending`.

```bash
cargo nextest run -p warp diff_storage request_file_edits
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
