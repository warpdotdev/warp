# APP-3920: Push and Publish Dialogs — Tech Spec
## Problem
APP-3918 added the git operations button with `OpenPushDialog` and `PublishBranch` actions, but both were stubbed as TODOs. This branch implements push/publish behavior, adds per-commit file stats to the `Commit` struct, and chains the push operation after "Commit and push" from the commit dialog.
During review of the initial per-dialog implementation, reviewer feedback observed that commit, push, and the (upcoming) create-PR dialogs share the bulk of their UI — same `Dialog` chrome, same branch/file helpers, same loading lifecycle — but had diverged on behavior, particularly around how each reacts to failure. This branch therefore also unifies the previously separate `CommitDialog` and `PushDialog` views into a single `GitDialog` view with per-mode submodules. The upcoming PR dialog work (`edward/pr-dialog`) adds `CreatePr` as a third mode rather than a new standalone file.
## Relevant Code
- `app/src/code_review/git_dialog/mod.rs` — `GitDialog` view, `GitDialogMode`, shared chrome (title, close/cancel/confirm buttons, overlay), unified action/event/outcome enums, ESC keybinding, dispatch, loading lifecycle
- `app/src/code_review/git_dialog/commit.rs` — `CommitState`, `CommitIntent`, body renderer, async `run_commit` (+ optional chained `run_push`), `on_focus` targets the message editor
- `app/src/code_review/git_dialog/push.rs` — `PushState`, body renderer with commit list, async `run_push` (shared by push and publish flows)
- `app/src/code_review/dialog_common.rs` — shared helpers: `render_branch_section`, `render_chevron_icon`, `render_file_list`, `render_dialog_overlay`
- `app/src/code_review/code_review_view.rs` — `open_commit_dialog()`, `open_push_dialog()`, the shared `prepare_git_dialog()` / `attach_git_dialog()` helpers, single `git_dialog: Option<ViewHandle<GitDialog>>` field, collapsed render arm, `PublishBranch`/`OpenPushDialog`/`CommitAndPush` action wiring
- `app/src/util/git.rs` — `run_push()` (uses `--set-upstream` for both push and publish), `get_commit_files()` (per-commit file stats), `get_unpushed_commits()` and `Commit` struct
## Current State
The git operations button (APP-3918) dispatches `OpenPushDialog` and `PublishBranch` actions. The commit dialog (APP-3919) originally shipped as its own `CommitDialog` view. Prior to this branch, the push dialog did not exist, the publish action was a TODO, and the commit-and-push intent was not wired.
`run_push` already uses `git push --set-upstream origin <branch>`, so it handles both regular push and first-time publish identically at the git level.
## Proposed Changes
### 1. Unified `GitDialog` view (`git_dialog/`)
A single view replaces `CommitDialog` / `PushDialog` (and, in the stacked `edward/pr-dialog` branch, the would-be `PrDialog`). `mod.rs` owns everything shared; each mode lives in its own submodule.
**Module layout:**
```
app/src/code_review/git_dialog/
  mod.rs      // GitDialog view + GitDialogMode + actions/events + dispatch + ESC binding
  commit.rs   // CommitState + body renderer + run_commit async (chains run_push on CommitAndPush)
  push.rs     // PushState + body renderer + run_push async (used by both push and publish)
```
**Outer struct:**
```rust
pub struct GitDialog {
    repo_path: PathBuf,
    branch_name: String,
    mode: GitDialogMode,
    loading: bool,
    confirm_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    close_button: ViewHandle<ActionButton>,
}
enum GitDialogMode {
    Commit(CommitState),
    Push(PushState),
    // CreatePr(PrState) is added on edward/pr-dialog.
}
```
**Actions:**
```rust
pub enum GitDialogAction {
    Cancel,
    Confirm,
    Commit(CommitSubAction),   // SetIntent, ToggleIncludeUnstaged, ToggleChangesExpanded
    Push(PushSubAction),       // ToggleCommit(String)
}
```
**Events and outcomes:**
```rust
pub enum GitDialogOutcome {
    CommitOnly,
    CommitAndPush,
    Pushed { publish: bool },
}
pub enum GitDialogEvent {
    Succeeded(GitDialogOutcome),
    Failed(String),
    Cancelled,
}
```
**Constructors:**
- `GitDialog::new_commit(repo, branch, intent, ctx)`
- `GitDialog::new_push(repo, branch, publish, commits, ctx)`
### 2. Unified behavior policies
Decisions that had drifted per-dialog are now consolidated in one place:
- **ESC**: one `FixedBinding` in `git_dialog::init()` under `ui_name = "GitDialog"` dispatches `GitDialogAction::Cancel`. No-op while `loading`.
- **Loading**: `set_loading(label)` disables confirm, cancel, and close, and swaps the confirm label (e.g. `"Committing…"`, `"Committing and pushing…"`, `"Pushing…"`, `"Publishing…"`).
- **On failure**: emit `GitDialogEvent::Failed(err)`. Parent closes the dialog and toasts — same policy for all modes. This is a behavior change from the initial push dialog implementation, which kept itself open on failure; reviewer feedback explicitly called for this alignment with commit's behavior.
- **On success**: emit `GitDialogEvent::Succeeded(outcome)`. Parent closes the dialog, toasts an outcome-specific message, and refreshes diffs/metadata/PR info.
- **Focus**: `GitDialog::on_focus` delegates to a per-mode `on_focus(state, ctx)` helper. `commit::on_focus` focuses the message editor; `push::on_focus` is a no-op. Keeps `mod.rs` mode-agnostic while preserving commit's auto-focus.
### 3. Commit struct enrichment (`git.rs`)
`Commit` gains `files_changed`, `additions`, and `deletions` fields, populated during `get_unpushed_commits()` by parsing `--numstat` output alongside the existing `--format` output.
New function `get_commit_files(repo_path, hash)` runs `git diff-tree --no-commit-id -r --numstat <hash>` and returns `Vec<FileChangeEntry>` for the expanded commit view.
### 4. Code review view integration (`code_review_view.rs`)
The view now owns a single `git_dialog: Option<ViewHandle<GitDialog>>` field (replacing the previous separate `commit_dialog` and `push_dialog` fields). Two shared helpers eliminate the duplicated open logic:
- `prepare_git_dialog(&self, ctx) -> Option<(PathBuf, String)>` — guards: no-op if a dialog is already open or git operations are blocked; early-returns if `repo_path` is `None`; returns `(repo_path, branch_name)` otherwise.
- `attach_git_dialog(dialog, ctx)` — subscribes to the unified event stream, stores the handle, focuses it. Success matches on `GitDialogOutcome` to pick the toast (`"Changes successfully committed."`, `"Changes committed and pushed."`, `"Branch successfully published."`, `"Changes successfully pushed."`). Failure and success both clear the dialog and call `refresh_after_git_operation`.
**Per-mode entrypoints:**
- `open_commit_dialog(intent, ctx)` → `GitDialog::new_commit(...)`
- `open_push_dialog(publish, ctx)` → `GitDialog::new_push(...)`
Action wiring unchanged:
- `OpenCommitDialog` → `open_commit_dialog(CommitIntent::CommitOnly, ctx)`
- `CommitAndPush` → `open_commit_dialog(CommitIntent::CommitAndPush, ctx)`
- `OpenPushDialog` → `open_push_dialog(false, ctx)`
- `PublishBranch` → `open_push_dialog(true, ctx)`
Render block: a single `Some(git_dialog)` arm replaces the previous `else if let Some(commit_dialog)` / `else if let Some(push_dialog)` branches.
**Extracted helpers** (unchanged from the original push dialog landing):
- `show_toast(msg, ctx)` — shows an ephemeral `DismissibleToast`
- `refresh_after_git_operation(ctx)` — reloads diffs, refreshes diff metadata with `PromptRefresh`, refreshes PR info, and calls `ctx.notify()`
### 5. Commit and push chaining
Commit mode's `Confirm` handler checks `intent == CommitIntent::CommitAndPush`. If so, it chains `run_push` after a successful `run_commit` in the same `ctx.spawn` block. Loading label is `"Committing and pushing…"`. On success the parent toasts `"Changes committed and pushed."` via `GitDialogOutcome::CommitAndPush`. No mid-flight mode transition — the dialog remains in `Commit` mode throughout.
## End-to-End Flow
### Push flow
1. User clicks "Push" button or dropdown item → `OpenPushDialog` action
2. `open_push_dialog(false, ctx)` reads branch name and unpushed commits from `DiffStateModel`
3. `GitDialog` opens in `Push` mode with commit list; user can expand commits to see files
4. User clicks "Push" → `GitDialogAction::Confirm` → `push::start_confirm` spawns `run_push`
5. Confirm/cancel/close disabled, confirm label reads "Pushing…"
6. On success → `GitDialogEvent::Succeeded(Pushed { publish: false })` → parent closes dialog, toasts, refreshes
7. On error → `GitDialogEvent::Failed(err)` → parent closes dialog, toasts error, refreshes
### Publish flow
Same as push, triggered by `PublishBranch` → `open_push_dialog(true, ctx)`. Title reads "Publish branch"; confirm button reads "Publish" with `UploadCloud` icon; loading label is "Publishing…"; success toast is "Branch successfully published." `run_push` handles `--set-upstream` identically.
### Commit and push flow
1. User selects "Commit and push" in commit dialog → `CommitIntent::CommitAndPush`
2. `Confirm` spawns `run_commit`, then `run_push` on success, in a single `ctx.spawn`
3. Emits `Succeeded(CommitAndPush)` on success or `Failed(err)` on any stage failing
4. Single toast on success; error toast on failure at either stage; dialog closes either way
## Risks and Mitigations
### Failure always closes the dialog (behavior change)
Under the unified policy, failures close the dialog and toast rather than keeping it open. The user can reopen and retry. This was an explicit outcome of reviewer feedback asking for consistency across commit/push/PR; it mirrors the original commit dialog behavior.
### Stale commit list
The commit list is read from `DiffStateModel` at dialog open time. If the user commits via terminal while the dialog is open, the list may be stale. This is acceptable — the dialog is short-lived and the user can close and reopen it.
### `run_push` used for both push and publish
`run_push` always passes `--set-upstream`. For branches that already have an upstream, this is a no-op flag. No risk of incorrect behavior.
### Single ESC binding under `GitDialog`
The unified `ui_name = "GitDialog"` replaces the per-dialog bindings (`CommitDialog`, `PushDialog`). No other code paths depend on the old ui_names.
## Testing and Validation
- Verify commit dialog opens with correct branch name, message editor, and file list.
- Verify commit and commit-and-push both succeed with the correct toast.
- Verify push dialog opens with correct branch name and commit list.
- Verify expanding a commit lazily loads and displays file stats.
- Verify loading state (all three chrome buttons disabled) during each mode's async op.
- Verify success closes dialog and shows mode-specific toast for commit / commit-and-push / push / publish.
- Verify error closes dialog and shows error toast for all modes.
- Verify cancel/close/ESC dismisses the dialog without side effects.
- Verify diff metadata and git operations button update after any successful git operation.
- Verify `on_focus` moves focus to the message editor in commit mode and is a no-op in push/publish.
## Follow-ups
- Create PR dialog: add `git_dialog/pr.rs` with `CreatePr` mode on `edward/pr-dialog`. Extend `GitDialogMode` and `GitDialogOutcome` accordingly. No new top-level file.
- `CommitAndCreatePr` intent: extend `CommitIntent` to include it, chaining `Commit → Push → CreatePr`.
- Add header icon badge matching Figma (`ArrowUp` for push, `UploadCloud` for publish).
- Align close button tooltip across modes (currently all modes show "ESC").
