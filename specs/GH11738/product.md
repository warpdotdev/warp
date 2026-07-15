# Product Spec: Customize the Project Explorer font size (GH11738)

**Issue:** [warpdotdev/warp#11738](https://github.com/warpdotdev/warp/issues/11738)

**Figma:** none provided

## Summary

Add a dedicated **Project explorer font size** setting so users can make file and directory names in the Project Explorer easier to read without changing terminal text, notebook text, or Warp's global UI zoom. The existing 14 px appearance remains the default. A valid saved change updates visible Project Explorers immediately, including an active inline file or folder rename.

## Problem

Project Explorer rows currently render file and directory names at a fixed 14 px size. Users who need larger navigation text must increase the zoom of Warp's entire interface, while users who want a more compact tree cannot reduce only the file-name text. Terminal and notebook font controls do not affect the Project Explorer, so there is no targeted workaround.

The file tree is a dense interactive surface: rows contain hierarchy indentation, chevrons, file icons, selection and hover backgrounds, drag previews, and an inline rename editor. A font-size setting must keep those states aligned and usable rather than changing only the ordinary label.

## Goals

- Let users set the Project Explorer's file and directory name size independently.
- Preserve today's appearance for users who do not opt in.
- Apply valid changes without reopening Settings, the tools panel, or the workspace.
- Keep ordinary rows, inline rename, ignored-item styling, and drag previews visually consistent.
- Reject unsafe or unusable values before they can produce broken file-tree layout.
- Make the setting discoverable and operable with keyboard and assistive technology.

## Non-goals

- Changing the Project Explorer font family or font weight.
- Resizing chevrons, file/folder icons, indentation, the tools-panel header, context menus, loading/empty/error states, or other tools-panel surfaces.
- Coupling the Project Explorer to terminal or notebook font controls.
- Replacing or redefining Warp's global UI zoom behavior.
- Adding per-workspace or per-project values.
- Changing file-tree sorting, expansion, selection, keyboard commands, drag-and-drop semantics, or filesystem operations.
- Adding a TUI Project Explorer setting. The current Project Explorer is a GUI surface.

## User experience

In **Settings → Code → Editor and Code Review**, a row named **Project explorer font size** appears immediately after the existing **Project explorer** visibility row. Its numeric text field is labeled in pixels and describes the accepted range of **8–32 px**. It remains available when the Project Explorer visibility toggle is off so a user can configure the tree before reopening it.

The field starts with the saved value. A user can type any finite numeric value from 8 through 32, inclusive; decimal values are allowed. Pressing Enter or moving focus away commits a valid value. Pressing Escape cancels the edit and restores the saved value.

An empty, non-numeric, non-finite, below-minimum, or above-maximum entry is not saved. The tree continues using the last valid value, the field displays an error state with the range, and the error is exposed to assistive technology. Leaving an invalid field or pressing Escape restores the last saved value rather than leaving invalid text in Settings.

Once a valid value is saved, every visible Project Explorer in the process updates without being closed and reopened. If a rename is active, its text, selection, and keyboard focus stay intact while the editor adopts the new size.

## Product invariants

1. The setting is named **Project explorer font size** and is shown in **Settings → Code → Editor and Code Review**, immediately after the **Project explorer** visibility setting.
2. The public settings-file key is `code.editor.project_explorer_font_size`, represented as a numeric pixel value. The default is `14.0`.
3. If the key is absent, including for every existing installation upgraded from an older Warp version, the Project Explorer renders exactly at the current 14 px base size. Merely opening Settings does not persist an explicit value.
4. The setting is local to the device and is not uploaded through settings sync. When settings sync is enabled, the row uses Warp's standard local-only indicator and tooltip.
5. The only valid persisted values are finite numbers in the inclusive range `8.0..=32.0`. Decimal values are valid. The Settings field and direct settings-file edits enforce the same range.
6. A valid value is committed from the Settings field on Enter or focus loss. The saved value and all visible Project Explorers update in the same interaction; no app, workspace, panel, or Settings restart is required.
7. Empty, non-numeric, `NaN`, infinite, below-8, and above-32 field values never replace the saved value. The field exposes a visible and accessible error that says the value must be between 8 and 32 px. Escape or focus loss restores the saved value.
8. An invalid direct settings-file value never reaches rendering. At initial launch Warp uses the 14 px default; on a hot reload Warp retains the last valid in-memory value. The existing settings error/reporting path identifies the invalid key and does not rewrite the user's invalid entry.
9. The chosen size applies to the names of files and directories in every Project Explorer row, including ignored-item italic/light styling, the inline create/rename editor, and the label shown while dragging a tree item.
10. The chosen size does not resize hierarchy indentation, chevrons, file/folder icons, row horizontal padding, tools-panel chrome, context menus, or Project Explorer loading, empty, and error messages.
11. Terminal font size, notebook font size, and Project Explorer font size are separate stored values. Changing any one does not modify either of the other two.
12. Global UI zoom does not rewrite or derive the Project Explorer setting. The Project Explorer value remains a base logical-pixel size and continues to participate in Warp's existing renderer-level UI zoom in the same way as other UI text.
13. Project Explorer rows remeasure after a valid change. A larger value grows the row when necessary so glyphs, the 16 px icons, and vertical padding do not overlap or clip; a smaller value can reduce text height but never makes a row shorter than its unchanged icon-and-padding content requires.
14. File and directory names remain single-line. Under horizontal pressure they keep the existing trailing-edge clipping/shrinking behavior rather than wrapping, overlapping icons, or extending beneath the scrollbar. The underlying full file name and filesystem operation target are unchanged.
15. If inline create or rename is active when the value changes, the editor adopts the value immediately without clearing or committing its buffer, moving the selection, dropping focus, or ending the edit.
16. A font-size update does not rebuild or reorder filesystem data. Expanded directories, selected item, active-file highlight, pending edit, vertical scroll position, hover state, and context menu state remain unchanged except for layout adjustment required to keep the same logical scroll position valid.
17. If the Project Explorer is hidden when the value changes, reopening it uses the latest saved value on its first frame; it must not briefly render at 14 px first.
18. The numeric control is reachable in the normal Settings Tab order and exposes its name, current value, pixel unit, valid range, and validation error to supported screen readers. Enter commits and Escape cancels without changing the surrounding Settings focus order.
19. Changing the font size does not change file-tree mouse or keyboard behavior: select, expand/collapse, open, context menu, drag-and-drop, create, rename, and delete retain their current targets and commands.
20. The setting is available on every desktop platform on which the GUI Project Explorer is available and has no effect on the headless TUI.

## Validation

- Set the value to 8, 14, a decimal such as 17.5, and 32; verify ordinary files, folders, ignored names, a drag preview, and an active rename all use the selected size.
- Change the value while a large tree is scrolled, expanded, and selected; verify the same logical content and edit state remain in place after rows remeasure.
- Enter empty text, letters, `NaN`, infinity, 7.99, and 32.01; verify the setting and tree keep the last valid value, the field shows/announces the range error, and Escape or blur restores saved text.
- Put valid boundary, decimal, wrong-type, and out-of-range values in the settings file; verify valid changes hot-reload and invalid values follow invariant 8.
- Change terminal font size, notebook font size, global UI zoom, and Project Explorer font size independently and verify no setting rewrites another.
- Hide the Project Explorer, change the value, then show it; verify its first rendered frame uses the new value.
- Exercise keyboard-only and screen-reader navigation through the setting and an inline rename.
- Confirm existing users with no key still render the current 14 px layout.

## Open questions

None.
