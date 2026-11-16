# APP-3830: Worktree Sidecar Selection Semantics

Linear: `APP-3830` (inferred from the branch name)
Related spec: `specs/moirahuang/APP-3743/PRODUCT.md`

## Summary

Polish the `New worktree config` sidecar so the repo that looks active is always the repo that executes. Mouse hover should take precedence over stale keyboard selection, repo clicks should consistently open a worktree, and keyboard confirmation from the search field should use the same visible selection model.

## Problem

The worktree sidecar currently mixes two concepts of "active repo":

- keyboard selection, which is initialized so the search field can drive arrow-key navigation
- hover state, which can move to a different repo row without updating the underlying actionable selection

This creates confusing behavior:

- two different repo rows can appear active at once
- clicking a hovered repo can no-op or execute from stale state
- `Enter` from the search field and mouse click do not reliably resolve the same repo

The sidecar is intended to be a fast path for worktree creation, so ambiguity in selection makes the feature feel flaky and untrustworthy.

## Goals

- The hovered repo row becomes the active selection as soon as the user moves the mouse over it.
- Mouse click and keyboard confirmation resolve the same repo the user sees as active.
- The search field remains keyboard-focused without becoming an actionable repo selection.
- `+ Add new repo` does not accidentally execute a previously selected repo.
- Closing behavior remains predictable: selecting a repo closes both the sidecar and the parent menu.

## Non-goals

- Redesigning the unified new-session menu layout.
- Changing how the default worktree tab config is authored or executed.
- Reworking all Warp menus to use a new selection model.
- Removing the search field or the pinned `+ Add new repo` footer.

## Figma / design references

Figma: none provided

## User experience

### Opening the sidecar

When the user opens the `New worktree config` sidecar:

- the search field is focused
- the first actionable repo row is selected if at least one repo is available
- if no repos are available, the search field and `+ Add new repo` footer still render and no repo row is selected

### Keyboard behavior

While the search field is focused:

- `Down` moves selection to the next actionable repo row
- `Up` moves selection to the previous actionable repo row
- `Enter` opens a worktree for the currently selected repo
- if no repo is selected but actionable repo rows exist, `Enter` first resolves the first actionable repo and opens it
- if no actionable repo rows exist, `Enter` does nothing
- `Escape` closes the sidecar and parent menu as it does today

### Hover precedence

If the user moves the mouse over a repo row:

- that hovered repo row becomes the active selection immediately
- any previously selected repo row stops being the effective active selection
- subsequent click or keyboard confirmation uses the hovered repo row unless the user changes selection again

Hover precedence only applies to actionable rows. Non-actionable rows such as the search row do not replace the current repo selection.

### Mouse click behavior

Clicking a repo row:

- opens a worktree for that repo using the default worktree tab config
- closes the sidecar
- closes the parent new-session menu

The clicked row should never no-op because of responder-chain routing or stale sidecar state.

### Search behavior

Typing in the search field filters repo rows live.

When filtering changes the available repo list:

- the filtered repo list updates immediately
- if actionable rows remain, the first actionable repo row becomes selected
- if no actionable rows remain, repo selection is cleared

### Add new repo footer

The pinned `+ Add new repo` footer remains visible while the repo list scrolls.

Clicking it:

- opens the folder picker
- does not open a worktree for any repo
- does not reuse stale repo selection as part of the click handling

## Success criteria

- At most one actionable repo row is treated as active at a time.
- Hovering a repo row updates the underlying actionable selection, not just the visual hover state.
- Clicking a repo row consistently opens a worktree for that repo.
- `Enter` from the search field opens the same repo the UI currently presents as active.
- The search field does not steal actionable selection when hovered.
- `+ Add new repo` opens the picker without also opening a repo.
- The sidecar closes cleanly after repo selection and leaves no stale selection state behind.

## Validation

- Unit-test hover precedence by starting with one selected repo, hovering a different repo row, and asserting the selected row changes to the hovered row.
- Unit-test the close-via-select path to verify sidecar repo selection executes from `Workspace`.
- Unit-test keyboard confirmation from the search editor to verify `Enter` opens the selected repo and closes the menu.
- Manual validation:
  - open `New worktree config`
  - move selection with arrow keys
  - hover a different repo row and confirm the active row updates
  - click the hovered repo and verify the correct worktree tab opens
  - click `+ Add new repo` and verify only the picker opens

## Open questions

None currently.
