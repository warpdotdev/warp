# APP-3922: Create PR Dialog

## Summary

Add a "Create PR" dialog and a "Commit and create PR" flow to the code review panel, allowing users to create GitHub pull requests directly from the diff view. The dialog shows the branch name and a summary of all changes that will be included in the PR. The commit dialog gains a third intent option that chains commit → push → PR creation in one action.

## Problem

After APP-3920 (push/publish dialogs), the git operations button can reach the "Create PR" state (everything committed and pushed, no existing PR), but clicking it was a no-op. Similarly, the "Commit and create PR" dropdown option was stubbed out. Users had to leave the editor to create a PR on GitHub.

## Goals

- Provide a confirmation dialog before creating a PR that shows the target branch and aggregate change stats (files, additions, deletions).
- Allow expanding the changes section to see per-file stats with additions/deletions.
- Show loading state during PR creation with disabled controls.
- On success, show a toast with an "Open PR" link and refresh the git operations button to show "PR #N".
- On failure, close the dialog and surface a friendly error toast so the user can retry via the git operations button.
- Add "Commit and create PR" as a third intent in the commit dialog, chaining commit → push → `gh pr create --fill` in one operation.

## Non-goals

- Editing PR title, body, reviewers, or labels (uses `gh pr create --fill` which derives title/body from commits).
- Draft PR support.
- Checking whether `gh` CLI is installed/authenticated before opening the dialog (TODO for follow-up).

## Figma

https://www.figma.com/design/T2CtyXgIdjtrLfC03K1n1H/Code-review-2.0?node-id=6138-21140&m=dev

## User Experience

### Opening the dialog

The PR dialog opens when the user clicks:
- The "Create PR" primary action button (when in CreatePr mode — everything pushed, no existing PR, not on main).
- "Create PR" from the git operations dropdown menu.

The "Commit and create PR" flow opens the commit dialog with the `CommitAndCreatePr` intent selected instead.

Only one dialog can be open at a time.

### Dialog layout

The dialog is a centered modal overlay (460px wide) with a blurred background. It contains:

1. **Header**: Title "Create pull request" and a close button (X, with "ESC" tooltip).
2. **Branch section**: "Branch" label with a git branch icon and the current branch name.
3. **Changes section**: A bordered card showing aggregate stats (file count, +additions in green, -deletions in red) with a chevron to expand/collapse the per-file list.
4. **File list (expanded)**: Scrollable list (max 130px) of per-file rows showing filename, directory, and +/- stats. Files are loaded asynchronously when the dialog opens.
5. **Footer**: Cancel button and "Create PR" primary button.

### Loading state

When the user clicks "Create PR":
- The button label changes to "Creating…" and becomes disabled.
- The cancel button becomes disabled.
- The PR creation runs asynchronously via `gh pr create --fill`.

### Success

On success:
- The dialog closes.
- A toast appears: "PR successfully created." with an "Open PR" link that opens the PR URL in the browser.
- PR info is refreshed, which updates the git operations button to show "PR #N".

### Error

On failure:
- The dialog closes.
- An ephemeral toast appears with a friendly error message mapped from the raw git / `gh` error (e.g. "GitHub CLI (gh) not installed. See https://cli.github.com/.", "Authentication failed. Check your Git credentials.", or the generic "Git operation failed." fallback).
- The git operations button stays in `CreatePr` mode (since nothing changed), so the user can retry by clicking it again.

### Cancellation

The user can cancel via the Cancel button, the X button, or pressing ESC. Cancellation closes the dialog with no side effects.

### Commit and create PR flow

The "Commit and create PR" intent is shown in the commit dialog only when creating a PR would be meaningful — i.e. the branch has no existing PR and the user is not on the repo's main branch. In either of those cases the intent is hidden entirely (not just disabled); only "Commit" and "Commit and push" remain.

When the user selects "Commit and create PR" from the commit dialog:
1. The commit executes first.
2. On successful commit, the branch is pushed.
3. On successful push, `gh pr create --fill` runs.
4. On success, the commit dialog closes and the same "PR successfully created." toast with "Open PR" link appears.
5. On failure at any stage, an error toast is shown.

The commit dialog shows "Committing and pushing…" during the operation. No separate PR dialog is shown for this flow.

### Changes section data

The changes section shows the diff between the base (main) branch and `origin/{current_branch}`. If the remote ref doesn't exist yet (branch not pushed), it falls back to diffing against HEAD. This represents what would actually be included in the PR.

## Success Criteria

1. Clicking "Create PR" in the header opens a dialog showing the branch name and aggregate change stats.
2. Expanding the changes section shows per-file +/- stats.
3. Confirming PR creation shows a loading state, then closes the dialog and shows a success toast with "Open PR" link.
4. If PR creation fails, the dialog closes and a friendly error toast is shown; the header button stays in `CreatePr` mode so the user can retry.
5. "Commit and create PR" from the commit dialog chains commit → push → PR creation without opening the PR dialog.
6. After a successful PR creation, the git operations button updates to show "PR #N".
7. The dialog can be dismissed via Cancel, X, or ESC at any time (when not loading).

## Validation

- On a branch with everything pushed and no PR, click "Create PR", verify the dialog shows the correct branch and change stats.
- Expand changes and verify per-file list loads with correct stats.
- Confirm PR creation, verify loading state, success toast with link, and dialog dismissal.
- Simulate a failure (e.g. `gh` not authenticated) and verify the dialog closes, an error toast appears with appropriate copy, and the header button still allows retrying.
- Use "Commit and create PR" from the commit dialog and verify all three operations succeed with a single toast.
- Cancel the dialog via each method (Cancel, X, ESC) and verify no operation is performed.
- After PR creation, verify the header button shows "PR #N" and clicking it opens the PR URL.

## Open Questions

- Should we check for `gh` CLI availability/auth before opening the dialog?
- Should we support editing the PR title/body instead of using `--fill`?
- Should we support draft PRs?
