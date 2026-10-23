# Product Spec: Reload File Tree action in Command Palette

**Issue:** [warpdotdev/warp#10003](https://github.com/warpdotdev/warp/issues/10003)

## Summary

Add a Command Palette action — `Reload File Tree` — that forces Warp's File Tree (Wildtree) to re-scan the filesystem for the active project/session. The automatic refresh path is unchanged; this is a deliberate, explicit fallback for the cases where the tree drifts out of sync with disk (the issue cites filesystem changes in directories that lack a `.git` repo, where filesystem watchers may miss certain changes).

Adding this is a small, low-risk addition that gives users a recovery path without introducing persistent UI chrome (a refresh button on the tree itself would be heavier and visually noisier per the issue's analysis).

## Problem

Per the issue: a user reported that after running a CLI command that changed the filesystem (specifically, deleting a visible directory), the File Tree did not update to reflect the deletion. The report calls out that this surfaces in directories *without a `.git` repo*. Warp's automatic refresh is normally adequate, but when it isn't, there's currently no obvious recovery path short of closing and reopening the workspace.

The user-stated workaround would be reloading Warp itself, which is disproportionate — the application is fine; only the File Tree's view of disk is stale.

## Goals

- A user with a stale File Tree can open the Command Palette, type a few characters of "reload", select the action, and see the tree reflect current disk contents.
- The action works regardless of whether the active directory is a Git repository.
- The automatic refresh path is unchanged. This is a fallback, not a replacement.
- The action is discoverable through the same surface as every other Command Palette action — no new keybinding, no new menu entry, no new persistent UI.

## Non-goals (V1 — explicitly deferred to follow-ups)

- **Investigating and fixing the underlying automatic-refresh failure.** The issue notes the root cause is *"still uncertain"* and the action is needed *"regardless"*. V1 ships the recovery action; the watcher gap is a separate concern tracked elsewhere.
- **A visible Refresh button on the File Tree itself.** Explicitly rejected in the issue body — *"would likely feel too heavy or janky for the file tree."*
- **A keyboard shortcut bound by default.** Power users can rebind via the existing keybindings system once the action exists. V1 ships the action and lets the binding emerge from usage.
- **Reload-on-focus or reload-on-window-activation.** Would be a behavior change rather than an explicit user action; out of scope.
- **A reload action scoped to a specific subtree.** V1 reloads the whole tree for the active session; per-folder reload is a follow-up if usage shows it's wanted.

## User experience

### Invoking the action

1. User notices the File Tree is out of sync with disk (e.g. after a `rm -rf old-dir/` that doesn't reflect in the tree).
2. User opens the Command Palette (`Cmd+P` / `Ctrl+P`).
3. User types `reload file tree`, `refresh file tree`, or `reload wildtree`. All three queries match the same action via aliasing in the search index.
4. Selecting the action triggers a tree re-scan from disk for the active session's project root. The tree visibly updates within one frame.

### Action visibility

- The action is always present in the palette when there is an active File Tree (i.e. the user has a workspace open). It is *not* gated on "looks like the tree is stale" — the user is the source of truth on whether the tree looks right; the action is always available when a tree exists to reload.
- The action is omitted from the palette when no project / session is active (palette would have nothing to reload).

### Result feedback

- A small, non-blocking toast confirms the reload finished: *"File tree reloaded."* with a millisecond duration shown for users who want to see it work (e.g. *"File tree reloaded (47 ms)."*). The toast is dismissible and auto-dismisses after 2 seconds.
- If the reload fails (e.g. the project directory was unmounted between palette-open and selection), the toast surfaces the failure: *"File tree reload failed: `<reason>`"* and the existing tree state is preserved (no flash of empty tree).

## Configuration shape

No new settings, no new on-disk artefacts. The action consumes the existing project / session state to know what to reload.

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. With a workspace open, the Command Palette filtered for `reload file tree` shows the action.
2. With no workspace open, the action does not appear in the palette.
3. Selecting the action triggers a re-scan that picks up files that were added on disk after the tree was last updated, and removes files that were deleted on disk after the tree was last updated.
4. The action works on directories that are not Git repositories — verified by a test that operates on a project directory created via `mkdir -p` with no `.git` subdirectory.
5. After the reload completes, a non-blocking toast appears confirming success. The toast auto-dismisses within 2 seconds.
6. If the reload fails (e.g. the project directory has been unmounted), the toast surfaces the failure and the tree's existing state is preserved.
7. Aliases `refresh file tree` and `reload wildtree` resolve to the same action as `reload file tree` in the palette search index.
8. Selecting the action emits a telemetry event `FileTreeReloadInvoked` with the duration of the re-scan in milliseconds.

## Open questions

- **Action label.** "Reload File Tree" vs "Refresh File Tree" vs "Reload Wildtree". The issue uses all three interchangeably. Recommend "Reload File Tree" as the canonical label (matches the user-facing terminology in the docs) with the other two as aliases for search.
- **Toast feedback.** Show duration always, or only when ≥ 100 ms (suppress noise on instant reloads of small trees)? Recommend always-shown so the user gets unambiguous confirmation; suppress only if maintainer feedback signals it's noisy.
- **Per-tree-pane reload.** If a workspace has multiple panes each with their own File Tree (does it?), does this action reload all of them or only the focused one? Recommend the focused pane's tree only; the others can be reloaded individually.
