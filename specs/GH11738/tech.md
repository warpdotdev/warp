# Tech Spec: Make Tools pane text respect the app font size (GH11738)

**Issue:** [warpdotdev/warp#11738](https://github.com/warpdotdev/warp/issues/11738)

**Code reference:** [`fd210f13fe5904f01308d3853e09268d15aa6597`](https://github.com/warpdotdev/warp/tree/fd210f13fe5904f01308d3853e09268d15aa6597)

## Context

Warp already persists the user-controlled font size as `FontSettings::monospace_font_size`, with storage key `FontSize` and public TOML path `appearance.text.font_size` ([`app/src/settings/font.rs#L13-L38`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/settings/font.rs#L13-L38)). `AppearanceManager` copies setting changes into `Appearance`, whose `monospace_font_size()` accessor is the runtime source used throughout the app ([`app/src/appearance.rs#L48-L91`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/appearance.rs#L48-L91), [`crates/warp_core/src/ui/appearance.rs#L280-L310`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/crates/warp_core/src/ui/appearance.rs#L280-L310)). A size change emits `AppearanceEvent::MonospaceFontSizeChanged` and invalidates all views.

The existing Settings editor parses whole-pixel values and accepts the inclusive `1..=120` range ([`app/src/settings_view/appearance_page.rs#L106-L113`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/settings_view/appearance_page.rs#L106-L113), [`app/src/settings_view/appearance_page.rs#L1864-L1878`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/settings_view/appearance_page.rs#L1864-L1878)). This proposal preserves that validation and must lay out safely at both exact boundaries.

`Appearance::ui_font_size()` is not that setting: it returns the fixed `DEFAULT_UI_FONT_SIZE` of 12 px. Window Zoom is also distinct; `WindowSettings::zoom_level` sets WarpUI's global zoom factor and therefore already scales the complete scene ([`app/src/window_settings.rs#L79-L106`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/window_settings.rs#L79-L106), [`app/src/lib.rs#L1334-L1342`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/lib.rs#L1334-L1342)). This change must derive from the existing monospace setting and must not add a setting or reuse Zoom.

`Appearance` already provides the derivation used when UI text or geometry should track that setting: `monospace_ui_scalar()` returns `monospace_font_size / DEFAULT_UI_FONT_SIZE` ([`crates/warp_core/src/ui/appearance.rs#L288-L305`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/crates/warp_core/src/ui/appearance.rs#L288-L305)). Existing consumers multiply a UI baseline by this scalar, including profile/model-selector text and subordinate todo text ([`app/src/terminal/profile_model_selector.rs#L91-L96`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/terminal/profile_model_selector.rs#L91-L96), [`app/src/ai/blocklist/block/view_impl/todos.rs#L209-L218`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/ai/blocklist/block/view_impl/todos.rs#L209-L218)). The Tools pane must reuse that accessor rather than reimplementing the division or treating the selected monospace size as a 14 px UI baseline.

The Tools pane currently has four views: `ProjectExplorer`, `ConversationListView`, `GlobalSearch`, and `WarpDrive` ([`app/src/workspace/view/left_panel.rs#L104-L110`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/left_panel.rs#L104-L110)). Each tab contains fixed typography:

- Project Explorer defines `ITEM_FONT_SIZE` as 14 px and uses it for row labels, inline create/rename, and drag previews ([`app/src/code/file_tree/view.rs#L173-L177`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/code/file_tree/view.rs#L173-L177), [`app/src/code/file_tree/view.rs#L675-L692`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/code/file_tree/view.rs#L675-L692), [`app/src/code/file_tree/view.rs#L1884-L1970`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/code/file_tree/view.rs#L1884-L1970)). Its in-pane unavailable/error heading and body separately use fixed `appearance.ui_font_size() + 2.` text ([`app/src/code/file_tree/view.rs#L2750-L2792`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/code/file_tree/view.rs#L2750-L2792)).
- Global Search captures 14 px in its query editor and hard-codes 14/12 px for result names, paths, counts, and status text ([`app/src/workspace/view/global_search/view.rs#L637-L659`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/global_search/view.rs#L637-L659), [`app/src/workspace/view/global_search/view.rs#L1195-L1223`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/global_search/view.rs#L1195-L1223), [`app/src/workspace/view/global_search/view.rs#L2088-L2185`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/global_search/view.rs#L2088-L2185)).
- Conversation History captures fixed or fixed-UI-derived sizes in its query and rename editors; item titles, timestamps, subtext, empty states, actions, and section labels use 16/14/12/11 px roles ([`app/src/workspace/view/conversation_list/view.rs#L220-L261`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/conversation_list/view.rs#L220-L261), [`app/src/workspace/view/conversation_list/item.rs#L198-L283`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/conversation_list/item.rs#L198-L283), [`app/src/workspace/view/conversation_list/view.rs#L781-L832`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/workspace/view/conversation_list/view.rs#L781-L832)).
- Warp Drive's index and row styling define fixed 16/14/12 px roles, and its inline title editor is constructed independently ([`app/src/drive/index.rs#L108-L115`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/drive/index.rs#L108-L115), [`app/src/drive/items/item.rs#L78-L108`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/drive/items/item.rs#L78-L108), [`app/src/drive/index.rs#L906-L935`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/drive/index.rs#L906-L935)).

Render-time text automatically sees an `Appearance` change because the app invalidates all views. Long-lived `EditorView` instances are different: their `TextOptions` capture a size at construction, so their existing handles must be updated with `EditorView::set_font_size` when `AppearanceEvent::MonospaceFontSizeChanged` arrives ([`app/src/editor/view/mod.rs#L3345-L3357`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/app/src/editor/view/mod.rs#L3345-L3357)).

## Proposed changes

### 1. Define shared Tools pane typography roles

Add a small shared module under `app/src/workspace/view/` that derives Tools pane font roles from an `Appearance`. Read `let scalar = appearance.monospace_ui_scalar()` once and apply it to the tabs' existing UI baselines:

- `primary = 14.0 * scalar`
- `secondary = 12.0 * scalar`
- `heading = 16.0 * scalar`
- `overline = 11.0 * scalar`

Return these values from a copyable `ToolsPaneTypography` struct or equivalently named helpers. Reusing `monospace_ui_scalar()` follows the app's established UI-scaling path and preserves today's hierarchy exactly at the 12 px unit-scalar point. At the current 13 px default, the roles are approximately 15.17/13/17.33/11.92 px; at 14 px they are approximately 16.33/14/18.67/12.83 px. The results stay finite and positive across the exact inclusive 1–120 range and avoid unrelated per-tab constants drifting apart. Do not round the derived values before layout. At 1 and 120 px the roles must still satisfy `heading > primary > secondary > overline`; layout safety, rather than legibility at 1 px or full horizontal visibility at 120 px, is the boundary guarantee.

Keep `Appearance::ui_font_family()` as the family for all four tabs. The existing setting supplies the scalar only; each role retains its UI baseline. Do not add fields or events to `FontSettings`, `CodeSettings`, `WindowSettings`, or `Appearance`, and do not change `appearance.text.font_size` persistence or validation.

The shared roles apply only to content rendered inside the four `ToolPanelView` variants. Toolbelt buttons and panel chrome remain on standard UI metrics. Dialogs, menus, workflow editors, and full panes launched from Warp Drive are not consumers of this helper.

### 2. Adapt Project Explorer

In `app/src/code/file_tree/view.rs`:

- Remove `ITEM_FONT_SIZE` and use `ToolsPaneTypography::primary` for ordinary and ignored row labels and drag previews.
- Construct the inline create/rename editor with the current primary value so a newly shown tree cannot flash at 14 px.
- In `render_error_state`, use `primary` for both the unavailable heading and body, preserving the heading's semibold weight. This replaces both fixed `ui_font_size() + 2.` calls and ensures the entire in-pane state follows the setting without inventing a different size hierarchy.
- Subscribe to `AppearanceEvent::MonospaceFontSizeChanged` and call `set_font_size(primary, ctx)` on the existing editor handle. Reuse the editor; do not recreate it or its buffer.
- Leave `FOLDER_INDENT`, icon constraints, horizontal padding, and `ITEM_PADDING` unchanged. `Flex` determines the row's cross-axis size from the larger of text/editor and icons: fixed icons establish the minimum at 1 px, while text establishes the row height at 120 px. Existing `Shrinkable`/`Clipped` behavior handles horizontal pressure without overlap.
- Do not extend `CodeSettingsChangedEvent`; this behavior has no Code setting. The existing `ShowHiddenFiles` subscription remains unchanged.

`UniformList` measures its first row during each layout, applies that height to visible rows, and recalculates visible bounds ([`crates/warpui_core/src/elements/gui/uniform_list.rs#L175-L226`](https://github.com/warpdotdev/warp/blob/fd210f13fe5904f01308d3853e09268d15aa6597/crates/warpui_core/src/elements/gui/uniform_list.rs#L175-L226)). Global view invalidation plus the editor notification therefore remeasures rows without rebuilding `root_directories`, flattened items, selection, expansion, pending edits, or scroll handles.

### 3. Adapt Global Search

In `app/src/workspace/view/global_search/view.rs`:

- Use `primary` for the query editor, Search label, directory/file labels, matching lines, and zero-state body text that is 14 px today.
- Use `secondary` for paths, match counts, progress/result summaries, capped-result copy, and badges that are 12 px today.
- Use `heading` for zero-state headings currently expressed as `appearance.ui_font_size() + 2.` or another 14/16 px fixed role, choosing the role that preserves the current hierarchy at the 12 px unit-scalar point.
- Update the existing query editor with `set_font_size(primary, ctx)` on `MonospaceFontSizeChanged`; preserve its buffer, selection, focus, debounce channel, and search model.
- Keep query controls and file icons at their existing dimensions. Let result and directory row containers grow from their text rather than introducing fixed heights.

Do not restart or rerun a search solely because the font size changed. The results model, selected row, collapsed directories, query text, and scroll state are data state rather than typography state.

### 4. Adapt Conversation History

In `app/src/workspace/view/conversation_list/view.rs` and `item.rs`:

- Replace fixed 14 px query/zero-state content with `primary`, fixed 12 px control/subtext with `secondary`, fixed 16 px item/rename roles with `heading`, and fixed 11 px section labels with `overline`.
- Update both the query editor and active rename editor from the same current roles when `MonospaceFontSizeChanged` arrives. Reuse both handles so search text, rename text, selection, focus, and pending rename ownership survive.
- Derive item title, timestamp, status, and subtext styles during render. Recalculate status-element and row geometry from the derived roles where it currently depends on `appearance.ui_font_size()`.
- Preserve existing single-line clipping, overflow actions, section collapse behavior, selection, and keyboard navigation.

Do not rebuild conversation data or reset active/past sections in response to typography. The app-wide invalidation is sufficient for render-derived rows.

### 5. Adapt Warp Drive tab content

In `app/src/drive/index.rs`, `app/src/drive/items/item.rs`, and tab-local item renderers:

- Replace `ITEM_FONT_SIZE`, `SECTION_HEADER_FONT_SIZE`, `TEAM_SECTIONS_TITLE_FONT_SIZE`, `TITLE_FONT_SIZE`, and fixed 14/12 px index-content usages with the corresponding shared roles.
- Build `WarpDriveItemStyles` from `ToolsPaneTypography` on each render. Derive row height from the resulting primary size while retaining icon minimums, margins, indentation, and click targets.
- Construct the existing `DriveIndex` title editor with both the primary size and UI font family, for example `TextOptions::ui_text(Some(primary), appearance)` in `SingleLineEditorOptions` (or explicit `set_font_size` plus `set_font_family`). On `MonospaceFontSizeChanged`, update only its size and verify it remains on `Appearance::ui_font_family()`; do not recreate the editor or alter its buffer/focus.
- Apply the roles to tab-local empty, warning, history, countdown, section, item, and action text. Keep existing wrapping or clipping behavior.
- Do not change constants in sharing dialogs, import modals, workflow modals, object panes, or other UI merely reachable from Drive; those are not rendered as Tools pane tab content.

Cloud model data, sorting, sections, expansion, selected item, focused index, drag state, title edits, and network operations must not change when typography changes.

### 6. Preserve reactivity and state

Subscribe each owner of a long-lived editor to `Appearance`, not directly to `FontSettings`. Match `AppearanceEvent::MonospaceFontSizeChanged` exhaustively within the existing event handler and update only captured editor text options. Render-only consumers require no subscription because `Appearance::set_monospace_font_size` already calls `invalidate_all_views`.

At construction, read the current `Appearance` value before creating editors and initialize both size and `Appearance::ui_font_family()`. This covers a tab first shown after a change, avoids a one-frame fixed-size fallback, and prevents a default editor family from diverging from surrounding Tools pane text.

Do not rebuild filesystem, search, conversation, or cloud models. Preserve existing view/model handles and let layout recompute row bounds. When a previous scroll offset exceeds the new maximum, normal list clamping is allowed; otherwise retain the logical scroll position.

## Testing and validation

### Unit and view tests

Add focused tests for the shared typography helper:

- a 12 px app font size produces a unit scalar and exactly 14 px primary, 12 px secondary, 16 px heading, and 11 px overline roles;
- exact 1 and 120 px boundaries plus the current 13 px default and 14 px value produce finite positive roles and preserve `heading > primary > secondary > overline`;
- the helper reuses `Appearance::monospace_ui_scalar()` and never duplicates its division or reads window Zoom.

Extend the existing view tests for each tab:

- Project Explorer starts with a configured primary size and a live change updates the same inline-editor handle without changing flattened paths, selection, expansion, pending-edit text/selection, or scroll handle. Error-state heading/body render at `primary` for app font settings 1, 12, 13, 14, and 120 px and retain the UI family and semibold heading distinction.
- Global Search starts and updates its query editor at the primary role without changing query text, result data, collapsed directories, selected result, or debounce/search state.
- Conversation History updates its query and rename editors without changing buffers, active rename, selection, collapsed sections, or conversation ordering.
- Warp Drive constructs the title editor with `primary` and `Appearance::ui_font_family()`, then a size event updates the same handle without changing that family, buffer, focus, ordered items, focused item, sections, sorting, drag state, or cloud actions.
- Render/layout tests exercise exact 1 and 120 px values in all four tabs. At 1 px, roles remain positive and fixed icon/control minimums keep geometry operable; at 120 px, rows grow to the text's finite height and existing horizontal clipping, shrinking, ellipsis, or wrapping prevents overlap.

Where renderer-test helpers cannot expose text styles or measured bounds without coupling to private element internals, keep those assertions in the existing GUI integration/manual suite rather than adding production-only accessors.

### Manual / GUI integration validation

1. Set **Font size (px)** to exactly 1, the 12 px unit-scalar compatibility point, the current 13 px default, 14, and exactly 120. Open all four Tools pane tabs and verify their primary and subordinate roles move together.
2. Change size with a Project Explorer rename active, a Global Search query and result selected, a conversation rename active, and a Drive title edit active. Confirm buffers, selections, focus, and operations survive.
3. Exercise long names, deep nesting, badges, warnings, empty/error states, ignored files, search matches, timestamps, and Drive metadata at narrow and wide panel widths. At 1 px verify fixed icon/control minimums; at 120 px verify vertical remeasurement and existing horizontal overflow behavior without overlap.
4. Scroll and expand populated views, change size in both directions, and confirm logical selection, expansion, and scroll location remain stable.
5. Hide the panel, change the value, then show each tab and verify its first frame uses the current size.
6. Change notebook font size and window Zoom independently. Confirm notebook changes do not affect Tools pane roles, while Zoom scales the entire rendered window without rewriting `appearance.text.font_size`.
7. Verify Project Explorer, Global Search, Conversation History, and Warp Drive behavior with mouse and keyboard at the exact 1 and 120 px accepted boundaries. Tiny text need not be legible at 1 px and narrow-width labels need not be fully visible at 120 px, but interactions and layout must remain stable.
8. Run macOS, Windows, and Linux GUI smoke tests and a headless TUI smoke test.

## Invariant-to-test map

| Product invariant(s) | Primary coverage |
| --- | --- |
| 1, 14, 15, 16 | Existing-setting source test; notebook/Zoom independence manual pass |
| 2, 3, 4 | Shared typography-role tests and four-tab render assertions |
| 5, 6 | Appearance-event editor tests; hidden-then-show integration pass |
| 7 | Project Explorer render/editor/drag/error-state tests |
| 8 | Global Search query/result/status render tests |
| 9 | Conversation query/item/rename/section tests |
| 10 | Warp Drive index/row/title-editor size-and-family tests |
| 11 | Per-view state-preservation tests and populated-view manual pass |
| 12, 13, 16 | Exact 1/120 boundary layout tests and narrow-panel GUI pass |
| 17 | Scope diff plus dialog, toolbelt, and TUI smoke tests |

## Risks and mitigations

### Confusing configurable font size with fixed UI size or Zoom

`Appearance::ui_font_size()` is fixed, and Zoom already scales everything. The shared helper accepts `Appearance` and reuses `monospace_ui_scalar()`, which derives from `monospace_font_size()` and the fixed UI baseline. A unit test locks the unit-scalar and boundary behavior, while the implementation makes no `WindowSettings` or zoom-factor changes.

### Long-lived editors retaining stale sizes

Parent invalidation does not rewrite an existing editor's captured `TextOptions`. Each owning tab handles `MonospaceFontSizeChanged` and calls `set_font_size` on the existing editor handles. Tests assert handle and buffer continuity.

### Typography hierarchy drifting between tabs

Hard-coded 16/14/12/11 px values are replaced by one shared role calculation. A 12 px unit-scalar compatibility test locks the current values and prevents a tab from inventing a new independent derivation.

### Larger text clipping or destabilizing virtualized lists

Rows derive height from text and fixed icon minimums, and virtualized lists remeasure on invalidation. The change avoids model rebuilds and tests both layout bounds and logical state at the exact 1 and 120 px boundaries.

### Extreme accepted values exceeding ordinary usability

The existing control accepts 1 through 120 px. Proportional roles remain positive and ordered at both boundaries. At 1 px, fixed icons and controls establish minimum geometry even though text may be illegible; at 120 px, rows grow vertically and existing horizontal overflow behavior may hide part of long text in a narrow panel. Boundary tests require finite geometry, no overlap or panic, and preserved interactions rather than universal legibility or full horizontal visibility.

### Drive title editor inheriting the wrong family

`EditorView::single_line` defaults are not sufficient evidence that the title editor matches surrounding Tools pane text. Construct it with `TextOptions::ui_text(Some(primary), appearance)` (or explicitly set both properties) and assert that font-size events preserve the same UI family and editor handle.

### Scope expanding into every Drive-related surface

Only content rendered inside `ToolPanelView::WarpDrive` adopts the helper. Dialogs, menus, workflow editors, and full object panes remain on their existing typography; the changed-file review and GUI smoke pass enforce that boundary.

## Follow-ups

- If future feedback demonstrates a need for different sizes between terminal and Tools pane text, evaluate one Tools pane-wide setting rather than a Project Explorer-only key.
- New `ToolPanelView` implementations should use the shared typography roles for their tab content.
