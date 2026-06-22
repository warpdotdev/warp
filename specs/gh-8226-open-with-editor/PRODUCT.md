# gh-8226: Open with $EDITOR from tab and file-tree context menus

## Summary

Add an "Open with <Editor>" item to two existing right-click context menus —
session tabs and file-tree directories — that launches the user's configured
external editor on the directory in question. The menu label is rendered from
the editor that the user has already chosen in `Settings → Code → External
editor`; no editor is hardcoded.

## Problem

[Issue #8226](https://github.com/warpdotdev/warp/issues/8226) asks for a way to
open a directory in `$EDITOR` from Warp. Today Warp's "Open in editor" plumbing
([docs](https://docs.warp.dev/terminal/more-features/files-and-links)) only
opens *files*. There is no fast path from "this tab represents a project I'm
working on" or "this is a folder in the file tree" to "open this folder in my
editor."

Users currently fall back to typing `code .`, `cursor .`, or alt-tabbing into
the editor and using its recent-projects list. Both break flow.

The original issue's screenshot shows the request applied to a path printed in
terminal output (CLI agent output). That is a third entry point with its own
hit-testing and disambiguation cost; this spec does **not** attempt it. A
prompt-area "Open in IDE" chip explored by @ATERCATES on a fork (see
[issue #8226](https://github.com/warpdotdev/warp/issues/8226) discussion)
is also out of scope here. The three entry points are complementary and can
ship independently; this spec covers only the two right-click menus.

## Non-goals

- Hijacking clicks on directory paths printed inside terminal output (the
  original #8226 request). Tracked separately.
- Adding an "Open in IDE" chip to the prompt area (ATERCATES's prototype on his
  fork).
- Adding the menu item to the Warp Drive workspace switcher
  (`app/src/drive/index.rs:2466 render_workspace_picker`). Warp Drive workspaces
  are cloud entities without a local filesystem path; "Open in editor" has no
  defined target there.
- Supporting remote / SSH paths in this iteration. The file tree's
  `is_remote_item` branch and SSH-backed terminal sessions are deferred.
- Editor selection UX. The user's existing
  `code.editor.open_file_editor` setting is read as-is; no new picker, no new
  settings surface, no per-tab override.
- Line/column positioning. Directories don't have a meaningful caret; the
  existing `LineAndColumnArg` plumbing for files is irrelevant.
- Changing the menu when the configured editor is `SystemDefault`, `Warp`, or
  `EnvEditor` — see Behavior 4 / 5 for what's shown in those modes.

## Figma

None provided. The visible change is a single additional `MenuItem` inserted
into two existing menus; no layout, color, or icon changes.

## Behavior

1. **Session tab right-click menu (horizontal and vertical tab bar).** When
   the user right-clicks a tab whose pane has a resolvable local current
   working directory, the menu shows an additional item labeled
   "Open with <Editor display name>" (e.g. "Open with VS Code", "Open with
   Cursor"). The item is grouped with `modify_tab_menu_items` — i.e.
   immediately after the existing "Move to group / Pin / Rename" cluster and
   before the close-tab section. Selecting it launches the configured editor
   with the tab's cwd as the target.

2. **File-tree directory right-click menu.** When the user right-clicks a
   `DirectoryHeader` in the file tree, an item labeled "Open with <Editor
   display name>" is inserted immediately after the existing "Reveal in
   Finder" / "Reveal in Explorer" / "Reveal in file manager" item. Selecting
   it launches the configured editor with the directory's absolute path as
   the target.

3. **Menu label is data-driven.** The editor display string comes from
   `Editor::Display` (`app/src/util/file/external_editor/mod.rs`). The label
   updates the next time the menu is opened after the user changes the
   `code.editor.open_file_editor` setting; an open menu is not mutated mid-
   flight.

4. **`EditorChoice::ExternalEditor(_)` is the only case that shows a named
   editor.** When the setting is `SystemDefault`, `Warp`, or `EnvEditor`, the
   item label falls back to "Open in editor", and selecting it uses the
   corresponding fallback behavior:
   - `SystemDefault` → platform `open` / `xdg-open` / `start` on the
     directory.
   - `Warp` → no-op: directories do not open inside Warp's built-in editor.
     The menu item is omitted in this mode.
   - `EnvEditor` → spawn `$EDITOR` with the directory path as the single
     argument. If `$EDITOR` is empty or unset, the item is omitted.

5. **No menu item when there is no usable target.** The item is omitted (not
   shown disabled) when any of the following hold:
   - The tab's pane group has no resolvable cwd (e.g. SSH-only session
     without a synced local path, agent-only tabs).
   - The file-tree item is a remote entry (`is_remote_item == true`), matching
     the existing "Reveal in Finder" treatment that already excludes these.
   - The configured editor is `Warp` (per Behavior 4), or `EnvEditor` with no
     `$EDITOR`.

6. **Feature flag gate.** Both menu items are gated behind a single
   `FeatureFlag::OpenDirectoryInExternalEditor`. When the flag is off, neither
   menu changes — current behavior is preserved exactly.

7. **Failure modes are non-blocking.** If the editor launch fails (binary not
   found, permission denied, editor returns non-zero), Warp logs a warning
   via the existing external-editor error path but does not surface a modal
   or block the menu close. The same handling that exists today for
   `open_file_path_in_external_editor` is reused; this spec adds no new
   user-facing error UI.

8. **Telemetry is unchanged.** No new analytics events. If existing
   external-editor events are emitted for file opens, the same event family
   is reused for directory opens with a `target_kind = "directory"` field
   added (see TECH.md).

## Validation

- A tab with cwd `/Users/x/code/foo`, editor setting `ExternalEditor(VSCode)`:
  right-click tab → menu shows "Open with VS Code" between the modify-tab and
  close-tab sections; selecting it spawns VS Code on `/Users/x/code/foo`.

- Same tab, editor setting changed to `ExternalEditor(Cursor)`: reopen the
  menu, label is now "Open with Cursor".

- Same tab, editor setting is `Warp`: menu does not contain the item.

- Same tab, editor setting is `EnvEditor` with `EDITOR=zed`: label is "Open in
  editor"; selecting it spawns `zed /Users/x/code/foo`.

- File tree root directory of an indexed local repo: right-click → menu shows
  "Open with <Editor>" immediately after "Reveal in Finder".

- File tree root directory of a remote (SSH-backed) repo: menu does not
  contain the item (matches existing "Reveal in Finder" treatment).

- Feature flag off (`OpenDirectoryInExternalEditor = false`): tab and file-
  tree menus are byte-for-byte unchanged from current master.
