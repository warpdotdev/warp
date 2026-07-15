# Tech Spec: Customize the Project Explorer font size (GH11738)

**Issue:** [warpdotdev/warp#11738](https://github.com/warpdotdev/warp/issues/11738)

**Code reference:** [`9e19f0741e3224c1bf8311c0223fd5f4d4a2e260`](https://github.com/warpdotdev/warp/tree/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260)

## Context

The Project Explorer currently owns a single fixed `ITEM_FONT_SIZE` of 14 px. That constant is used by ordinary row labels, the shared inline rename `EditorView`, and drag previews:

- [`app/src/code/file_tree/view.rs#L174-L177`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/code/file_tree/view.rs#L174-L177) defines the font, indentation, and padding constants.
- [`app/src/code/file_tree/view.rs#L677-L693`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/code/file_tree/view.rs#L677-L693) constructs the inline editor with the fixed size and UI font family.
- [`app/src/code/file_tree/view.rs#L1811-L1935`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/code/file_tree/view.rs#L1811-L1935) renders row indentation, 16 px icons, label text, clipping/shrinking, and 4 px vertical padding.
- [`app/src/code/file_tree/view.rs#L1946-L1982`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/code/file_tree/view.rs#L1946-L1982) renders the drag preview with the same fixed size.

The file tree already subscribes to `CodeSettings` while active, but only handles `ShowHiddenFiles` by rebuilding the flattened tree ([`app/src/code/file_tree/view.rs#L653-L665`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/code/file_tree/view.rs#L653-L665)). Activation catches up the current hidden-files value and deactivation removes the subscription ([`app/src/code/file_tree/view.rs#L343-L397`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/code/file_tree/view.rs#L343-L397)). Font-size updates can use that lifecycle without rebuilding repository data.

Rows are virtualized by `UniformList`. On each layout pass it builds and measures the first row, fixes every visible row to that measured height, recalculates visible indices and scroll bounds, and then lays out the visible rows ([`crates/warpui_core/src/elements/gui/uniform_list.rs#L175-L226`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/crates/warpui_core/src/elements/gui/uniform_list.rs#L175-L226)). A file-tree view notification is therefore sufficient to remeasure row height; no list or filesystem reconstruction is needed.

The closest settings ownership is `CodeSettings`, which already defines the Project Explorer visibility and hidden-file controls under `code.editor` ([`app/src/settings/code.rs#L46-L78`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings/code.rs#L46-L78)). Terminal and notebook font sizes live in `FontSettings` and are device-local (`SyncToCloud::Never`) ([`app/src/settings/font.rs#L15-L38`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings/font.rs#L15-L38), [`app/src/settings/font.rs#L82-L100`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings/font.rs#L82-L100)). The new value should follow that device-local precedent while remaining owned by Code rather than becoming a terminal appearance setting.

The Code settings page already groups `ProjectExplorerToggleWidget`, global search, and hidden-files widgets in **Editor and Code Review** in each of its categorized/subpage builders ([`app/src/settings_view/code_page.rs#L349-L381`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/code_page.rs#L349-L381), [`app/src/settings_view/code_page.rs#L417-L470`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/code_page.rs#L417-L470), [`app/src/settings_view/code_page.rs#L475-L533`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/code_page.rs#L475-L533)). Numeric font editors and Enter/blur commits already exist on the Appearance page ([`app/src/settings_view/appearance_page.rs#L1767-L1777`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/appearance_page.rs#L1767-L1777), [`app/src/settings_view/appearance_page.rs#L1867-L1895`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/appearance_page.rs#L1867-L1895)). The Code page can reuse that interaction pattern while adding explicit validation rather than silently ignoring an invalid entry.

## Proposed changes

### 1. Add a constrained public Code setting

In `app/src/settings/code.rs`:

- Add public constants:
  - `DEFAULT_PROJECT_EXPLORER_FONT_SIZE: f32 = 14.0`
  - `MIN_PROJECT_EXPLORER_FONT_SIZE: f32 = 8.0`
  - `MAX_PROJECT_EXPLORER_FONT_SIZE: f32 = 32.0`
- Add a transparent `ProjectExplorerFontSizeValue` value type. Its only public constructor is fallible and accepts only finite values in the inclusive range. Expose `get() -> f32` and implement `Default` with 14 px. The distinct `Value` suffix avoids colliding with the `ProjectExplorerFontSize` setting wrapper generated by `define_settings_group!`.
- Implement/derive `Debug`, `Clone`, `Copy`, `PartialEq`, `Serialize`, custom `Deserialize`, `JsonSchema`, and `SettingsValue`. Deserialization must enforce the same finite/range constraint as the constructor; the JSON schema must advertise numeric minimum 8 and maximum 32.
- Add `project_explorer_font_size: ProjectExplorerFontSize { type: ProjectExplorerFontSizeValue, ... }` to `CodeSettings` with:
  - default `ProjectExplorerFontSizeValue::default()`;
  - `SupportedPlatforms::ALL`;
  - `SyncToCloud::Never`;
  - `SettingSurfaces::GUI`;
  - `private: false`;
  - TOML path `code.editor.project_explorer_font_size`;
  - description `The font size used for file and directory names in the Project Explorer.`

Using a validated value type, rather than a raw `f32` checked only by Settings UI, ensures startup, hot reload, native-store migration, and future programmatic callers cannot put `NaN`, infinity, or an out-of-range value into layout. `Setting::read_from_preferences` already rejects a `SettingsValue::from_file_value` failure, reports the setting key, and inhibits writes to that invalid key; no settings-manager change is needed.

The absent-key path naturally uses the 14 px default and remains not-explicitly-set, preserving existing installations without writing a new key. `SyncToCloud::Never` keeps the display-specific preference local, matching terminal and notebook size settings.

### 2. Add the Code settings numeric editor

In `app/src/settings_view/code_page.rs`:

- Give `CodeSettingsPageView` a single-line `EditorView` for the value and a small validation state (`Valid` or `Invalid`). Initialize its text from `CodeSettings::project_explorer_font_size`.
- Add `ProjectExplorerFontSizeWidget` immediately after `ProjectExplorerToggleWidget` in all three Editor/Code Review widget lists. Its search terms include `project explorer file tree font text size`.
- Render the row with label **Project explorer font size**, a pixel/range description, the numeric editor, and `LocalOnlyIconState::for_setting(ProjectExplorerFontSize::storage_key(), ProjectExplorerFontSize::sync_to_cloud(), ...)`. The page must retain the associated `MouseStateHandle` instead of constructing it during render. Warp's standard local-only implementation and tooltip are in [`app/src/settings_view/settings_page.rs#L492-L545`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/settings_page.rs#L492-L545) and [`app/src/settings_view/settings_page.rs#L602-L617`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/settings_view/settings_page.rs#L602-L617).
- Add an action such as `CommitProjectExplorerFontSize`. Dispatch it for Enter and `Dismiss`/blur. Parse as `f32`, construct `ProjectExplorerFontSizeValue`, and call `set_value` only on success.
- On a parse/constructor failure, leave `CodeSettings` untouched, mark the field invalid, and render/announce `Enter a value from 8 to 32 px.` On blur, restore the saved value after exposing the failed validation; on Escape, restore without saving and return focus through the existing Settings flow.
- Subscribe the page to `CodeSettingsChangedEvent::ProjectExplorerFontSize` so a valid settings-file hot reload updates the editor text when it is not holding an uncommitted user edit. Rebuilding a Code subpage must reuse the same editor handle and validation state.
- Attach accessible name, numeric value, unit, minimum, maximum, and error description to the text input. This is additive to the generic editor semantics and does not change surrounding focus order.

Keep the field enabled when `show_project_explorer` is false. Visibility and font configuration are independent preferences.

### 3. Drive every file-tree text state from the setting

In `app/src/code/file_tree/view.rs`:

- Remove `ITEM_FONT_SIZE`; keep `FOLDER_INDENT` and `ITEM_PADDING` unchanged.
- Add `item_font_size: f32` to `FileTreeView`, initialized from `CodeSettings::project_explorer_font_size` before constructing the inline editor. Pass that same initial value to `TextOptions::ui_text`/`font_size_override` so a newly shown tree cannot flash at 14 px.
- Extend the existing `CodeSettingsChangedEvent` handling with a `ProjectExplorerFontSize` branch alongside the current `ShowHiddenFiles` branch; leave the group's unrelated setting events as no-ops. For a size event:
  1. copy the validated value into `item_font_size`;
  2. call `EditorView::set_font_size` on the existing inline editor;
  3. call `ctx.notify()`;
  4. do **not** call `rebuild_flattened_items`.
- When a local-filesystem tree is reactivated after being unsubscribed, refresh both `show_hidden_files` and `item_font_size` from `CodeSettings` and update the editor before notifying/rendering. Non-local/remote construction must likewise initialize from the current value.
- Thread `item_font_size` through `render_item_with_hover` for ordinary/ignored labels and through `render_item_while_dragging` for the drag preview.

`EditorView::set_font_size` updates only `text_options.font_size_override` and notifies the editor ([`app/src/editor/view/mod.rs#L3349-L3357`](https://github.com/warpdotdev/warp/blob/9e19f0741e3224c1bf8311c0223fd5f4d4a2e260/app/src/editor/view/mod.rs#L3349-L3357)). Reusing the view therefore preserves its buffer, selection, focus, and pending-edit ownership.

Keep icons at 16 px, indentation at 16 px per level, the existing horizontal margins, and `ITEM_PADDING` at 4 px. The row's natural `Flex` cross-axis size becomes `max(text/editor height, icon height) + vertical padding`; `UniformList` remeasures that height after the notification. Keep `Text::new_inline` inside `Shrinkable` and the rename editor inside `Clipped`, preserving single-line trailing-edge clipping without introducing wrapping or horizontal overlap.

Do not apply the value to loading, empty, error, context-menu, header, or toolbelt text. Those are UI chrome and continue using `Appearance::ui_font_size()`.

### 4. Preserve state and setting independence

No change is required to `FontSettings`, `WindowSettings::zoom_level`, repository models, the flattened `FileTreeItem` vectors, selection identifiers, expansion maps, scroll handles, or edit actions.

The new Code setting supplies a logical base font size to the existing UI text renderer. Warp's window zoom remains a renderer-level multiplier; neither setting writes the other. Terminal and notebook font size continue to read only `FontSettings`.

The size-event handler deliberately avoids `rebuild_flattened_items`. Retaining `UniformListState`, `ScrollStateHandle`, the root/item vectors, `selected_item`, `pending_edit`, and `editor_view` preserves logical state. The next layout pass recalculates row height and clamps only if the previous scroll offset no longer fits the new content bounds.

## Testing and validation

### Unit tests

Add `app/src/settings/code_tests.rs`, included from `code.rs` using the repository's separate-test-file convention:

- default value is exactly 14 px and an absent key remains implicit;
- constructor and file deserialization accept 8, 14, 17.5, and 32;
- they reject `NaN`, both infinities, 7.99, 32.01, strings, booleans, and null;
- file serialization round-trips a valid decimal as a numeric value;
- setting metadata is public GUI, `SyncToCloud::Never`, and uses `code.editor.project_explorer_font_size`.

Extend `app/src/settings_view/code_page_tests.rs`:

- a factored parser accepts boundaries/decimals and rejects empty/non-finite/out-of-range text;
- commit persists a valid value and synchronizes the field after a model event;
- invalid commit does not change `CodeSettings`, marks the editor invalid, and Escape/blur restores the saved text;
- the widget's search terms match both `project explorer` and `file tree font size` queries.

Extend `app/src/code/file_tree/view/view_tests.rs`:

- a new tree starts with the default or preconfigured setting in both its row state and inline editor;
- changing the setting while active updates `item_font_size` and the existing editor handle, without changing flattened item paths, selection, expansion, pending-edit text/selection, or scroll handle;
- changing the setting while inactive and then reactivating catches up before rendering;
- the show-hidden-files event still rebuilds/filter items while a font-size event does not, guarding both handled event branches;
- boundary and decimal sizes can complete a layout pass without panic, negative height, or non-finite geometry.

Where element-test helpers expose measured bounds, add a GUI layout assertion that 32 px text yields a taller non-clipping row than 14 px while 8 px still fits the unchanged 16 px icon plus padding. Otherwise keep that geometry assertion in the existing real-display integration/manual suite rather than coupling a unit test to private renderer internals.

### Manual / GUI integration validation

1. Open a tree containing deep nesting, long names, ignored items, files and folders. Exercise 8, 14, 17.5, and 32 px and capture the ordinary, selected, hover, scrolled, and drag-preview states.
2. Begin renaming a long item, select part of its name, then change the setting. Verify the buffer, selection, focus, and edit remain live at the new size; Enter and Escape still work.
3. Scroll and expand a large tree, change size in both directions, and confirm selection/expansion and logical scroll location survive.
4. Hide the Project Explorer, change the value, show it, and verify the first frame uses the saved size.
5. Test the field with mouse, Tab/Shift-Tab, Enter, Escape, blur, VoiceOver, and invalid inputs. Confirm its error and 8–32 range are announced.
6. Change terminal size, notebook size, Project Explorer size, and UI zoom in turn; confirm their stored values are independent while global UI zoom still scales the rendered window normally.
7. Start with a profile/settings file that lacks the new key and compare against the pre-change 14 px Project Explorer.
8. Run on macOS, Windows, and Linux GUI builds, plus a TUI smoke test confirming the GUI-only setting does not alter TUI behavior or schema output.

## Invariant-to-test map

| Product invariant(s) | Primary coverage |
| --- | --- |
| 1, 18 | Code page widget/search tests; keyboard and screen-reader manual pass |
| 2, 3, 20 | Setting metadata/default tests; upgrade and GUI/TUI smoke tests |
| 4 | Setting metadata test; local-only icon UI check |
| 5, 7, 8 | Value-type serialization/deserialization tests; Code page invalid-input tests; settings-file hot-reload test |
| 6, 17 | File-tree active/inactive subscription tests; hidden-then-show manual pass |
| 9, 10 | File-tree render/layout test plus ordinary/ignored/drag manual pass |
| 11, 12 | Settings-model independence assertions and UI zoom manual pass |
| 13, 14 | GUI geometry assertion or real-display integration test at min/default/max and narrow widths |
| 15 | Pending rename state/editor-handle unit test and manual rename pass |
| 16 | File-tree state preservation unit test and large-tree manual pass |
| 19 | Existing file-tree tests plus mouse/keyboard regression pass |

## Risks and mitigations

### Invalid values entering through the settings file

A raw `f32` setting would let non-finite and extreme values bypass the Settings field. The constrained value type validates every deserialization path and exposes schema bounds, so unsafe values fail before layout and use the settings system's existing report/inhibit-write behavior.

### Inline rename diverging from ordinary row text

The rename editor is constructed once and currently captures the fixed constant. The setting-event handler explicitly updates that existing editor with `set_font_size`, while construction and reactivation initialize it from the same source as ordinary rows. Tests assert handle/buffer/focus continuity.

### Virtualized rows using stale height

The list measures its first row on each layout and stores scroll state separately. Notifying the file-tree view after updating the size forces a fresh measure. Avoiding a flattened-data rebuild prevents unrelated selection/expansion churn.

### Very large labels reducing usable horizontal space

The upper bound is limited to 32 px, and existing `Shrinkable`/`Clipped` single-line behavior remains in place. Icons, indentation, and the scrollbar retain their current dimensions and labels never wrap beneath another row.

### Device sync producing inconsistent physical results

Displays, DPI, and window scale differ across machines. Marking the setting `SyncToCloud::Never`, like terminal and notebook font size, keeps the value device-specific and makes the standard local-only indicator explicit.

## Follow-ups

- If users request it, consider a separate Project Explorer font-family control. It is intentionally not implied by this size-only setting.
- If future tools-panel surfaces need independent size controls, evaluate a broader tools-panel typography model rather than silently reusing this Project Explorer-specific key.
