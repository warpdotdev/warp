# APP-3648: Tech Spec — Vertical Tabs Control Bar

## Current State

The vertical tabs panel is rendered in `app/src/workspace/view/vertical_tabs.rs` via the free function `render_vertical_tabs_panel`. The panel layout is:

```
Resizable (drag-right edge)
  └─ Container (background + right border)
       └─ Flex::column
            └─ Shrinkable(ClippedScrollable::vertical(tab groups))
```

There is currently no control bar — tab groups fill the entire panel. The panel is resizable (min 200px, max 50% window width) with state managed in `VerticalTabsPanelState`.

The "new tab" split button in the top bar (`render_new_session_button` in `view.rs`) dispatches `WorkspaceAction::AddTab` on left-click and `WorkspaceAction::ToggleNewSessionMenu { position }` on right-click/chevron-click. The dropdown menu is rendered as an overlay in the workspace's main `Stack`, positioned at the stored `show_new_session_dropdown_menu` coordinates.

## Proposed Changes

### 1. Add state handles to `VerticalTabsPanelState`

`vertical_tabs.rs (struct VerticalTabsPanelState)`:
- `search_mouse_state: MouseStateHandle` — drives the tooltip on the search input
- `add_tab_mouse_state: MouseStateHandle` — drives the plus button hover/click states

Initialize both with `Default::default()` in the existing `Default` impl.

### 2. Add `render_control_bar` function

New free function in `vertical_tabs.rs`:

```
render_control_bar(state, app) -> Box<dyn Element>
```

Layout: `Flex::row` with `CrossAxisAlignment::Center`, padded with horizontal `GROUP_HORIZONTAL_PADDING` (12px) and vertical padding ~8px.

**Search input** (left, fills remaining space via `Shrinkable`):
- `Hoverable` wrapping a `Container` styled as an inert search field:
  - Left: `WarpIcon::Search` magnifying glass icon (12px, sub-text color)
  - Right: `Text::new_inline("Search tabs...", ...)` placeholder in sub-text color, clipped
  - Background: `fg_overlay_1` or similar subtle fill; rounded corners
  - Height: ~24px to match icon button sizing
- On hover: show tooltip "Not yet implemented" via `ui_builder.tool_tip_on_element(...)` (overlay variant, so it's not clipped by the panel's `Clipped` wrapper)
- No click handler, no cursor change — remains inert

**Plus button** (right, fixed width):
- Use `icon_button(appearance, Icon::Plus, false, state.add_tab_mouse_state.clone())`
- Wrap in a `Hoverable` to add a tooltip with "New Tab" + keybinding sublabel
- `.on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::AddTab))`
- `.on_right_click(|ctx, _, position| ctx.dispatch_typed_action(WorkspaceAction::ToggleNewSessionMenu { position }))`
- Wrap in `SavePosition` with a dedicated position ID (e.g. `"vertical_tabs_add_tab_button"`) so the dropdown menu can anchor to it

### 3. Integrate into `render_vertical_tabs_panel`

Change the `Flex::column` to include the control bar as the first child, outside the scrollable:

```
Flex::column()
    .with_main_axis_size(MainAxisSize::Max)
    .with_child(render_control_bar(state, app))          // NEW — fixed at top
    .with_child(Shrinkable::new(1., scrollable_groups))  // existing — scrolls
    .finish()
```

The control bar stays fixed because it's a sibling of the `Shrinkable`-wrapped scrollable, not inside it.

### 4. Dropdown menu positioning

No changes needed to the workspace-level menu rendering. The existing `ToggleNewSessionMenu { position }` flow stores the click position and renders the menu in the workspace `Stack` overlay. The right-click on the plus button passes its screen position directly, so the menu will appear below the button naturally. If the menu clips at the panel edge, `ParentOffsetBounds::WindowByPosition` (already used for the shell-selector variant) will reposition it within the window.

## Edge Cases Handled

- **Narrow panel (200px min)**: Search input uses `Shrinkable` so it compresses; plus button has fixed width and remains usable.
- **Focus**: No focus handlers on the search input; clicking it does nothing.
- **Tooltip clipping**: Use the overlay tooltip variant (`overlay_tool_tip_on_element`) so the tooltip renders above the `Clipped` scrollable.

## Parallelism

This is a single-file change (`vertical_tabs.rs`) with a minor dependency on imports from `buttons.rs` and `icons.rs`. No sub-agents needed — the work is sequential and localized.

## Files Changed

- `app/src/workspace/view/vertical_tabs.rs` — all substantive changes (state, rendering, control bar)
