# gh-8226: Tech Spec

## Context

See `PRODUCT.md` for user-visible behavior.

The feature wires a new menu item into two existing right-click context menus
and adds a single platform-dispatched "open this directory in editor X"
operation. There is no new view, no new settings surface, and no schema
change.

Anchors in current code:

- `app/src/util/file/external_editor/mod.rs:293
  open_file_path_in_external_editor` — existing entry point. Reads
  `EditorSettings::open_file_editor`, extracts the `Editor` from
  `EditorChoice::ExternalEditor(_)`, then calls
  `open_file_path_with_editor` which `cfg_if!` dispatches to
  `mac::open_file_path_with_line_and_col`,
  `linux::open_file_path_with_line_and_col`, or
  `windows::open_file_path_with_line_and_col`. **This path opens files only;
  there is no equivalent for directories today.**
- `app/src/util/file/external_editor/{mac,linux,windows}.rs` — per-platform
  implementations. Each currently takes a `LineAndColumnArg` and a file
  `&Path`. We add a sibling function that takes a directory path only.
- `app/src/util/file/external_editor/settings.rs:21 EditorChoice` — the
  setting variant the menu reads. No change to the enum or the underlying
  TOML schema; the existing `code.editor.open_file_editor` value drives the
  new directory open as well.
- `app/src/util/file/external_editor/mod.rs:60-148 Editor` enum and its
  `Display` impl — the source of the menu label's human-readable editor
  name.
- `app/src/tab.rs:213 menu_items_with_pane_name_target` — composes the tab
  context menu by chaining section methods (`pin_menu_items`,
  `tab_group_menu_items`, `session_sharing_menu_items`,
  `copy_metadata_menu_items`, `modify_tab_menu_items`,
  `close_tab_menu_items`, `save_config_menu_items`,
  `color_option_menu_items`). The new menu item is added via a new
  section method `open_in_editor_menu_items`, slotted between
  `modify_tab_menu_items` and `close_tab_menu_items`.
- `app/src/tab.rs:1134-1146` — established pattern for reading the active
  session's working directory from a `Tab`:
  `tab.pane_group.as_ref(ctx).focused_session_view(ctx)` →
  `.model.lock().block_list().active_block().metadata().current_working_directory()`.
  The same accessor is reused.
- `app/src/code/file_tree/view.rs:2328 context_menu_items` — file-tree menu
  builder. The `FileTreeItem::DirectoryHeader` branch is the insertion
  point. The new item is pushed immediately after the existing
  "Reveal in Finder" / "Reveal in Explorer" / "Reveal in file manager"
  item at lines 2395-2406.
- `app/src/code/file_tree/view.rs:2333 is_remote_item` — the existing
  remote-suppression check that gates the entire local-only block of menu
  items. The new item lives inside this branch and inherits the suppression
  automatically.
- `app/src/workspace/action.rs` (existing `WorkspaceAction`) and
  `app/src/code/file_tree/view.rs` (`FileTreeAction`) — both already carry
  the menu's `on_select` enums. New variants are added in each.
- `app/src/features.rs` — `FeatureFlag` registry. A new
  `OpenDirectoryInExternalEditor` variant gates both menu items.

Out of scope for this spec:

- SSH / remote terminal cwd resolution. The current cwd reader returns
  `Option<String>`; we treat `None` as "no menu item" and defer remote
  handling.
- Re-ordering or restyling the existing menu sections. The new item is
  additive.
- Touching `app/src/drive/index.rs render_workspace_picker` (Warp Drive cloud
  workspaces have no local cwd).

## Proposed changes

1. **Add `open_directory_in_external_editor`** in
   `app/src/util/file/external_editor/mod.rs`, structured to mirror
   `open_file_path_in_external_editor`:

   ```rust path=null start=null
   pub fn open_directory_in_external_editor(
       directory: PathBuf,
       ctx: &mut AppContext,
   ) {
       let editor = match *EditorSettings::as_ref(ctx).open_file_editor {
           EditorChoice::ExternalEditor(editor) => Some(editor),
           // SystemDefault, Warp, EnvEditor fall through to the
           // platform-default open path. Menu callers omit the item for
           // `Warp` and for `EnvEditor` with no $EDITOR — see PRODUCT.md
           // Behavior 4 / 5.
           _ => None,
       };
       open_directory_with_editor(directory, editor, ctx);
   }

   pub fn open_directory_with_editor(
       directory: PathBuf,
       editor: Option<Editor>,
       ctx: &mut AppContext,
   ) {
       cfg_if::cfg_if! {
           if #[cfg(target_os = "macos")] {
               mac::open_directory(editor, &directory, ctx);
           } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
               linux::open_directory(editor, &directory, ctx);
           } else if #[cfg(windows)] {
               windows::open_directory(editor, &directory, ctx);
           } else {
               ctx.open_file_path(&directory);
           }
       }
   }
   ```

2. **Per-platform `open_directory` implementations** in
   `app/src/util/file/external_editor/{mac,linux,windows}.rs`. Each one:
   - When `editor` is `Some(Editor::*)`, look up the editor's app-id /
     binary name from the same per-platform table the file path already
     uses, then `Command::new(...).arg(directory).spawn()` (mac/linux) or
     the Windows equivalent. Reuse the existing not-found / spawn-error
     logging path.
   - When `editor` is `None`, dispatch to the platform default-open call
     (`open` / `xdg-open` / `start`) on the directory.
   - Logging on failure matches `open_file_path_with_line_and_col` —
     warn-level message and graceful return, no panic, no toast.

   Tests live alongside in
   `external_editor/{mac,linux,windows}_tests.rs` covering the editor
   resolution → command construction for at least one
   `Editor::ExternalEditor(_)` and the `None` fallback path.

3. **Feature flag.**

   - Add `FeatureFlag::OpenDirectoryInExternalEditor` to
     `app/src/features.rs`.
   - Both menu insertions are gated by
     `FeatureFlag::OpenDirectoryInExternalEditor.is_enabled()`. When the
     flag is off, neither menu builder pushes the new item, so the
     resulting `Vec<MenuItem<_>>` is identical to current master.

4. **Session-tab menu insertion** (`app/src/tab.rs`):

   - Add a new `WorkspaceAction::OpenTabCwdInExternalEditor { tab_index:
     usize }`.
   - Add a new section method
     `fn open_in_editor_menu_items(&self, index: usize, ctx: &AppContext)
     -> Vec<MenuItem<WorkspaceAction>>` that:
     - Returns `vec![]` if the feature flag is off.
     - Returns `vec![]` if the focused session view has no
       `current_working_directory()`.
     - Returns `vec![]` if `EditorSettings::open_file_editor` is
       `EditorChoice::Warp` (no in-app directory open) or `EditorChoice::
       EnvEditor` with `std::env::var("EDITOR")` empty.
     - Otherwise builds a single `MenuItemFields` with a label derived from
       the editor choice (see step 6) and an `on_select_action`
       `WorkspaceAction::OpenTabCwdInExternalEditor { tab_index: index }`.
   - Slot the section into `menu_items_with_pane_name_target`'s
     section list, between `modify_tab_menu_items` and
     `close_tab_menu_items`. The existing separator-insertion logic at
     `tab.rs:237-244` handles spacing automatically when the new section
     returns non-empty.

5. **`WorkspaceAction::OpenTabCwdInExternalEditor` handler** in
   `app/src/workspace/view.rs`: look up the tab by index, read the
   focused-session-view's `current_working_directory()`, and call
   `open_directory_in_external_editor(path, ctx)`. If the cwd is `None`
   between menu-open and selection (extremely narrow race), drop the
   action silently — matches the existing pattern for "Copy git branch"
   when the data is gone.

6. **File-tree menu insertion** (`app/src/code/file_tree/view.rs`):

   - Add a new `FileTreeAction::OpenWithExternalEditor { id:
     FileTreeIdentifier }`.
   - In `context_menu_items`, inside the
     `FileTreeItem::DirectoryHeader { .. }` branch, immediately after the
     "Reveal in Finder"-family item is pushed (`view.rs:2395-2406`), push a
     new `MenuItemFields` when:
     - `FeatureFlag::OpenDirectoryInExternalEditor.is_enabled()`, AND
     - the same `EditorChoice` gating as the tab menu (not `Warp`, not
       `EnvEditor` with empty `$EDITOR`).
     The label comes from the same helper as the tab menu (step 7).
   - Action handler (in the same `view.rs` action match) resolves the
     `FileTreeIdentifier` to an absolute local path following the existing
     `OpenInFinder` action handler's resolution pattern (the same `id →
     local path` mapping that lookup uses), then calls
     `open_directory_in_external_editor(path, ctx)`.

7. **Shared label helper.** Both menus compute the label from the same
   function:

   ```rust path=null start=null
   fn open_with_editor_menu_label(ctx: &AppContext) -> Option<String> {
       match *EditorSettings::as_ref(ctx).open_file_editor {
           EditorChoice::ExternalEditor(editor) =>
               Some(format!("Open with {editor}")),
           EditorChoice::SystemDefault => Some("Open in editor".to_string()),
           EditorChoice::EnvEditor if !std::env::var("EDITOR")
               .unwrap_or_default().is_empty() =>
               Some("Open in editor".to_string()),
           // Warp and EnvEditor-without-$EDITOR have no item.
           _ => None,
       }
   }
   ```

   The helper lives in `app/src/util/file/external_editor/mod.rs` so both
   call sites depend on the same place the platform dispatch already lives.

## Testing

- **`external_editor/{mac,linux,windows}_tests.rs`** — extend with
  directory-open cases matching the existing file-open cases.
- **`app/src/workspace/view_tests.rs`** — add a tab-context-menu test
  alongside the existing `test_tab_context_menu_copies_metadata`
  (`view_tests.rs:2042`) that asserts the new item is present with the
  expected label when the flag is on and an `Editor::VSCode` is configured,
  and absent when the flag is off.
- **`app/src/code/file_tree/view_tests`** (or the existing
  `file_tree::view` test module) — assert the same for a `DirectoryHeader`
  context menu, including the remote-suppression case.

## Rollout

- Single feature flag gates both menu items. Default off in `master`.
- Per `git-workflow` discipline, ship the spec PR first; the
  implementation PR (with the flag default still off) ships second and is
  enabled in a follow-up flip after manual verification.
