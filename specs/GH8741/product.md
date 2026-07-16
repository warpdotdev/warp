# Product Spec: Wait for a file view opened by `warpctrl`

Issue: https://github.com/warpdotdev/Warp/issues/8741
Figma: none provided

## Summary

Add an opt-in `--wait` flag to `warpctrl file open PATH`. Without the flag, the
command keeps its current nonblocking behavior. With the flag, it opens or
focuses the same in-Warp file view that `file open` selects today and keeps the
CLI process running until that exact logical view is closed.

This gives tools that honor an `$EDITOR`-style command a simple synchronization
contract comparable to `code --wait`, while keeping the public Warp Control
surface limited to the existing `file.open` action.

## Problem

`warpctrl file open PATH` can open a file in Warp's code editor or Markdown
viewer, but it acknowledges the request immediately. A caller that needs a
human to review or edit a temporary file cannot tell when that interaction is
finished, so it cannot safely resume and read the file back.

## Goals

- Support `warpctrl file open PATH --wait`.
- Wait for the exact code-editor tab or Markdown viewer pane selected by that
  invocation, including an already-open view that Warp focuses instead of
  duplicating.
- Preserve the existing behavior and output of `warpctrl file open PATH` when
  `--wait` is absent.
- Compose with the existing file-open arguments and target selectors.
- Keep cancellation safe: stopping the waiting CLI must not close or otherwise
  mutate the Warp view.

## Non-goals

- A separate public `warpctrl file wait` action.
- Public file-view IDs, handles, inspection commands, or lifecycle APIs.
- New file mode, viewer/editor, layout, split-direction, or sizing flags.
- Reporting whether a view was saved, discarded, or closed unchanged.
- Changing file-load, new-file, save, or unsaved-changes behavior.
- Adding terminal-command execution, general pane automation, or the other
  capabilities requested in issue #8741.
- Changes to Oh My Pi or any other editor client. Once Warp supports `--wait`,
  clients can opt into it through their existing editor-command configuration.

## Behavior

1. `warpctrl file open PATH` continues to open or focus the file and return the
   existing `file.open` acknowledgement without waiting for the user to close
   the view.

2. `warpctrl file open PATH --wait` opens or focuses the file using the same
   target, layout, Markdown preference, line, column, and selector resolution
   as `file open` does today, then blocks until the exact logical file view
   selected by that invocation is closed.

3. If Warp creates a new code-editor tab or Markdown viewer pane, that new view
   is the wait target. If the file is already open in the selected workspace and
   Warp focuses it, that existing view is the wait target. Another view showing
   the same path must not accidentally satisfy the wait.

4. Reordering the target within its current tab or pane group, or moving the
   target to another pane, workspace tab, or window, does not complete the
   command. If a move merges the target into a different already-open view of
   the same path and removes the original logical view, the original target is
   considered closed and the wait completes.

5. Closing the target code-editor tab or Markdown viewer pane completes the
   command successfully. Closing a containing pane, workspace tab, or window
   also completes the command when it removes the target view. Warp's
   undo-close grace period does not delay completion after the view has left the
   visible layout.

6. For a code-editor tab with unsaved changes, choosing Save or Discard and
   completing the close resolves the wait successfully. Canceling the close
   leaves both the view and the CLI wait active. The successful response does
   not classify which choice the user made.

7. Multiple `file open --wait` processes may wait on the same already-open
   logical view. Each process completes when that view closes; canceling one
   waiter does not affect the others.

8. Interrupting the CLI with Ctrl+C stops only that CLI process and its pending
   request. The file remains open in Warp with its current content and unsaved
   state.

9. If the selected Warp process, Warp Control server, or local transport exits
   before the target is normally closed, the CLI returns a nonzero transport or
   bridge error rather than reporting a successful close.

10. The success payload remains the existing `file.open` acknowledgement in
    text and structured output modes. `--wait` changes when that acknowledgement
    is returned, not its schema.

11. `--wait` composes with `--line`, `--column`, `--new-tab`, `--instance`,
    `--pid`, and the existing window/tab/pane/session selectors. Their parsing,
    validation, and targeting behavior is unchanged.

12. Existing file semantics remain unchanged. In particular, a path that the
    code editor currently treats as a new file is still editable and saveable;
    Markdown load errors still use the existing viewer behavior; and `--wait`
    does not add a new preflight existence or readability check.

## Success criteria

1. An `$EDITOR`-style command can run `warpctrl file open temporary.md --wait`,
   let the user edit and save in Warp, and resume only after the selected Warp
   view closes.
2. The same command without `--wait` remains nonblocking and wire-compatible.
3. Waiting follows the exact selected view through ordinary moves and cannot be
   completed by closing an unrelated duplicate path.
4. Save, discard, cancel, Ctrl+C, multiple waiters, and Warp shutdown have the
   behavior defined above.
5. No new public Warp Control action or file-view identifier is introduced.
