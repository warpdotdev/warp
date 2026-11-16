# APP-3922: Create PR Dialog — Tech Spec
## Problem
APP-3920 unified commit, push, and publish into a single `GitDialog` view and collapsed the event contract to `Completed | Cancelled` (each mode owns its own toasts and error messaging). The git operations button can reach `PrimaryGitActionMode::CreatePr` (everything pushed, no PR, not on main), but `OpenCreatePrDialog` and `CommitAndCreatePr` actions were stubbed as TODOs. This branch adds the PR dialog as a third mode alongside commit and push, wires the "Commit and create PR" chain in commit mode, and includes a few related fixes.
## Relevant Code
- `app/src/code_review/git_dialog/pr.rs` — new submodule for the `CreatePr` mode
- `app/src/code_review/git_dialog/mod.rs` — extended with `CreatePr(PrState)`, `Pr(PrSubAction)`, `new_for_pr()`, and mode dispatch
- `app/src/code_review/git_dialog/commit.rs` — extended with `CommitAndCreatePr` intent, `allow_create_pr: bool` parameter, third intent button (hidden when the intent isn't meaningful), a private `CommitOutcome` enum for confirm results, and a chained `run_commit → run_push → create_pr` async
- `app/src/code_review/code_review_view.rs` — `open_pr_dialog()`, `allow_create_pr` computed and passed through `open_commit_dialog()`, `OpenCreatePrDialog` / `CommitAndCreatePr` action handlers, `update_git_operations_ui` refresh on `NewDiffsComputed`
- `app/src/util/git.rs` — `get_branch_diff_entries()` (branch-level numstat diff). `create_pr()` and `PrInfo` already existed.
## Current State
`GitDialog` owns commit and push modes with a shared shape: per-mode state struct, body renderer, confirm async, and dispatch in `mod.rs`. Each mode calls `show_toast` / `user_facing_git_error` (declared in `git_dialog/mod.rs`) on success and failure, then emits `GitDialogEvent::Completed`. The parent closes the dialog and refreshes metadata; it no longer knows anything about outcomes. `gh` CLI helpers (`run_gh_command`, `get_pr_for_branch`, `PrInfo`, `create_pr`) already exist.
## Proposed Changes
### 1. PR mode for `GitDialog` (`git_dialog/pr.rs`)
A new submodule, following the same shape as `commit.rs` and `push.rs`.
**State:**
```rust
pub struct PrState {
    file_changes: Vec<FileChangeEntry>,
    changes_expanded: bool,
    summary_mouse_state: MouseStateHandle,
    changes_scroll_state: ClippedScrollStateHandle,
}
```
**Sub-action:**
```rust
pub enum PrSubAction {
    ToggleChangesExpanded,
}
```
**Body:** branch header + "Changes" section with aggregate stats (file count, +additions, -deletions) and expandable per-file list (scrollable, max 130px). Uses the shared `render_branch_section` / `render_chevron_icon` / `render_file_list` helpers in `git_dialog/mod.rs`.
**Constructor:** `pr::new_state(repo_path, ctx)` spawns `get_branch_diff_entries` to populate `file_changes`.
**Confirm:** `pr::start_confirm` spawns `create_pr(&repo_path)`.
- On success: calls `show_pr_created_toast(&pr_info, ctx)` (see below).
- On failure: logs the raw error and calls `show_toast(user_facing_git_error(&err), ctx)`.
- Either way emits `GitDialogEvent::Completed`.
**Toast helper:** `pr::show_pr_created_toast(pr_info, ctx)` — ephemeral `DismissibleToast` with message `"PR successfully created."` and a clickable "Open PR" `ToastLink` pointing at `pr_info.url`. Declared `pub(super)` so `commit.rs` can reuse it for the `CommitAndCreatePr` chain.
**Labels/icon:** title = "Create pull request"; confirm button = "Create PR" / `Icon::Github`; loading = "Creating…".
### 2. `GitDialog` mode dispatch (`git_dialog/mod.rs`)
- New variant `GitDialogMode::CreatePr(PrState)`
- New variant `GitDialogAction::Pr(PrSubAction)`
- New constructor `GitDialog::new_for_pr(repo_path, branch_name, ctx)`
- Title / body / focus / confirm / sub-action dispatch extended for `CreatePr`
- `new_for_commit` signature grows an `allow_create_pr: bool` parameter so commit mode can hide its "Commit and create PR" button when the intent isn't meaningful (existing PR or main branch). The caller encodes both conditions into the single boolean so the dialog doesn't need to know the underlying reasons.
### 3. `CommitAndCreatePr` intent (`git_dialog/commit.rs`)
- Adds `CommitIntent::CommitAndCreatePr` variant
- `confirm_label_for` / `confirm_icon_for` / `loading_label_for` extended
- `CommitState` gains `commit_and_create_pr_button: Option<ViewHandle<ActionButton>>` — `None` when `allow_create_pr` is false (existing PR or main branch)
- `new_state` takes `allow_create_pr: bool`. When false:
    - The "Commit and create PR" button is omitted entirely (not just disabled)
    - A `debug_assert!` catches callers that dispatched `CommitAndCreatePr` anyway; in release the subsequent `create_pr` call surfaces the real `gh` error via the normal failure path rather than silently rewriting the intent
- `apply_intent_selector` and `render_intent_buttons` skip the third button when it's `None`
- A private `CommitOutcome { Committed | Pushed | PrCreated(PrInfo) }` enum represents what actually ran, keeping "which stages fired" decoupled from the user's selected intent so the callback can't drift out of sync with the async body.
- `start_confirm` chains `run_commit` → `run_push` (for `CommitAndPush` or `CommitAndCreatePr`) → `create_pr` (for `CommitAndCreatePr`) in a single `ctx.spawn`, returning a `CommitOutcome`
- Success toasts by outcome:
    - `Committed` → `"Changes successfully committed."`
    - `Pushed` → `"Changes committed and pushed."`
    - `PrCreated(pr)` → `show_pr_created_toast(&pr, ctx)` (same link-bearing toast as standalone PR creation)
- Failure → `show_toast(user_facing_git_error(...), ctx)`
- Either way emits `GitDialogEvent::Completed`
### 4. Code review view integration (`code_review_view.rs`)
- **`open_pr_dialog(ctx)`** — uses the existing `prepare_git_dialog` / `attach_git_dialog` helpers; constructs `GitDialog::new_for_pr(...)`.
- **`open_commit_dialog(intent, ctx)`** — computes `allow_create_pr = pr_info.is_none() && !is_on_main_branch()` from `diff_state_model` and passes it through to `GitDialog::new_for_commit`, so the commit dialog can hide the "Create PR" intent when it isn't meaningful.
- **Action wiring:**
  - `OpenCreatePrDialog` → `self.open_pr_dialog(ctx)` (was TODO)
  - `CommitAndCreatePr` → `self.open_commit_dialog(CommitIntent::CommitAndCreatePr, ctx)` (was TODO)
- **Git operations button refresh fix:** `DiffStateModelEvent::NewDiffsComputed` handler now calls `update_git_operations_ui(ctx)` so after a commit the button transitions from "Commit" → "Push" (or "Create PR") without waiting for another event.
Parent still knows nothing about outcomes — its `attach_git_dialog` subscriber remains just `Completed → close + refresh` and `Cancelled → close`.
### 5. Git utility (`util/git.rs`)
**`get_branch_diff_entries(repo_path)`** — returns per-file change stats for the branch diff:
- Detects base branch via `detect_main_branch`, current branch via `detect_current_branch`.
- Diffs `{base}..origin/{current}`, falling back to `{base}..HEAD` if the remote ref doesn't exist (e.g. branch not yet pushed).
- Parses `git diff --numstat` into `Vec<FileChangeEntry>`.
## End-to-End Flows
### Standalone "Create PR" flow
1. User clicks "Create PR" button → `OpenCreatePrDialog` action
2. `open_pr_dialog(ctx)` → `GitDialog::new_for_pr(...)`; `pr::new_state` spawns `get_branch_diff_entries`
3. Dialog renders with branch info and change summary
4. User clicks "Create PR" → `GitDialogAction::Confirm` → `pr::start_confirm` spawns `create_pr`
5. Confirm/cancel/close disabled, confirm label reads "Creating…"
6. On success → `show_pr_created_toast` fires ("PR successfully created." with "Open PR" link) → emits `Completed` → parent closes dialog + refreshes metadata (header button becomes "PR #N")
7. On error → toast with friendly message → emits `Completed` → parent closes dialog + refreshes
### "Commit and create PR" flow
1. User selects "Commit and create PR" in the commit dialog → `CommitIntent::CommitAndCreatePr`
2. Confirm handler chains `run_commit` → `run_push` → `create_pr` in a single `ctx.spawn`
3. On success → `show_pr_created_toast(&pr, ctx)` → `Completed` → parent closes dialog + refreshes
4. On failure at any stage → friendly error toast → `Completed` → parent closes dialog + refreshes
### "Commit and create PR" when the intent isn't meaningful (existing PR or main branch)
1. `open_commit_dialog` computes `allow_create_pr` from the diff state model (PR info + `is_on_main_branch`) and passes it to `GitDialog::new_for_commit`
2. The third intent button is omitted entirely (no disabled dead button)
3. The caller is expected to not dispatch `CommitAndCreatePr` in this state; a `debug_assert!` in `commit::new_state` catches violations in dev builds, and in release the subsequent `create_pr` failure surfaces via the normal error toast path
## Risks and Mitigations
### `gh` CLI not installed or not authenticated
Currently no pre-check. `create_pr` fails with a descriptive error from `gh`. The error surfaces as a friendly toast via the unified `Failed` path. Follow-up: add a `gh` availability check before enabling the action.
### Git operations button not updating after commit
Fixed by calling `update_git_operations_ui` in the `NewDiffsComputed` handler, so after a commit triggers a diff reload, the button state is re-evaluated.
## Testing and Validation
- Verify PR dialog opens with correct branch name and file change stats.
- Verify expanding changes shows per-file list with correct +/- stats.
- Verify loading state (all three chrome buttons disabled) during PR creation.
- Verify success shows the "PR successfully created." toast with "Open PR" link and header updates to "PR #N".
- Verify error shows friendly error toast and dialog closes.
- Verify "Commit and create PR" chains all three operations and shows the same PR toast on success.
- Verify commit dialog omits the third button entirely when the branch already has a PR OR when on the repo's main branch.
- Verify cancel/close/ESC dismisses the dialog without side effects.
- Verify the git operations button transitions correctly across Commit → Push → Create PR → PR #N.
## Follow-ups
- Add `gh` CLI availability/auth check before enabling "Create PR" and "Commit and create PR" actions.
- Support editing PR title and body (currently uses `--fill`).
- Support draft PRs.
- Mid-flight mode transitions during `CommitAndCreatePr` so the dialog reflects "Committing…" → "Pushing…" → "Creating PR…" stages.
