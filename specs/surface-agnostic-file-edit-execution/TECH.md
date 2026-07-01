# Surface-Agnostic File-Edit Execution TECH
## Context
This branch is stacked on `tui-agent-tool-calling`, which renders `RequestFileEdits` in the transcript like any other tool call but leaves it unexecuted on non-GUI surfaces: `RequestFileEditsExecutor::execute` requires a registered GUI `CodeDiffView` and returns `NotReady` otherwise (`app/src/ai/blocklist/action_model/execute/request_file_edits.rs`).
The executor is a shared tool executor that both the GUI terminal and the TUI drive, but it is coupled to the GUI in three ways:
- It stores `diff_views: HashMap<AIAgentActionId, ViewHandle<CodeDiffView>>` and drives that view to save files and read back the result (`request_file_edits.rs`).
- The GUI `CodeDiffView` is the only producer of a successful `RequestFileEditsResult`, assembled in `try_emit_diffs_saved` and emitted as `CodeDiffViewEvent::SavedAcceptedDiffs` (`app/src/ai/blocklist/inline_action/code_diff_view.rs`), which the executor repackages.
- The diff data types `FileDiff`, `DiffBase`, and `DiffSessionType` are defined inside the GUI view module `code_diff_view.rs`.
`RequestFileEdits` has a two-phase lifecycle: `preprocess_action` resolves the LLM's edits into concrete diffs (async file reads via `ApplyDiffModel::apply_diffs`) and `execute` runs later to persist them. State computed in preprocess must survive an arbitrary user-interaction gap. Today the `CodeDiffView` is that survivor — its editor buffers hold the resolved (and possibly user-edited) diffs and produce the result on accept.
Relevant code:
- `app/src/ai/blocklist/action_model/execute/request_file_edits.rs` — executor: `diff_views`, `diff_application_failures`, `execute`, `on_diffs_applied`, `should_autoexecute`, `register_requested_edits`, and the shared `updated_file_contexts_from_editor_buffers` helper.
- `app/src/ai/blocklist/action_model/execute/request_file_edits/apply_diff_model.rs` — the GUI-less resolve-side submodel pattern this refactor mirrors for the persist side.
- `app/src/ai/blocklist/inline_action/code_diff_view.rs` — `CodeDiffView`, the `FileDiff`/`DiffBase`/`DiffSessionType` definitions, `set_candidate_diffs`, `accept_and_save`, `SavingDiffs`/`SaveStatus`/`DiffApplicationState`, `handle_save_completed`, `accepted_file_diff_computed`, `try_emit_diffs_saved`, and the `SavedAcceptedDiffs` event.
- `app/src/code/inline_diff.rs` — `InlineDiffView::register_file`, `save_content`, `accept_and_save_diff`, `restore_diff_base`.
- `app/src/ai/blocklist/block.rs` — `handle_requested_edit_complete` creates the `CodeDiffView`, calls `register_requested_edits`, subscribes to `CodeDiffViewEvent`, and on `TryAccept` calls `action_model.execute_action`.
- `app/src/terminal/view.rs` — `on_maa_code_diff_generated` builds a `CodeDiffView::new_passive` and self-drives `accept_and_save` + `SavedAcceptedDiffs` for passive code-diff suggestions, independent of the executor.
- `crates/warp_files/src/lib.rs` — `FileModel::save` / `rename_and_save` / `delete` route local vs. remote and emit `FileSaved` / `FailedToSave`; registration via `register_file_path` / `register_remote_file`.
## Goal
Make file-edit tool calls executable on any surface (GUI and TUI/headless) by routing all persistence through one shared, non-GUI model, and remove the executor's GUI coupling. The interactive review surface — the only thing that genuinely differs between GUI and TUI — stays in the GUI, which hands the executor plain data.
## Proposed changes
### Two focused models
Keep `ApplyDiffModel` as-is — it remains the resolve/read-and-compute half (`apply_diffs` -> `Vec<AIRequestedCodeDiff>`). Add a sibling `PersistDiffModel` in `app/src/ai/blocklist/action_model/execute/request_file_edits/persist_diff_model.rs` that owns `active_session` and is the single writer + result producer for both surfaces.
### PersistDiffModel
One entry point:
```rust
pub(crate) fn persist(
    &mut self,
    files: Vec<ResolvedFileEdit>,
    ctx: &mut ModelContext<Self>,
) -> BoxFuture<'static, RequestFileEditsResult>
```
where `ResolvedFileEdit` is plain data: `{ path, base_content, op: DiffType, final_content: String }`. It resolves the backend from `active_session` once (`SessionType::WarpifiedRemote { host_id: Some(_) }` -> remote, else local — the match currently in `on_diffs_applied`), then per file registers with `FileModel` (`register_file_path` local / `register_remote_file` remote) and dispatches `save` / `rename_and_save` / `delete`, subscribes to `FileModelEvent` keyed by `FileId`, and when all files resolve assembles `RequestFileEditsResult`: combine per-file `DiffResult` via a `similar`-based `diff_result`, build `updated_files` via the shared `updated_file_contexts_from_editor_buffers`, collect `deleted_files` (including rename sources), and map any `FailedToSave` -> `DiffApplicationFailed`.
The delta-application/diff helpers move here from the GUI/executor: `apply_deltas_to_content`, `split_lines_preserving_newlines`, `diff_result`, `changed_lines_for_result` (headless variant), `changed_line_range_for_delta`, `inserted_content_range`.
### Executor becomes surface-agnostic
Collapse the executor's per-action state into one map. The existing `diff_application_failures` and the resolved diffs are mutually exclusive outcomes of preprocess, and reviewed content only applies to the prepared case, so model them as one state:
```rust
enum PendingFileEdits {
    Prepared {
        diffs: Vec<FileDiff>,
        session_type: DiffSessionType,
        reviewed: Option<Vec<(String, String)>>, // GUI-supplied final content
    },
    Failed(Vec1<DiffApplicationError>),
}
```
Executor fields become: `active_session`, `apply_diff_model`, `persist_diff_model`, `pending: HashMap<AIAgentActionId, PendingFileEdits>`, `terminal_view_id` — replacing `diff_views` and `diff_application_failures`. Remove `register_requested_edits`; add `set_reviewed_content(action_id, Vec<(path, content)>)` (plain data) and a getter exposing the prepared `(Vec<FileDiff>, DiffSessionType)` for the GUI to feed its own view.
- `on_diffs_applied`: insert `Prepared { diffs, session_type, reviewed: None }` on success, or `Failed(errors)` on failure; no view calls.
- `set_reviewed_content`: if the entry is `Prepared`, set `reviewed = Some(files)`.
- `should_autoexecute`: allow continue-on-failure via `matches!(self.pending.get(id), Some(PendingFileEdits::Failed(_)))`.
- `execute`: `match self.pending.remove(id)` — `Failed` returns `DiffApplicationFailed`; `Prepared` assembles per-file `ResolvedFileEdit` (`final_content` = `reviewed` if present, else `base + deltas` applied) and calls `persist_diff_model.persist(...)`, returning `ActionExecution::new_async`. One path for both surfaces; no `CodeDiffView`, no completion channel. Remove the `TODO(surface-agnostic-file-edit-execution)` left on the parent branch.
### GUI reroute (`block.rs`)
- `AIBlock` feeds prepared diffs to its own `CodeDiffView` as data (generalize the existing view-only branch in `handle_requested_edit_complete`), obtaining `(Vec<FileDiff>, DiffSessionType)` from the executor getter or recomputing `DiffSessionType` from its `active_session`.
- On `CodeDiffViewEvent::TryAccept`: extract `editor.text()` per file, call `executor.set_reviewed_content(action_id, files)`, then `action_model.execute_action` as today.
- Save-error surfacing: the failure now returns in `RequestFileEditsResult::DiffApplicationFailed`; `AIBlock` shows the error toast by observing the action result instead of the old per-file `FileModel` `FailedToSave` event.
### Strip view-save machinery
Remove from `CodeDiffView`: `accept_and_save`, `SavingDiffs`, `SaveStatus`, `DiffApplicationState`, `accepted_file_diff_computed`, `handle_save_completed`, the result-assembly half of `try_emit_diffs_saved`, and the `SavedAcceptedDiffs` event; collapse `CodeDiffState::Accepted(Option<SavingDiffs>)` to a payloadless `Accepted` and fix its match sites (`is_complete`, revert guard, `try_accept`). Remove from `InlineDiffView`: `accept_and_save_diff` and `save_content`.
Keep GUI-side: editor rendering, delta application for display, `was_edited` tracking, the malformed-line/edited telemetry (relocated out of `try_emit_diffs_saved`, still computed from editor state at accept and emitted GUI-side), and post-accept revert (`restore_diff_base` + its `FileModel` registration).
### Passive-suggestion reroute (`terminal/view.rs`)
`on_maa_code_diff_generated` currently self-drives `view.accept_and_save` + `SavedAcceptedDiffs`. Reroute it through `PersistDiffModel`: on `TryAccept`, assemble `ResolvedFileEdit`s from the passive `Vec<FileDiff>` (already in scope) plus the editor content, call `persist`, and notify on completion. The terminal view constructs/holds a `PersistDiffModel` via its `active_session`. The existing `ContinuePassiveCodeDiffWithAgent` result reporting is unchanged.
### Relocate diff data types
Move `FileDiff`, `DiffBase`, and `DiffSessionType` out of `code_diff_view.rs` into the `request_file_edits` module so neither the executor nor the persist model imports a GUI view file. Update imports in `code_diff_view.rs`, `inline_diff.rs`, `request_file_edits.rs`, `block.rs`, and `terminal/view.rs`.
### Call flow
Base (GUI-only), unchanged on the parent branch:
```
PHASE 1 preprocess: apply_diffs -> on_diffs_applied -> diff_view.set_candidate_diffs
                                                       (diffs live inside the view)
        |--------- GAP: user reviews / edits / clicks Accept ---------|
PHASE 2 execute: diff_views.get(id) -> accept_and_save
                     -> reads editor buffers -> FileModel::save -> result (view-produced)
```
New (surface-agnostic):
```
PHASE 1 preprocess: apply_diffs -> on_diffs_applied -> self.pending[id] = Prepared{..}
        |-- GUI: user reviews/edits -> set_reviewed_content(id) fills `reviewed` --|
        |   TUI/headless: no view, no gap, reviewed = None                         |
PHASE 2 execute: self.pending.remove(id)
                   Failed   -> DiffApplicationFailed
                   Prepared -> ResolvedFileEdit{ final = reviewed | base+deltas }
                            -> PersistDiffModel::persist -> FileModel -> result
```
The result diff sent to the LLM becomes uniformly `similar`-based for both surfaces (previously the GUI used the editor-computed diff).
## Boundaries
- Do not change the resolve side (`ApplyDiffModel` keeps its name and behavior).
- Revert stays GUI-local via `InlineDiffView::restore_diff_base`; the persist model does not own revert.
- No TUI review/approval UI; the TUI registers no reviewed content and runs the headless persist branch of `execute`.
## Testing and validation
Unit-test `PersistDiffModel::persist` (async `App::test`, await the future):
- Create, update, delete each write via `FileModel` and return `RequestFileEditsResult::Success`.
- Update-with-rename routes through `rename_and_save` and reports the old path in `deleted_files`.
- Save failure returns `DiffApplicationFailed`; preprocessing failure (from `on_diffs_applied`) returns `DiffApplicationFailed`.
- Remote backend selection under a `WarpifiedRemote` session dispatches to the remote path rather than local `std::fs`.
Add an executor test that `set_reviewed_content` overrides delta-applied content on the GUI path while its absence falls back to delta application on the headless path.
Targeted runs then format + clippy:
```bash
cargo test -p warp request_file_edits
cargo test -p warp persist_diff_model
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
## Parallelization
Do not use child agents. The change is tightly coupled across the executor, one new submodel, the GUI block wiring, the code-diff view, and the passive-suggestion path in a single crate; splitting across worktrees would create more merge overhead than wall-clock savings.
Implementation sequence:
```mermaid
flowchart LR
  A["Relocate diff data types"] --> B["PersistDiffModel"]
  B --> C["Executor surface-agnostic rewrite"]
  C --> D["GUI block reroute"]
  D --> E["Strip view-save machinery"]
  E --> F["Passive-suggestion reroute"]
  F --> G["Tests + format + clippy"]
```
