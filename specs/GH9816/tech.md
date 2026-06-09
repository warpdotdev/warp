# GH9816: Tech Spec — Configurable code editor line number modes
## Context
The product behavior is specified in `specs/GH9816/product.md`. The implementation should add a persistent editor setting and apply it to code editor gutters only.
- `app/src/settings/editor.rs:132` defines `AppEditorSettings`, including `vim_mode`, `vim_unnamed_system_clipboard`, and `vim_status_bar` under `text_editing.*`. This is the right settings group for an independent code/text editing line-number mode.
- `app/src/settings/init.rs:53` registers `AppEditorSettings`, so adding a field to that settings group automatically participates in normal startup registration.
- `app/src/settings_view/features_page.rs (2491-2690)` builds the Text Editing category. Today it includes `AutocompleteSymbolsWidget` and conditionally `VimModeWidget`.
- `app/src/settings_view/features_page.rs (5763-5962)` renders `VimModeWidget` and its nested Vim-only subsettings. The new line number mode should not be nested in this widget because the maintainer explicitly called for a setting independent of Vim settings.
- `app/src/code/editor/view.rs (46-245)` defines `CodeEditorViewDisplayOptions`, including `show_line_numbers` and `starting_line_number`.
- `app/src/code/editor/view.rs (1041-1239)` builds `LineNumberConfig` from appearance settings and passes it when line numbers are enabled.
- `app/src/code/editor/view.rs (2068-2267)` creates `EditorWrapper` with `line_number_config`, diff status, saved comments, and gutter behavior.
- `app/src/code/editor/element.rs (277-476)` defines `LineNumberConfig` and `EditorWrapper`.
- `app/src/code/editor/element.rs (500-790)` builds gutter elements from visible editor blocks. Current absolute display is computed with `line_count.as_usize() + line_number_config.starting_line_number.unwrap_or(1)`.
- `app/src/code/editor/element.rs (1048-1247)` renders the final gutter text in `render_gutter_element`.
- `app/src/code/editor/view.rs (1525-1724)` exposes cursor helpers such as `cursor_lsp_position`, `cursor_head_offset`, and offset-to-position conversion that can be used to determine the active cursor line.
- `app/src/terminal/input/common.rs:44`, `app/src/terminal/input/classic.rs (1-220)`, `app/src/terminal/input/universal.rs (1-200)`, and `app/src/editor/view/mod.rs (8546-8745)` show terminal input editors render Vim status and editor content but no line-number gutter. They should remain untouched except for regression testing.
- `app/src/notebooks/editor/view.rs (2466-2664)` renders the rich-text notebook editor with `RichTextElement` and explicitly does not support Vim; it also does not use the code editor gutter.
## Proposed changes
### 1. Add a persisted line number mode setting
In `app/src/settings/editor.rs`, add a new enum near the existing cursor/Vim editor enums:
- `CodeEditorLineNumberMode::Absolute`
- `CodeEditorLineNumberMode::Relative`
Derive the same traits used by nearby public settings enums: `Clone`, `Copy`, `Debug`, `Default`, `Eq`, `PartialEq`, `Deserialize`, `Serialize`, `Sequence`, `schemars::JsonSchema`, and `settings_value::SettingsValue`. Use `#[schemars(rename_all = "snake_case")]` and make `Absolute` the default.
Add a setting to `define_settings_group!(AppEditorSettings, settings: [...])`:
- field name: `code_editor_line_number_mode`
- type: `CodeEditorLineNumberMode`
- default: `CodeEditorLineNumberMode::default()`
- supported platforms: `SupportedPlatforms::ALL`
- sync: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- private: `false`
- TOML path: `text_editing.code_editor_line_number_mode`
- description: `How line numbers are displayed in code editors.`
Add small helpers on the enum:
- `dropdown_item_label(&self) -> &'static str` returning `Absolute` and `Relative`
- optional `search_terms()` or a widget-level search string that covers `line number relative vim gutter`
### 2. Add the settings UI dropdown
In `app/src/settings_view/features_page.rs`:
1. Import `CodeEditorLineNumberMode` and the generated setting type, likely `CodeEditorLineNumberModeSetting` or the actual generated name from `define_settings_group!`.
2. Add `SetCodeEditorLineNumberMode(CodeEditorLineNumberMode)` to `FeaturesPageAction`.
3. No new dedicated telemetry event is required for this setting in this iteration; the settings action should use the existing `FeaturesPageAction` telemetry path like other setters.
4. Add action handling that writes the setting:
   - `AppEditorSettings::handle(ctx).update(ctx, |settings, ctx| report_if_error!(settings.code_editor_line_number_mode.set_value(*mode, ctx)))`
   - Notify after the write so settings UI and open editors repaint.
5. Add a `code_editor_line_number_mode_dropdown: ViewHandle<Dropdown<FeaturesPageAction>>` field to `FeaturesPageView`.
6. Initialize it with `ctx.add_typed_action_view(Dropdown::new)` and call a helper such as `Self::update_code_editor_line_number_mode_dropdown(...)`.
7. Subscribe to `AppEditorSettings::handle(ctx)` changes or update the dropdown in the existing AppEditorSettings subscription if one is added. The selected item must stay in sync when settings change outside the dropdown, such as through `settings.toml`.
8. Add a `CodeEditorLineNumberModeWidget` to the Text Editing category in `build_page`, adjacent to `AutocompleteSymbolsWidget` and before/after `VimModeWidget`. This ensures it is not conditional on `vim_mode`.
9. Render the widget with `render_dropdown_item`, label it `Code editor line numbers:` or `Line numbering:`, pass the local-only/sync indicator for the generated setting, and point it at `view.code_editor_line_number_mode_dropdown`.
### 3. Pass the selected mode into code editor line-number rendering
Extend `LineNumberConfig` in `app/src/code/editor/element.rs`:
- add `mode: CodeEditorLineNumberMode`
- add `active_line_number: Option<LineCount>` or `active_line_index: Option<usize>`
In `CodeEditorView::line_number_config` (`app/src/code/editor/view.rs (1041-1239)`):
1. Read `let editor_settings = AppEditorSettings::as_ref(ctx)`.
2. Set `mode: *editor_settings.code_editor_line_number_mode.value()`.
3. Compute the active cursor line from the primary selection head so normal code editors can keep using Relative mode immediately after opening and after temporary focus changes:
   - Use `self.model.as_ref(ctx).selections(ctx).first().head`.
   - Convert the head to a buffer point with the code editor buffer.
   - Convert that row to the same `LineCount` convention used by `model.start_line_index(&**block)`.
   - Prefer keeping this conversion in a helper on `CodeEditorView` or `CodeEditorModel`, such as `active_cursor_line_for_line_numbers(&self, ctx) -> Option<LineCount>`, to avoid duplicating offset/index assumptions in the wrapper.
4. Also pass whether the editor is currently focused into `LineNumberConfig`. Normal code editors should not require focus to display Relative mode, but diff/review editors should still require editor focus before applying relative line numbers.
5. Keep returning `None` when `show_line_numbers` is false.
### 4. Compute the displayed gutter value per line
In `EditorWrapper::gutter_elements` (`app/src/code/editor/element.rs (500-790)`), replace the current absolute-only `current_line` computation with a helper:
```
fn display_line_number(
    line_count: LineCount,
    config: &LineNumberConfig,
) -> usize
```
The helper should implement:
- `absolute = line_count.as_usize() + config.starting_line_number.unwrap_or(1)`
- `relative = config.active_line_number.map(|active| active.as_usize().abs_diff(line_count.as_usize()))`
- Absolute mode returns `absolute`.
- Relative mode returns `absolute` when `Some(line_count) == active_line_number`, otherwise `relative.unwrap_or(absolute)`, so editors without an active cursor fall back gracefully.
Use the returned value as the `current_line` passed into `render_gutter_element`.
Important indexing detail: the current code’s absolute calculation implies `line_count` is zero-based for display purposes. The implementation must verify the active cursor conversion uses the same convention. A small unit test should cover this directly to avoid off-by-one bugs.
### 5. Keep non-number gutter elements unchanged
Do not display relative numbers for:
- temporary removed diff blocks, which currently pass `None` to `render_gutter_element`
- hidden-section controls, which use `construct_expand_hidden_section_gutter_element`
- surfaces where `line_number_config` is `None`
For diff and review editors, preserve absolute numbering unless the editor is focused and Relative is selected. When focused, numbered current-buffer lines across the editor should apply the selected Relative display from the active cursor line so review context outside the changed hunk uses the same relative origin. Normal code editor surfaces without diff status should use the retained primary selection head as the relative origin even if the editor is not currently focused. Diff hunk and comment interactions should continue to use `EditorLineLocation` and `line_range` exactly as they do today; only the text shown inside eligible numbered gutter elements changes.
### 6. Width and alignment
The existing `GUTTER_WIDTH` is fixed and currently supports absolute numbers plus gutter controls. Do not change it unless testing shows three-digit or larger relative values clip in common cases. Relative mode still shows the active line’s absolute number, so any width calculation must account for both absolute active-line values and relative non-active-line distances. If adjustment is needed, prefer the smallest safe change within `app/src/code/editor/element.rs`, and verify diff/comment buttons still fit.
### 7. Do not wire terminal input or notebook editors
No changes are needed in `app/src/terminal/input/*`, `app/src/editor/view/mod.rs`, or `app/src/notebooks/editor/view.rs` to render line numbers. The new setting can live in shared editor settings, but only `CodeEditorView` should consume it.
## End-to-end flow
1. User selects `Relative` from Settings > Text Editing > line numbering.
2. `FeaturesPageAction::SetCodeEditorLineNumberMode(Relative)` writes `AppEditorSettings.code_editor_line_number_mode`.
3. Open `CodeEditorView` instances observe settings changes and re-render.
4. `CodeEditorView::line_number_config` includes the selected mode and active cursor line.
5. `EditorWrapper::gutter_elements` computes each visible current-buffer line’s displayed number from the mode.
6. Cursor movement emits the existing selection/content events, causing the view to notify and repaint; relative gutter values update on the next render.
## Risks and mitigations
1. **Off-by-one errors between buffer rows and gutter `LineCount`.** Mitigate with focused tests for cursor on first, middle, and last lines in Relative mode, and with a code comment documenting the chosen convention.
2. **Settings UI accidentally scopes the setting under Vim.** Mitigate by implementing a separate Text Editing widget rather than adding it to `VimModeWidget`’s conditional subgroup.
3. **Open editors may not repaint when the setting changes.** `CodeEditorView::new` already subscribes to appearance and font settings; add or reuse an `AppEditorSettings` observation/subscription if necessary so setting changes notify code editor views.
4. **Diff/review gutter regression.** The implementation touches the shared code editor wrapper used by code review surfaces. Mitigate with manual testing in a diff editor, keeping `EditorLineLocation` unchanged, and verifying inactive diff/review editors keep absolute numbering while focused editors apply Relative numbering across numbered current-buffer lines.
5. **Multi-cursor ambiguity.** The product spec defines the primary selection head as the relative origin. Mitigate by using `selections(ctx).first().head`, which matches existing cursor-position helpers.
## Testing and validation
1. Add or update code editor view/element tests to cover the number calculation helper:
   - Absolute mode returns the same values as today.
   - Relative mode returns absolute on the active line and positive distances for lines above/below.
   - Missing active cursor line falls back to absolute values.
   - `starting_line_number` still affects absolute and relative active-line display without affecting non-active relative distances.
2. Add settings tests, if the existing settings test harness supports them, to verify `text_editing.code_editor_line_number_mode = "relative"` deserializes and invalid values fall back through normal settings validation.
3. Manually verify product invariants from `specs/GH9816/product.md`:
   - Behavior 1-7 in a normal code editor.
   - Behavior 8-10 with Vim disabled/enabled, visual selections, and soft-wrapped lines.
   - Behavior 11 with hidden/collapsed code regions.
   - Behavior 12 in code review/diff views with focused and unfocused sections, hidden sections, and inline comments.
   - Behavior 15 in terminal input, AI input, and notebook editors.
4. Run the repository’s standard formatting/check flow for touched Rust files. At minimum, run targeted Rust tests for settings and code editor modules; if feasible, run the broader app test command used by the repository before the implementation PR.
## Parallelization
After the settings enum name is settled, implementation can split across two agents:
1. Settings/UI agent: adds the `AppEditorSettings` enum/field, settings dropdown, action handling, and settings tests.
2. Editor rendering agent: adds `LineNumberConfig` mode/origin support, display calculation helper, code editor tests, and manual diff-editor validation.
These streams should coordinate on the exact enum and field names before parallel edits to avoid merge conflicts.
## Follow-ups
1. Consider Vim command support (`:set number`, `:set relativenumber`) only after the settings-based behavior ships.
2. Consider adding line-number mode telemetry only if product analytics need to measure adoption; this spec does not require new telemetry.
3. Revisit gutter width if future designs add more gutter affordances or larger inline controls.
