# APP-3920: Push and Publish Dialogs

## Summary

Add push and publish dialog overlays to the code review panel, allowing users to review and confirm push/publish operations without leaving the diff view. The push dialog shows the branch and included commits before pushing. The publish dialog reuses the same UI with adjusted labels for first-time branch publication (setting upstream tracking).

## Problem

The git operations button (APP-3918) surfaces the correct primary action in the header, and the commit dialog (APP-3919) handles committing. However, pushing and publishing still have no dedicated confirmation flow — clicking "Push" or "Publish" needs to run the operation with a clear preview of what will be pushed and appropriate loading/error states.

## Goals

- Provide a confirmation dialog before pushing that shows the target branch and the list of commits that will be pushed.
- Allow expanding individual commits to see per-file change stats (files changed, additions, deletions).
- Show loading state during the push operation with disabled controls.
- Display success/error toasts after the operation completes.
- Reuse the same dialog for "Publish" (first push to set upstream) with appropriately different labels and icon.
- Wire the "Commit and push" intent from the commit dialog so that a push is automatically chained after a successful commit.

## Non-goals

- Selecting or deselecting individual commits to push (all unpushed commits are always included).
- Force push or other advanced push options.
- The Create PR dialog (handled separately).

## Figma

https://www.figma.com/design/T2CtyXgIdjtrLfC03K1n1H/Code-review-2.0?node-id=6138-21140&m=dev

## User Experience

### Opening the dialog

The push dialog opens when the user clicks:
- The "Push" primary action button (when in Push mode).
- "Push" from the git operations dropdown menu.
- The "Publish" primary action button (when in Publish mode, i.e. no upstream tracking branch).

Only one push/publish dialog can be open at a time. If one is already open, the action is ignored.

### Dialog layout

The dialog is a centered modal overlay (460px wide) with a blurred background. It contains:

1. **Header**: A title ("Push changes" or "Publish branch") and a close button (X, with "ESC" tooltip).
2. **Branch section**: Shows "Branch" label with a git branch icon and the current branch name.
3. **Commits section**: Shows "Included commits" label followed by a scrollable list (max 300px) of commit cards. Each card shows:
   - Commit subject (single line, no wrap)
   - Stats: file count, additions (green), deletions (red)
   - A chevron to expand/collapse the commit's file list
4. **File list (expanded)**: When a commit is expanded, shows per-file rows with filename, directory path, and +/- stats. Files are loaded on demand when the commit is first expanded, with a "Loading…" placeholder.
5. **Footer**: Cancel button and the primary action button ("Push" or "Publish").

### Loading state

When the user clicks the primary action button:
- The button label changes to "Pushing…" or "Publishing…" and becomes disabled.
- The cancel button remains visible but clicking it is ignored while the operation is in progress.
- The push operation runs asynchronously.

### Success

On success:
- The dialog closes.
- A toast appears: "Changes successfully pushed." or "Branch successfully published."
- Diff metadata and PR info are refreshed, which updates the git operations button state.

### Error

On failure:
- The dialog stays open.
- The button reverts to its original label and becomes enabled again.
- The cancel button becomes functional again.
- A toast shows the error message.

### Cancellation

The user can cancel via the Cancel button, the X button, or pressing ESC. Cancellation closes the dialog with no side effects. Cancel is blocked while a push is in progress.

### Commit and push flow

When the user selects "Commit and push" from the commit dialog (APP-3919), the commit executes first. On success, a push is chained automatically — no separate push dialog is shown. The commit dialog shows "Committing and pushing…" during the operation. On success, a single "Changes committed and pushed." toast appears. On failure at either stage, an error toast is shown.

### Commit files loading

Per-commit file lists are fetched lazily via `git diff-tree --numstat`. Each file entry includes path, additions, and deletions. The data is cached per commit hash for the lifetime of the dialog.

## Success Criteria

1. Clicking "Push" in the header opens a dialog showing the branch name and all unpushed commits with stats.
2. Expanding a commit shows its changed files with per-file +/- stats.
3. Confirming the push shows a loading state, then closes the dialog and shows a success toast on completion.
4. If the push fails, the dialog remains open with an error toast and the button re-enables.
5. Clicking "Publish" opens the same dialog with "Publish branch" title and "Publish" button.
6. A successful publish shows "Branch successfully published." toast.
7. "Commit and push" from the commit dialog chains commit → push without opening the push dialog.
8. The dialog can be dismissed via Cancel, X, or ESC at any time (when not loading).
9. After a successful push or publish, the git operations button updates to reflect the new state (e.g. switches to "Create PR").

## Validation

- Open a repo with unpushed commits, click "Push", verify the dialog shows the correct branch and commits.
- Expand a commit and verify file list loads with correct stats.
- Confirm push, verify loading state, success toast, and dialog dismissal.
- Simulate a push failure (e.g. network issue) and verify the error toast and button recovery.
- On a branch with no upstream, verify "Publish" opens the dialog with publish-specific labels.
- Use "Commit and push" from the commit dialog and verify both operations succeed with a single toast.
- Cancel the dialog via each method (Cancel, X, ESC) and verify no operation is performed.

## Open Questions

- Should "Commit and push" show a separate push confirmation, or is the current chained behavior (no intermediate dialog) correct?
