# Surface-Agnostic File-Edit Execution TECH

## Context

This branch builds on the TUI agent tool-calling work (now on `master`), which renders `RequestFileEdits` in the transcript like any other tool call but leaves it unexecuted on non-GUI surfaces: `RequestFileEditsExecutor::execute` required a registered GUI `CodeDiffView` and returned `NotReady` otherwise, blocking the conversation (`app/src/ai/blocklist/action_model/execute/request_file_edits.rs`).

On master the GUI view owns the whole persistence flow: `CodeDiffView::accept_and_save` drives per-file editor-buffer saves through `InlineDiffView`, tracks completion in `SavingDiffs`, and assembles the `RequestFileEditsResult` in `try_emit_diffs_saved`, which the executor consumes via a `SavedAcceptedDiffs` event subscription. That flow is well proven, but it is expressed entirely in terms of one concrete GUI view, so no other surface can execute file edits.

`RequestFileEdits` has a two-phase lifecycle: `preprocess_action` resolves the LLM's edits into concrete diffs (async file reads via `ApplyDiffModel::apply_diffs`) and `execute` runs later to persist them. State computed in preprocess must survive an arbitrary user-interaction gap; the review surface's buffers are that survivor.

## Goal

Make file-edit tool calls executable on any surface (GUI, TUI, headless) by keeping master's surface-owned persistence flow but expressing its shared parts as a trait, so the GUI `CodeDiffView`, the up-stack TUI diff view, and a headless fallback all run the same save-completion and result-assembly code.

## Design

### The `DiffStorageView` trait (`app/src/ai/blocklist/diff_storage.rs`)

Implemented by every surface that stores pending diffs. Required methods are state accessors — the fields live on each impl, since traits cannot hold state — plus the surface-specific write kickoff. Provided methods are the shared save-completion flow:

- Required: `saving_diffs_mut` (per-file progress), `result_tx_mut` (result delivery channel), `pending_diff_count`, `pending_file_state` (per-file report state: reported paths, changed lines, final contents, user-edit flags), and `start_saving` (the write kickoff).
- Provided: `accept_and_save` (creates a oneshot, sizes `SavingDiffs`, calls `start_saving`, returns the receiver as a `BoxFuture<RequestFileEditsResult>`), `handle_file_saved` / `handle_diff_computed` (record per-file completion), `fail_saving`, and `try_finish` (when every file is saved and its result diff computed, assembles the result and sends it).

`try_finish` is master's `try_emit_diffs_saved` relocated to shared code: it combines the per-file `DiffResult`s into the unified diff, builds updated/deleted file state from `pending_file_state` (`updated_file_contexts_from_content_map`), and maps save errors to `DiffApplicationFailed`. Delivery through the stored oneshot replaces master's `SavedAcceptedDiffs` event + executor subscription. Dropping a surface mid-save drops the sender, resolving the future with `Cancelled`.

`SavingDiffs` (per-file save status + computed result diff, complete when every file has both) moves from `code_diff_view.rs` to `diff_storage.rs` unchanged in behavior.

### The `RegisteredDiffStorage` trait

The executor-facing handle over a registered surface. GUI `ViewHandle`s and model `ModelHandle`s share no common handle type, so each surface registers a thin wrapper that delegates through its own handle:

- `set_candidate_diffs(diffs, session_type, app)` — preprocess pushes resolved diffs into the surface.
- `take_candidate_diffs(app)` — hands diffs back out so a newly registered surface can take over; `None` when this storage keeps ownership.
- `accept_and_save(app)` — persists everything, resolving with the result for the LLM.

`GuiDiffStorage(WeakViewHandle<CodeDiffView>)` (`code_diff_view.rs`) upgrades and delegates; a dead view at execute time resolves `DiffApplicationFailed` recoverably. It flips the view to `Accepted` (`mark_accepted_for_save`) as persistence kicks off, then runs the trait's `accept_and_save`. It never relinquishes diffs — they live in the view's editor buffers.

### `HeadlessDiffStorageModel`

GUI-less `DiffStorageView`: plain diff storage with no review UI, created by the executor when diffs resolve with no registered surface (autoexecution racing view creation, or headless/TUI-driven conversations), so file edits stay executable everywhere. Its `start_saving` derives each file's final content by applying the diff's deltas to the base (`final_content_from_op` / `apply_deltas_to_content`), decides the actual write once via `PersistAction` (`Write` / `Rename` — local only, remote renames fall back to an in-place write reported at the original path / `Delete`), dispatches through `FileModel` (`register_file_path`/`register_remote_file` → `save`/`rename_and_save`/`delete` → `unsubscribe`), computes a `similar`-based `DiffResult` immediately, and forwards its own `FileModel` save events into the shared flow. `headless_file_outcome` derives the reported outcome from the same `PersistAction` as the dispatch, so report and write cannot drift. On wasm, `start_saving` fails the batch (`file editing is not supported in this environment`).

This model is also the template the up-stack TUI diff storage builds on.

### Executor (`request_file_edits.rs`)

Per-action state is a registry:

```rust
enum PendingFileEdits {
    Storage(Box<dyn RegisteredDiffStorage>),
    Failed(Vec1<DiffApplicationError>),
}
```

- `register_requested_edits(action_id, storage)` may be called before or after preprocess resolves. When a placeholder storage already holds prepared diffs, they are handed to the newly registered surface via `take_candidate_diffs`/`set_candidate_diffs`; an owner that does not relinquish (a review surface, or a placeholder already saving) stays registered.
- `on_diffs_applied` seeds the registered storage with the resolved diffs, or creates the headless placeholder when none is registered. Failures insert `Failed`.
- `execute`: `Storage` → `storage.accept_and_save(ctx)` wrapped in `ActionExecution::new_async` with the `EditResolved` accept telemetry; `Failed` → `DiffApplicationFailed`; no entry → `NotReady` (only reachable before preprocess resolves).
- Cleanup: every terminal outcome funnels through the action model's `handle_action_result` → `discard_action_state` → `discard_pending`, which drops the registry entry (and with it the headless model or GUI weak wrapper), so prepared content never outlives its action.
- `should_autoexecute` allows continue-on-failure for `Failed` entries, unchanged.

### GUI (`code_diff_view.rs`, `block.rs`, `inline_diff.rs`)

`CodeDiffView` implements `DiffStorageView`; its core save flow is master's, untouched:

- `start_saving` drives `InlineDiffView::accept_and_save_diff` per file (compute result diff + save editor content through `FileModel`); completions arrive via the per-file `FileSaved`/`FailedToSave`/`DiffAccepted` subscriptions, which forward into `handle_file_saved`/`handle_diff_computed` by index. The GUI's result diff stays editor-computed, as on master; save failures surface master's per-file toasts.
- `pending_file_state` is master's result extraction behind the accessor: final content from the editor buffers (possibly user-edited), changed lines from editor state, rename/delete bookkeeping.
- `saving_diffs` and `save_result_tx` are view fields; `CodeDiffState::Accepted` is payloadless. Revert requires a settled accept (`Accepted` with no in-flight save).
- `block.rs` registers `GuiDiffStorage(view.downgrade())` with the executor at view creation (`handle_requested_edit_complete`); registration order relative to preprocess no longer matters. On `TryAccept` the block emits malformed-line telemetry and calls `execute_action`, as before. View-only sessions still populate payload diffs directly and never register.

### Passive path (`terminal/view.rs`)

`on_maa_code_diff_generated`'s `TryAccept` handler calls the view's `accept_and_save` (the shared trait flow) directly — passive diffs are not executor actions, so the view is the sole owner. The result is not surfaced to the LLM; failed writes surface the per-file toasts.

### TUI surface (up-stack)

The TUI surface registers its own storage through `register_requested_edits`: a diff-storage model shaped like `HeadlessDiffStorageModel` plus display state, reusing the trait's provided flow and the shared `FileModel` dispatch helpers. `FileDiff::line_stats` (in `diff_types.rs`) exists for its summary rendering.

## Boundaries

- The review surface's state is the only resident copy of diff content while under review; the executor holds only erased storage handles (weak, for the GUI).
- The resolve side (`ApplyDiffModel`) is unchanged.
- Revert stays GUI-local via `InlineDiffView::restore_diff_base`.
- Result diffs are editor-computed on the GUI (master behavior) and `similar`-based on headless/TUI surfaces; the divergence is accepted.

## Testing and validation

- Shared-flow tests exercise the provided trait methods through `HeadlessDiffStorageModel` (`app/src/ai/blocklist/diff_storage_tests.rs`): create/update/delete/rename write-through and reported results, save failure → `DiffApplicationFailed`, delta application, remote-rename fallback reporting, and `take_candidate_diffs` ownership rules.
- Executor tests cover the registry lifecycle (`request_file_edits_tests.rs`): placeholder→surface diff handoff, non-relinquishing owners, execution through the registered storage, preprocess failure reporting, `NotReady` without prepared diffs, and `discard_pending`.

```bash
cargo nextest run -p warp diff_storage request_file_edits
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
