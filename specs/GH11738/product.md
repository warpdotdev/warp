# Product Spec: Make Tools pane text respect the app font size (GH11738)

**Issue:** [warpdotdev/warp#11738](https://github.com/warpdotdev/warp/issues/11738)

**Figma:** none provided

## Summary

Make text in every Tools pane tab scale from Warp's existing **Font size (px)** preference instead of adding a Project Explorer-only font-size setting. The Project Explorer, Conversation History, Global Search, and Warp Drive update together when the existing value changes, using the same UI-scaling convention already used elsewhere in the app.

This gives users one predictable control for text density across terminal and Tools pane content, avoids a growing collection of per-tool settings, and leaves window Zoom available when the user wants to scale the entire interface.

## Problem

The Project Explorer currently renders file and directory names at a fixed 14 px. Other Tools pane tabs contain the same class of fixed 14 px primary text plus fixed smaller or larger text roles. Those values do not follow the font-size preference that users already change in **Settings → Appearance → Text**.

As a result, changing Warp's font size changes terminal and other app text while file names, search results, conversation entries, and Warp Drive items remain at their hard-coded sizes. The original issue proposed an independent Project Explorer control, but applying that pattern separately to Project Explorer, Global Search, and future tools would make users coordinate several settings to achieve one readable Tools pane.

## Goals

- Make all current Tools pane tabs respond to the existing app font-size preference.
- Keep one source of truth for terminal text size and the UI scale applied to Tools pane typography.
- Preserve each tab's existing visual hierarchy between primary, secondary, heading, and overline text.
- Apply changes immediately without reopening a tab, panel, workspace, or window.
- Preserve selection, expansion, search, rename, drag, scroll, and focus state while text reflows.
- Keep layout finite, non-overlapping, and operable at the full range already accepted by the existing setting.

## Non-goals

- Adding `project_explorer_font_size`, `tools_panel_font_size`, or any other new setting or settings-file key.
- Letting individual Tools pane tabs use different user-selected sizes.
- Changing the UI font family, font weight, color, or the terminal/notebook font-family behavior.
- Making notebook font size control the Tools pane.
- Replacing or redefining **Zoom**, which continues to scale the complete window after layout.
- Scaling toolbelt icons, chevrons, file-type icons, indentation, horizontal padding, borders, or the panel resize handle with the font-size preference.
- Changing text in dialogs, menus, full editor panes, or workflows opened from a Tools pane tab.
- Changing sorting, selection, expansion, keyboard commands, drag-and-drop semantics, filesystem operations, cloud operations, or search behavior.
- Adding this GUI behavior to the headless TUI.

## User experience

The existing **Font size (px)** field in **Settings → Appearance → Text** remains the only control. No Project Explorer row is added to Code settings and no Tools pane-specific row is added to Appearance settings.

When the value changes, visible Tools pane content updates in the same interaction:

- Project Explorer file and directory names, the inline create/rename editor, drag previews, and in-pane unavailable/error heading and body use the derived primary role. The error heading keeps its semibold distinction.
- Global Search's query, result names and matching lines use the primary size; paths, counts, and status text retain their smaller secondary role.
- Conversation History's search, conversation names, empty states, and inline rename use the corresponding primary or heading role; timestamps, subtext, controls, and section labels retain their smaller roles.
- Warp Drive's index items, section content, empty states, and inline title editor use the corresponding roles. Dialogs or full panes launched from Warp Drive are unchanged.

Each existing Tools pane role scales by the app's established font-size scalar. When **Font size (px)** is 12 px, the scalar is 1 and the current 14 px primary, 12 px secondary, 16 px heading, and 11 px overline values remain unchanged. At other values, all four baselines are multiplied by `font size / 12`, preserving their current hierarchy while following the preference consistently with other app UI. At the current 13 px default, for example, those roles become approximately 15.17, 13, 17.33, and 11.92 px; this is an intentional consequence of reusing the existing UI-scaling convention rather than treating the selected terminal size as the primary Tools pane size.

The existing field accepts whole-pixel values from **1 through 120**, inclusive. At 1 px, derived text roles remain positive and ordered but may be intentionally too small to read; fixed icons and controls retain their current minimum geometry. At 120 px, rows grow vertically and existing single-line clipping, shrinking, ellipsis, or wrapping handles horizontal pressure. The feature guarantees stable layout and interaction at both boundaries, not that every character is visible in a narrow panel.

If a Tools pane tab is hidden when the value changes, it uses the current value on its first frame when shown. Existing interactive state remains attached to the same logical item even if larger or smaller text changes row heights and the amount of visible content.

## Product invariants

1. The existing `appearance.text.font_size` value is the sole persisted source for terminal and Tools pane text size; this feature creates no new setting, storage key, migration, or sync behavior.
2. The behavior covers every current `ToolPanelView`: Project Explorer, Conversation History, Global Search, and Warp Drive.
3. Tools pane roles use their current 16/14/12/11 px baselines multiplied by the existing app font-size scalar (`appearance.text.font_size / 12`). The resulting roles are positive and preserve `heading > primary > secondary > overline`, including at the setting's 1 and 120 px boundaries.
4. Tools pane text keeps the existing UI font family. Following the app font size does not switch file names, search results, conversations, or Drive items to the terminal font family.
5. A valid font-size change updates every visible Tools pane tab immediately. A panel, tab, workspace, window, and app restart are not required.
6. A tab created, reactivated, or shown after a change uses the current value on its first frame and never flashes at a hard-coded fallback size.
7. Project Explorer ordinary and ignored names, active create/rename text, drag-preview text, and in-pane unavailable/error heading and body use the primary size. The error heading remains semibold so its current hierarchy does not require an independent size.
8. Global Search query and primary result text use the primary size; path, result-count, and status text remain visually subordinate at every supported value.
9. Conversation History query, item title, zero-state, active rename, timestamp/subtext, action, and section-label text all use roles derived from the same scalar while retaining their current hierarchy.
10. Warp Drive index rows and tab-local primary, secondary, heading, warning, and inline title text all use roles derived from the same scalar. Separate dialogs and full panes launched from Drive are outside this behavior.
11. A font-size change does not rebuild or reorder Project Explorer files, search results, conversation data, or Drive data. Selection, expansion, collapsed sections, pending edits, query text, focus, and logical scroll position remain unchanged except for clamping required by new layout bounds.
12. Text remains single-line where it is single-line today and retains existing clipping, shrinking, ellipsis, or wrapping behavior. At 120 px, a narrow panel may show only part of a single-line label, but text must not overlap icons, controls, another row, or the scrollbar.
13. Rows remeasure after a change. Text must not clip vertically; fixed-size icons remain centered in the remeasured row and continue to meet their existing click targets. At 1 px, icon and control minimums, not the tiny text, may determine row height.
14. Terminal font size and the Tools pane UI scalar derive from the same saved value. Notebook font size remains governed by its current setting, and changing the Tools pane behavior does not rewrite notebook state.
15. Window Zoom remains independent. It continues to scale the already-laid-out Tools pane and does not rewrite `appearance.text.font_size`.
16. The existing settings field retains its current inclusive 1–120 integer validation, persistence, local-only behavior, keyboard operation, and accessibility semantics.
17. Toolbelt chrome, menus, dialogs, full editor panes, and the TUI are unaffected.

## Validation

- Change **Font size (px)** to exactly 1, the 12 px unit-scalar compatibility point, the current 13 px default, 14, and exactly 120 and compare all four Tools pane tabs in the same window.
- In Project Explorer, verify ordinary, ignored, selected, hovered, deeply nested, dragged, actively renamed, and unavailable/error states.
- In Global Search, verify the query, file and directory rows, matching lines, paths, counts, progress/error text, capped-results notice, and empty states.
- In Conversation History, verify search, active/past section labels, conversation titles, timestamps/subtext, zero/no-match states, actions, and an active rename.
- In Warp Drive, verify the main index, section headers and items, secondary metadata, warning/empty states, and an active title edit without changing launched dialogs or full panes.
- At 1 px, verify derived roles stay positive and ordered while fixed icons/controls retain minimum geometry. At 120 px, verify rows remeasure vertically and narrow-width text uses its existing horizontal overflow behavior without overlap.
- Change the value while each tab is scrolled, selected, filtered, expanded, or editing. Confirm state and focus survive and row heights remeasure without vertical clipping or overlap.
- Hide the Tools pane, change the value, then reopen each tab and confirm its first rendered frame uses the current value.
- Change notebook font size and Zoom independently and verify neither rewrites the app font-size value or breaks the Tools pane hierarchy.
- Smoke test macOS, Windows, and Linux GUI builds and confirm the headless TUI is unchanged.

## Open questions

None.
