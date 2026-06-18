# APP-3919: Commit Dialog

Parent spec: `specs/APP-3918/PRODUCT.md`
Branch: `edward/commit-dialog`

## Summary

Add a commit dialog overlay to the code review panel that lets users compose and execute a git commit directly from the code review header, without switching to the terminal.

## Problem

The git operations button (from the parent branch) dispatches `OpenCommitDialog` but has no dialog to open. Users must switch to the terminal to run `git commit`. This breaks the inline workflow the button is designed to enable.

## User Experience

### Opening the dialog

The commit dialog opens when the user:
- Clicks the "Commit" primary button in the code review header
- Selects "Commit" from the chevron dropdown menu

The dialog renders as a centered modal overlay with a blurred background, consistent with the existing discard confirmation dialog.

### Dialog layout

The dialog ("Commit your changes") contains four sections:

1. **Branch** — displays the current branch name with a git-branch icon. Read-only.

2. **Changes** — shows a summary of changed files with aggregate stats (file count, +additions, -deletions).
   - A collapsible file list shows per-file stats (filename, directory, +/-, -)
   - "Include unstaged" toggle (on by default) controls whether unstaged + untracked changes are included or only staged changes
   - Toggling the switch reloads the file list from git

3. **Commit message** — a multi-line text editor with placeholder text "Leave blank to autogenerate a commit message".
   - Supports soft-wrap and autogrow
   - ESC dismisses the dialog
   - Editor is focused on open

4. **Intent selector** — currently only "Commit" button. Future: "Commit and push", "Commit and create PR".

### Footer

- Cancel button (left)
- Confirm "Commit" button (right) — disabled when no files or no commit message

### After committing

On success:
- Dialog closes
- A toast notification shows "Changes successfully committed."
- Diffs are reloaded to reflect the new state
- Diff metadata and PR info are refreshed

On failure:
- Dialog closes
- A toast shows the error message
- Diffs are reloaded (state may have partially changed)

### Closing the dialog

The dialog can be closed by:
- Clicking Cancel
- Clicking the X button
- Pressing ESC in the message editor

## Non-goals

- Staging individual files/hunks from the dialog (the "Include unstaged" toggle is all-or-nothing)
- Commit amend
- Compound actions (commit+push, commit+create PR) — stubs exist but are wired in child branches

## Success Criteria

1. Clicking "Commit" in the header opens the dialog overlay
2. The dialog shows the current branch, file changes with stats, and a message editor
3. Toggling "Include unstaged" refreshes the file list (staged-only vs all changes)
4. Confirming runs `git commit` with the entered message
5. Success/failure toasts appear after the commit
6. The code review panel refreshes to reflect post-commit state
7. ESC and Cancel close the dialog without committing
