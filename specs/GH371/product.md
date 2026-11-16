# PRODUCT.md — Restore Open Files and Markdown Editors

**GitHub Issue:** [warpdotdev/warp-external#371](https://github.com/warpdotdev/warp-external/issues/371)
**Figma:** none provided

## Summary

When Warp is closed and restarted, code editor panes (open files in the built-in editor) and markdown viewer/editor panes should be restored to their previous state, just as terminal tabs, AI conversations, notebooks, and other pane types already are. Today, code editor panes are persisted to SQLite but skipped during restoration, and while markdown file panes are successfully restored, the overall experience for file-based panes has gaps.

## Problem

Users who open files in Warp's built-in code editor lose all open editor tabs when they quit and relaunch the app. Those panes are silently dropped during session restoration, which is surprising given that terminal sessions and other pane types are reliably restored. This forces users to manually reopen files, losing their working context.

## Goals

1. **Restore code editor panes on restart.** Every code pane that was open when the app was quit should be restored to the same position in the pane tree with the same file(s) open.
2. **Restore all tabs within a code pane.** A code pane can contain multiple file tabs. All open tabs (and the active tab index) should be restored, not just the single active file.
3. **Preserve markdown viewer display mode.** When a markdown file is open in the `FileNotebookView` (rendered mode), it should restore to the correct display mode (rendered vs. raw).
4. **Handle missing files gracefully.** If a persisted file path no longer exists on disk at restoration time, the pane should still be restored (e.g. showing an error state or empty editor) rather than causing the entire tab/pane-tree restoration to fail.
5. **Preserve code pane source semantics where possible.** When restoring a file-backed code pane, preserve enough source information to retain the expected deduplication behavior for that pane (for example, restoring file-tree panes as `CodeSource::FileTree` rather than collapsing everything to `CodeSource::Link`).

## Non-goals

- **Restoring unsaved edits / dirty buffer content.** This spec does not cover persisting uncommitted text changes in editor buffers. Only the file path and tab structure are restored; the editor reloads content from disk.
- **Restoring scroll position or cursor location.** Precise viewport state within a file is out of scope for this change.
- **Restoring code diff panes.** `CodeDiff` panes are tied to ephemeral AI actions and are not meaningful to restore across sessions.
- **Restoring code review panes.** Code review panes already have their own restoration path and are out of scope.
- **Remote file restoration.** Only locally-accessible files are restored. Remote/SSH file paths are skipped.

## User Experience

### Code editor panes

1. **Quit with open code panes.** The user has one or more code editor panes open, some containing multiple file tabs. When the user quits Warp (Cmd+Q or window close), the state of each code pane is persisted, including:
   - The list of open file paths (in tab order).
   - Which tab is active.
   - Whether each tab is a preview tab.
2. **Restart and restore.** On relaunch, each persisted code pane reappears in the same position in the tab/pane tree. All file tabs within the pane are reopened. The previously active tab is focused.
3. **File no longer exists.** If a file was deleted between quit and relaunch, the corresponding tab is still created but the editor shows its standard file-not-found / error state. Other valid tabs in the same pane are unaffected.
4. **Code pane with no valid file.** If a code pane was open to a file that can no longer be located, the pane is restored showing an empty/error state rather than being dropped entirely.
5. **Deduplication.** If the same file was open in two different code panes (in different tabs), both panes are restored independently. The `CodeManager` already handles deduplication within a single pane group — this behavior is preserved.
6. **Reopening from the same surface.** If a restored pane originally came from the file tree, reopening that same file from the file tree should focus the restored pane rather than create a duplicate. The restored pane should preserve the source behavior needed for that workflow.

### Markdown file panes

1. **Already restored.** Markdown files opened in `FileNotebookView` (the rendered markdown viewer) are already persisted and restored via the `NotebookPaneSnapshot::LocalFileNotebook` path. No change is needed for basic restoration.
2. **Display mode.** The markdown display mode (rendered vs. raw) should be preserved. If it was in "raw" (code editor) mode, it should reopen in raw mode, and vice versa.

### Edge cases

- **Empty code pane (no file open).** A code pane with no file loaded (e.g. a new unsaved buffer with no path) is not restored. This matches the existing behavior for empty notebook panes.
- **Binary files.** If a persisted path points to a binary file, the editor shows its standard binary-file handling. This is existing behavior and unchanged.
- **Permissions errors.** If a file exists but cannot be read, the editor shows its standard error state. This is existing behavior and unchanged.

## Success Criteria

1. When the user quits Warp with one or more code editor panes open and relaunches, all code panes are restored in the correct position within the pane tree (correct window, correct tab, correct split position).
2. Multiple file tabs within a single code pane are all restored, in the correct order, with the correct active tab.
3. Markdown file panes (`FileNotebookView`) continue to be restored correctly (this is already working and must not regress).
4. A code pane whose persisted file path no longer exists on disk is still restored (showing an error/empty state) rather than being silently dropped.
5. The pane tree structure is not corrupted by code pane restoration failures — a single invalid file path does not prevent other panes from being restored.

## Validation

1. **Manual test — single code pane, single file.** Open a file in the code editor, quit Warp, relaunch. Verify the file is reopened in the same tab/pane position.
2. **Manual test — multi-tab code pane.** Open 3 files in the same code pane (as tabs), set the 2nd tab as active, quit, relaunch. Verify all 3 tabs are restored with the 2nd tab active.
3. **Manual test — split pane with code + terminal.** Have a horizontal split with a terminal on the left and a code editor on the right, quit, relaunch. Verify both panes are restored in the correct split positions.
4. **Manual test — deleted file.** Open a file, quit, delete the file from disk, relaunch. Verify the code pane still appears (with an error state for the missing file) and does not crash the restoration.
5. **Manual test — markdown viewer.** Open a markdown file in the rendered markdown viewer, quit, relaunch. Verify the file is shown in the same display mode.
6. **Unit/integration tests.** The snapshot→restore round trip for `CodePaneSnapShot` should be covered by tests that serialize a code pane snapshot and verify the code pane is created on restoration.

## Open Questions

1. **Should preview tabs be restored?** Preview tabs (file tree single-click) are ephemeral by design. This spec proposes restoring them as preview tabs to preserve user context, but an alternative is to drop preview-only tabs on restoration to keep the session clean.
