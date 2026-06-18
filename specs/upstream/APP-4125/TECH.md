# APP-4125: Unified Git Operations Dropdown — Tech Spec
Product spec: `specs/APP-4125/PRODUCT.md`
Parent stack: APP-3918 (header button) → APP-3920 (commit/push dialogs) → APP-3922 (create-PR dialog) → **APP-4125** (this branch).
## Problem
APP-3920/APP-3922 landed separate dropdown shapes per `PrimaryGitActionMode` and duplicated the commit-chain intents (`CommitAndPush`, `CommitAndCreatePr`) as top-level `CodeReviewAction` variants wired straight from the dropdown. That produced three sources of state that had to stay consistent: the primary button, the chevron's dropdown items, and the commit dialog's intent selector. The chained items in particular lived in two places — the dropdown and the dialog's segmented selector — and the wrong one was always out of date.
Also, the send-to-remote action is semantically a single thing (`git push --set-upstream`) but has two user-facing surfaces (Push when the branch has an upstream, Publish when it doesn't), and that distinction leaked all over the code in ad-hoc `if has_upstream` checks.
Collapse all of this: one dropdown shape, one place that decides push-vs-publish labelling, and the chained intents live only inside the dialog.
## Relevant code
- `app/src/code_review/code_review_view.rs:289-302` — `PrimaryGitActionMode` enum; `Publish` re-split from `Push` so the chevron can be hidden in Publish mode.
- `app/src/code_review/code_review_view.rs:367-406` — `CodeReviewAction`; `CommitAndPush` and `CommitAndCreatePr` variants removed.
- `app/src/code_review/code_review_view.rs:6781-6822` — `open_git_dialog`'s `Commit` arm; computes `allow_create_pr` and `has_upstream` from `DiffStateModel` and plumbs both into `GitDialog::new_for_commit`.
- `app/src/code_review/code_review_view.rs:6826-6856` — `primary_git_action_mode`; Publish is a distinct arm.
- `app/src/code_review/code_review_view.rs:6860-6948` — `update_git_operations_ui`; Publish clears `adjoined_side`, Push enables the chevron.
- `app/src/code_review/code_review_view.rs:6972-7003` — three menu-item helpers: `commit_menu_item`, `push_or_publish_menu_item`, `pr_menu_item`.
- `app/src/code_review/code_review_view.rs:7009-7037` — `git_operations_menu_items`; Commit and Push modes produce a three-item list via the helpers; Publish / CreatePr / ViewPr return `vec![]`.
- `app/src/code_review/git_dialog/mod.rs:48-60` — `GitDialogKind::Commit` no longer carries an intent.
- `app/src/code_review/git_dialog/mod.rs:452-520` — `GitDialog::new_for_commit(repo, branch, allow_create_pr, has_upstream, ctx)` takes both diff-state booleans; `build_dialog_buttons` accepts `Option<Icon>`.
- `app/src/code_review/git_dialog/commit.rs:91-207` — `commit::new_state(repo, allow_create_pr, has_upstream, ctx)`; pre-picks the push/publish label and icon for the middle intent button; initial intent is `CommitIntent::CommitOnly`.
- `app/src/code_review/git_dialog/commit.rs:217-304` — dispatch + confirm: `SetIntent` updates only the segmented selector (confirm chrome is static); `start_confirm` branches on `state.intent` exactly as before.
- `app/src/code_review/code_review_header/header_revamp.rs:116-123` — chevron visibility: `matches!(mode, Commit | Push)`; `Publish` is intentionally excluded.
## Current state
Before this branch:
- The dropdown shape was per-mode: Commit mode listed Commit + Commit-and-push + (Commit-and-create-PR | PR), Push mode listed Commit (greyed) + Push + PR. Publish mode had no dropdown.
- `CommitIntent` was exposed through `GitDialogKind::Commit(CommitIntent)`; the dropdown dispatched `OpenCommitDialog`, `CommitAndPush`, or `CommitAndCreatePr`, which each opened the dialog with a pre-selected intent.
- The commit dialog's confirm button label and icon tracked the selected intent (`Commit` / `Commit and push` / `Commit and create PR` / `GitCommit` / `ArrowUp` / `Github`), updated via a re-render inside `handle_sub_action::SetIntent`.
- The middle intent was always labelled "Commit and push" regardless of whether the branch had an upstream.
- `PrimaryGitActionMode` had five variants including both `Push` and `Publish`.
## Proposed changes
### 1. `CodeReviewAction` cleanup
Drop the two chained variants:
```rust path=null start=null
pub enum CodeReviewAction {
    // …
    OpenCommitDialog,
    ToggleGitOperationsMenu,
    OpenPushDialog,
    OpenCreatePrDialog,
    // CommitAndPush,        // removed
    // CommitAndCreatePr,    // removed
    ViewPr(String),
    PublishBranch,
}
```
Any dropdown item or keyboard shortcut that dispatched those variants is either removed (dropdown items) or was never wired.
### 2. Menu-item helpers
Three static-ish helpers on `CodeReviewView` keep the dropdown arms uniform and capture the small amount of per-item decision logic:
```rust path=null start=null
fn commit_menu_item(disabled: bool) -> MenuItem<CodeReviewAction>;
fn push_or_publish_menu_item(has_upstream: bool, disabled: bool) -> MenuItem<CodeReviewAction>;
fn pr_menu_item(&self, app: &AppContext) -> MenuItem<CodeReviewAction>;
```
`push_or_publish_menu_item` is the single place that swaps label ("Push" / "Publish"), icon (`ArrowUp` / `UploadCloud`), and action (`OpenPushDialog` / `PublishBranch`) based on `has_upstream`. `pr_menu_item` handles both the "PR #N" link and the "Create PR" case, disabling Create PR when on main or when no upstream exists.
`git_operations_menu_items` uses the helpers for its two active arms:
```rust path=null start=null
PrimaryGitActionMode::Commit => vec![
    Self::commit_menu_item(false),
    Self::push_or_publish_menu_item(has_upstream, !has_local_commits),
    self.pr_menu_item(app),
],
PrimaryGitActionMode::Push => vec![
    Self::commit_menu_item(true),
    Self::push_or_publish_menu_item(has_upstream, false),
    self.pr_menu_item(app),
],
PrimaryGitActionMode::Publish
| PrimaryGitActionMode::CreatePr
| PrimaryGitActionMode::ViewPr => vec![],
```
### 3. Keep `Publish` distinct from `Push`
We briefly collapsed Push and Publish, then re-split because the header needs to hide the chevron in Publish mode and the match-based dispatch in `header_revamp.rs` is the natural place to do that. Keeping Publish as its own variant means:
- `primary_git_action_mode()` returns `Publish` exactly when `!has_upstream && (has_local_commits || is_on_main_branch && has_head)`.
- `update_git_operations_ui`'s `Publish` arm calls `clear_adjoined_side` and does not enable the chevron.
- `header_revamp::render_git_operations_button`'s `matches!(mode, Commit | Push)` check excludes Publish, so the chevron is not rendered in the layout.
This is one more enum variant but zero new conditionals outside the enum.
### 4. Commit dialog plumbing
`GitDialogKind::Commit` no longer carries a `CommitIntent` — the dialog always opens on `CommitOnly` and the user switches inside the dialog. The dialog does need two booleans from the diff state model at open time:
- `allow_create_pr` — controls whether the third intent button is rendered.
- `has_upstream` — controls whether the middle intent button is "Commit and push" or "Commit and publish".
Both are passed through from `CodeReviewView::open_git_dialog` (which is the only caller) into `GitDialog::new_for_commit(repo, branch, allow_create_pr, has_upstream, ctx)` and then to `commit::new_state(repo, allow_create_pr, has_upstream, ctx)`. The commit dialog doesn't know about `DiffStateModel`; it receives exactly the scalars it needs.
### 5. Commit dialog confirm button: static "Confirm", no icon
`commit::confirm_label` / `confirm_icon` helpers are gone. `GitDialog::new_for_commit` calls `Self::build_dialog_buttons("Confirm", None, ctx)` directly. `build_dialog_buttons` now takes `Option<Icon>` and skips `.with_icon(...)` when `None`; push and pr still pass `Some(icon)`.
`handle_sub_action::SetIntent` updates only the segmented selector via `apply_intent_selector`; the confirm button's label/icon are untouched. The previous `confirm_button()` accessor on `GitDialog` is removed because nothing calls it anymore.
`loading_label(intent)` is replaced with a `const LOADING_LABEL: &str = "Committing…";`. Mid-flight copy is the same regardless of which chain is running. The success toast continues to be intent-aware via `CommitOutcome` and the existing per-outcome match in the spawn callback.
### 6. Commit dialog middle intent: push vs publish
`commit::new_state` computes `(push_label, push_icon)` from `has_upstream` and feeds those into `ActionButton::new(push_label, SecondaryTheme).with_icon(push_icon)`:
```rust path=null start=null
let (push_label, push_icon) = if has_upstream {
    ("Commit and push", Icon::ArrowUp)
} else {
    ("Commit and publish", Icon::UploadCloud)
};
```
The click handler still dispatches `CommitSubAction::SetIntent(CommitIntent::CommitAndPush)` — one enum variant covers both cases because `run_push` always uses `--set-upstream`.
### 7. `build_dialog_buttons` Option<Icon>
Signature changes from `(label, icon, ctx)` to `(label, Option<Icon>, ctx)`. Inside, it conditionally chains `.with_icon(...)`:
```rust path=null start=null
let mut button = ActionButton::new(confirm_label, SecondaryTheme)
    .with_size(ButtonSize::Small)
    .with_height(32.);
if let Some(icon) = confirm_icon {
    button = button.with_icon(icon);
}
button.on_click(|ctx| ctx.dispatch_typed_action(GitDialogAction::Confirm))
```
Commit passes `None`; push and pr pass `Some(...)`.
## End-to-end flow
### Commit-mode primary button click
```mermaid
sequenceDiagram
    participant User
    participant Header as code_review_view
    participant Dialog as GitDialog (Commit mode)
    User->>Header: click primary "Commit"
    Header->>Dialog: new_for_commit(repo, branch, allow_create_pr, has_upstream)
    Dialog->>Dialog: intent = CommitOnly; push label chosen from has_upstream
    Dialog->>User: show dialog with 3-button selector + "Confirm"
    User->>Dialog: pick "Commit and publish" (say, no upstream)
    Dialog->>Dialog: SetIntent(CommitAndPush); re-highlight segment
    User->>Dialog: click "Confirm"
    Dialog->>Dialog: set_loading("Committing…")
    Dialog->>Dialog: run_commit, then run_push (--set-upstream)
    Dialog->>Header: emit Completed
    Header->>User: "Changes committed and pushed." toast
```
### Dropdown interaction in Commit mode
User clicks chevron → menu opens with Commit (enabled), Publish (enabled if local commits, with UploadCloud icon), Create PR (enabled if upstream and not on main, else disabled, else PR#N link). Picking Push/Publish opens the push dialog directly; picking Commit opens the commit dialog.
### Why the chevron is hidden in Publish mode
Publish primary mode only fires when there's nothing to commit (primary would be Commit) and no upstream (primary would otherwise be Push, CreatePr, or ViewPr). In that state:
- Commit in a dropdown would be disabled (nothing to commit).
- Publish in the dropdown would be redundant with the primary.
- Create PR would be disabled (no upstream).
So the dropdown would be all-grey + one redundant row. The chevron hides to avoid the noise.
## Risks and mitigations
### Dropdown shape change may surprise users who relied on chained items
The "Commit and push" / "Commit and create PR" dropdown items are gone. Users who used them will now click Commit, pick the chained intent inside the dialog, and confirm. The chain itself still works. Mitigation: the dialog's intent selector is the same place the user would go to choose a chain anyway; surfacing it only in the dialog is the opinionated cleanup.
### `Publish` variant looks duplicative of `Push`
We explicitly kept them separate (see §3) to route chevron visibility through the enum. A future refactor could derive chevron visibility from a more compositional state, but that's bigger than the scope here.
### `has_upstream` is sampled at dialog-open time
If the user is in the commit dialog and the upstream is added/removed externally (unlikely during a dialog session), the "Commit and push" vs "Commit and publish" label won't update. `run_push` uses `--set-upstream` in both cases, so the action still works; only the label could be stale. Accepted.
### Regressions to the chain semantics
`start_confirm`'s match on `state.intent` (`CommitOnly` / `CommitAndPush` / `CommitAndCreatePr`) is unchanged; the only difference is that the user reaches the same intents via buttons inside the dialog rather than via two top-level actions.
## Testing and validation
Same manual validation as APP-3920 / APP-3922, plus:
- Open the dropdown across the five `PrimaryGitActionMode` states and verify the item shape matches the product spec's matrix.
- On a branch with no upstream, verify the commit dialog's middle intent is "Commit and publish" with the cloud icon, and that running that chain sets the upstream (`git rev-parse --abbrev-ref --symbolic-full-name @{u}` shows the new upstream).
- On a tracked branch, verify "Commit and push" is selected (middle label / icon).
- Verify the confirm button says "Confirm" and has no icon regardless of which intent is selected.
- Verify the loading label during all three chains reads "Committing…"; the success toast still differentiates outcomes.
- Verify Publish primary mode renders the primary button without a chevron.
- Grep to confirm `CodeReviewAction::CommitAndPush` and `CodeReviewAction::CommitAndCreatePr` are not referenced anywhere in `app/`.
No automated tests added — consistent with APP-3920 / APP-3922 which ship without tests, and the `git_dialog` module has no test harness yet.
## Follow-ups
- Consider collapsing `PrimaryGitActionMode::Push` and `Publish` back together if we introduce a generic "dropdown visibility" flag so the enum can stay semantic (same git action) rather than encoding UX surface differences.
- Add a `maybe_with_icon(Option<Icon>)` helper on `ActionButton` so the conditional in `build_dialog_buttons` collapses to one line; out of scope here.
- If `allow_create_pr` / `has_upstream` grow to more booleans plumbed through `open_git_dialog` → `new_for_commit` → `new_state`, bundle them into a `CommitDialogContext` struct to avoid signature churn.
