# Product Spec: Wait for a file view opened by `warpctrl`

Issue: https://github.com/warpdotdev/Warp/issues/8741
Figma: none provided

## Problem

`warpctrl file open PATH` returns as soon as Warp accepts the request. Tools that
use an `$EDITOR`-style command therefore cannot wait for a user to finish
reviewing or editing a file.

## Goals

- Add `warpctrl file open PATH --wait`, comparable to `code --wait`.
- Wait for the exact code-editor tab or Markdown pane selected by the command.
- Preserve existing behavior, output, arguments, and targeting without
  `--wait`.
- Let callers cancel safely without changing the Warp view.

## Non-goals

- A separate wait action or public file-view identifier.
- New editor, viewer, layout, split, sizing, save, or load behavior.
- Reporting whether the user saved, discarded, or closed an unchanged file.
- Other automation requested in issue #8741 or changes to editor clients.

## Behavior invariants

1. Without `--wait`, `file open` keeps its current behavior and returns the
   existing `file.open` acknowledgement immediately.

2. With `--wait`, Warp uses the existing path, line, column, layout, Markdown,
   instance, and target-selector logic, then waits for the exact logical view
   that logic creates or focuses.

3. If Warp focuses an already-open view, that view becomes the target. Closing
   another view of the same path does not satisfy the wait.

4. Reordering or moving the target among panes, workspace tabs, or windows does
   not complete the wait. If a move merges it into an already-open view of the
   same path and removes the original logical view, the wait completes.

5. The wait succeeds when the target leaves the visible layout, whether closed
   directly or through its containing pane, workspace tab, or window. Warp's
   undo-close grace period does not delay completion.

6. For unsaved code-editor tabs, Save or Discard completes the wait once the tab
   closes. Cancel leaves both the view and the wait active. The response does
   not report which choice the user made.

7. Multiple processes may wait on the same view. They all complete when it
   closes; canceling one does not affect the view or other waiters.

8. Ctrl+C stops only the calling CLI process. The view remains open with its
   current content and unsaved state.

9. If the selected Warp process, Warp Control server, or transport exits before
   a normal close, the CLI returns a nonzero transport or bridge error.

10. Success remains the existing `file.open` acknowledgement in text and
    structured output. `--wait` changes only when it is returned.

11. Existing file semantics remain unchanged. Paths treated as new files remain
    editable and saveable, Markdown load errors keep their current UI behavior,
    and `--wait` adds no existence or readability preflight.
