# APP-3656: Tech Spec — Compact Mode + View Toggle

## Problem

The vertical tabs panel currently renders every pane as a multi-line card (2–4 lines each). With many tabs/panes the panel requires heavy scrolling. The product spec (APP-3656 PRODUCT.md) defines a compact single-line rendering mode and a control-bar settings popup to toggle between compact and expanded views. This spec translates that behavior into concrete implementation changes.

## Relevant code

- `app/src/workspace/view/vertical_tabs.rs` — all rendering for the vertical tabs panel; `VerticalTabsPanelState`, `render_control_bar`, `render_pane_row`, `render_terminal_row_content`, `TypedPane`, `PaneProps`
- `app/src/workspace/tab_settings.rs` — `TabSettings` group and `define_settings_group!` / `implement_setting_for_enum!` macros for persisted settings
- `app/src/workspace/action.rs` — `WorkspaceAction` enum for dispatching UI events
- `app/src/workspace/view.rs` — `Workspace` struct fields, `show_new_session_dropdown_menu` pattern for popup overlays
- `ui/src/ui_components/segmented_control.rs` — `SegmentedControl<T>` view, `RenderableOptionConfig`, `SegmentedControlEvent`
- `warp_core/src/ui/icons.rs` — `Icon` enum and SVG path mappings
- `app/src/ai/conversation_status_ui.rs` — `render_status_element` for agent status badges
- `app/src/terminal/view/tab_metadata.rs` — `terminal_title_from_shell()`, `display_working_directory()`, `selected_conversation_display_title()`
- `app/src/terminal/view/pane_impl.rs (926-973)` — `is_ambient_agent_session()`, `selected_conversation_status()`, `selected_conversation_display_title()`

## Current state

The panel layout is:

```
Resizable (drag-right edge)
  └─ Container (background + right border)
       └─ Flex::column
            ├─ render_control_bar (search input + new-tab split button)
            └─ Shrinkable(ClippedScrollable::vertical(tab groups))
```

`render_pane_row` dispatches to either `render_terminal_row_content` (for terminal panes, producing 3 lines: primary dir+branch, secondary agent/title, tertiary kind+badges) or an inline multi-line layout for non-terminal panes (title, subtitle, kind badge row). There is no concept of a view mode — every row is always expanded.

The `SegmentedControl<T>` view is fully reusable. It takes a vec of options, a render config callback, and emits `SegmentedControlEvent::OptionSelected(T)` on segment click. Icon-only segments are supported via `RenderableOptionConfig` with `label: None` and a populated `icon_path`.

## Proposed changes

### 1. Add `VerticalTabsViewMode` setting

In `tab_settings.rs`, add an enum and wire it into `TabSettings`:

```rust
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq, Copy, Clone)]
pub enum VerticalTabsViewMode {
    Compact,
    #[default]
    Expanded,
}
```

Register with `implement_setting_for_enum!` using `SyncToCloud::Globally(RespectUserSyncSetting::Yes)` and hierarchy `"appearance.tabs"`. Add `vertical_tabs_view_mode: VerticalTabsViewMode` to the `define_settings_group!` block.

### 2. Add icon variants

The segmented control needs two icons not currently in the `Icon` enum:

- `Menu` already exists (`layout-left.svg`) but maps to the sidebar icon, not a hamburger/list icon. Verify `bundled/svg/` for a `menu-01.svg` or similar. If absent, add `ListMenu` → `bundled/svg/list-menu.svg` (or the closest available SVG). The Figma mock's `menu-01` icon is a standard 3-line hamburger.
- `Grid` already exists → `bundled/svg/grid.svg`. Should work for the expanded segment.
- For the settings button, `Settings` → `bundled/svg/settings.svg` already exists. Check whether the Figma `settings-04` sliders icon matches visually; if not, add a `Settings04` variant mapping to a new SVG.

If new SVGs are needed, add them to `resources/bundled/svg/` and extend the `Icon` enum + `From<Icon> for &'static str` match.

### 3. Add state to `VerticalTabsPanelState`

```rust
pub(super) struct VerticalTabsPanelState {
    // ... existing fields ...
    settings_button_mouse_state: MouseStateHandle,
    settings_popup_mouse_state: MouseStateHandle,
    show_settings_popup: bool,
}
```

Initialize all with `Default::default()` / `false`.

### 4. Add `WorkspaceAction` variants

```rust
pub enum WorkspaceAction {
    // ... existing ...
    ToggleVerticalTabsSettingsPopup,
    SetVerticalTabsViewMode(VerticalTabsViewMode),
}
```

`ToggleVerticalTabsSettingsPopup` toggles `show_settings_popup` on the panel state and calls `ctx.notify()`.

`SetVerticalTabsViewMode` writes the new value through `TabSettings` (same as other setting mutations). Both actions should be listed in the `should_save_app_state_on_action` match as `false` (no workspace state save needed).

### 5. Update `render_control_bar`

Insert the settings button between the search bar and the new-tab button. Layout becomes:

```
Flex::row [search_bar (Shrinkable)] [settings_button] [new_tab_button]
```

The settings button:
- Uses `Hoverable` wrapping an icon button with `WarpIcon::Settings` (or `Settings04`) at 16×16 in a 20×20 container with 2px padding.
- Background is `fg_overlay_3` when `state.show_settings_popup` is true, `fg_overlay_2` on hover, transparent otherwise.
- `on_click` dispatches `WorkspaceAction::ToggleVerticalTabsSettingsPopup`.
- Wrap in a `Stack` to position a tooltip ("View options") as an overlay when hovered and popup is closed.
- Wrap the whole button in a `SavePosition` with ID `"vertical_tabs_settings_button"` for popup anchoring.

### 6. Render the settings popup

Inside `render_vertical_tabs_panel`, after building the panel content `Flex::column`, wrap the result in a `Stack`. When `state.show_settings_popup` is true, add a positioned overlay child:

```rust
if state.show_settings_popup {
    let popup = render_settings_popup(state, app);
    stack.add_positioned_overlay_child(
        popup,
        OffsetPositioning::offset_from_parent(
            vec2f(0., 4.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::BottomLeft,
            ChildAnchor::TopLeft,
        ),
    );
}
```

**Alternative (simpler)**: Rather than making the popup a positioned overlay on the Stack, render it inline as an absolutely-positioned element anchored to the settings button's `SavePosition`. Both approaches work; use whichever is more consistent with existing popup patterns in the file.

`render_settings_popup` builds a `Container` styled as a popover (border `neutral_4`, background with subtle overlay, corner radius 6px, drop shadow). Contents:

```
Container (popup styling)
  └─ Padding(16px horizontal, 8px vertical)
       └─ SegmentedControl (rendered inline via ChildView)
```

The `SegmentedControl<VerticalTabsViewMode>` is **not** stored as a `ViewHandle` — it is too lightweight for that. Instead, build the segmented control UI manually using the same pattern: two `Hoverable` icon buttons inside a rounded container, with the active segment highlighted. This avoids needing a persistent `ViewHandle` on `VerticalTabsPanelState` and keeps the popup self-contained.

Concretely, `render_settings_popup` returns:

```rust
fn render_settings_popup(state: &VerticalTabsPanelState, app: &AppContext) -> Box<dyn Element> {
    let current_mode = *TabSettings::as_ref(app).vertical_tabs_view_mode;
    // Build two icon buttons (compact / expanded), highlight the active one
    // on_click dispatches WorkspaceAction::SetVerticalTabsViewMode(...)
}
```

**Dismiss handling**: The popup should close on:
- Outside click: Wrap the popup's overlay in a dismiss layer (same pattern as existing menus — e.g., a transparent full-window Hoverable behind the popup that dispatches close on click).
- Escape: Add a keybinding handler or check focus loss.
- Re-click on settings button: Already handled by `ToggleVerticalTabsSettingsPopup`.

### 7. Add `render_compact_pane_row`

New function in `vertical_tabs.rs`:

```rust
fn render_compact_pane_row(props: PaneProps<'_>, app: &AppContext) -> Box<dyn Element>
```

This function shares the same `PaneProps`, `Hoverable` wrapper, click handler, background/border logic, and cursor as `render_pane_row`. The difference is only the content layout.

**Approach**: Extract the shared interaction wrapper into a helper, then call either compact or expanded content rendering inside it. Specifically:

```rust
fn render_pane_row_wrapper(
    props: PaneProps<'_>,
    is_compact: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    // ... Hoverable + click + right-click + cursor + background logic (unchanged) ...
    let content = if is_compact {
        render_compact_content(&props, app)
    } else {
        render_expanded_content(&props, app)  // current logic extracted
    };
    // ... container with padding, border, corner radius ...
}
```

**Compact content layout**:

```
Flex::row (CrossAxisAlignment::Center, spacing 4px)
  ├─ [icon 16×16]         // type-dependent
  ├─ [title text, 12px]   // Shrinkable, single line, ellipsis
  └─ [indicator 16×16]?   // optional, right-aligned (unsaved circle for code panes)
```

**Compact padding**: 8px vertical, 12px horizontal (vs 12px uniform in expanded). This matches the Figma mock's `py-8 px-12`.

**Per-type icon and title logic**:

For terminal panes, read from `TerminalView`:

```rust
let (icon, title) = if let Some(view_handle) = terminal_view_handle.as_ref() {
    let tv: &TerminalView = view_handle.as_ref(app);
    let conversation_title = tv.selected_conversation_display_title(app);
    let conversation_status = tv.selected_conversation_status(app);
    let is_ambient = tv.is_ambient_agent_session(app);

    if let Some(conv_title) = conversation_title {
        // Agent session: status icon + conversation title
        let icon_element = if let Some(status) = conversation_status {
            render_status_element(&status, 12., appearance)
        } else if is_ambient {
            WarpIcon::OzCloud icon element
        } else {
            WarpIcon::Oz icon element
        };
        (icon_element, conv_title)
    } else {
        // Non-agent terminal: terminal icon + terminal title (NOT pwd)
        let terminal_title = tv.terminal_title_from_shell();
        (WarpIcon::Terminal icon element, terminal_title)
    }
} else {
    // Non-terminal pane: type icon + pane title (already in props.title)
    (typed.icon() icon element, props.title.clone())
};
```

For the unsaved indicator (code panes only): append a `CircleFilled` icon (16×16, sub-text color) to the right of the row when `typed.badge(app).is_some()`.

### 8. Integrate view mode into `render_pane_row` call sites

In `render_tab_group`, where `render_pane_row(pane_props, app)` is called inside the rows loop, read the current view mode and branch:

```rust
let view_mode = *TabSettings::as_ref(app).vertical_tabs_view_mode;
let row = match view_mode {
    VerticalTabsViewMode::Compact => render_compact_pane_row(pane_props, app),
    VerticalTabsViewMode::Expanded => render_pane_row(pane_props, app),
};
rows.add_child(row);
```

Or use the combined `render_pane_row_wrapper` approach from section 7.

### 9. Handle `WorkspaceAction` in `view.rs`

In the `WorkspaceAction` match in `handle_action`:

```rust
WorkspaceAction::ToggleVerticalTabsSettingsPopup => {
    self.vertical_tabs_panel.show_settings_popup =
        !self.vertical_tabs_panel.show_settings_popup;
    ctx.notify();
}
WorkspaceAction::SetVerticalTabsViewMode(mode) => {
    TabSettings::handle(ctx).update(ctx, |settings, ctx| {
        settings.vertical_tabs_view_mode.set_value(mode, ctx);
    });
    ctx.notify();
}
```

## End-to-end flow

1. User clicks the settings icon button in the vertical tabs control bar.
2. `ToggleVerticalTabsSettingsPopup` is dispatched → `show_settings_popup` flips to `true` → panel re-renders.
3. The panel's `Stack` now includes the popup overlay anchored below the button.
4. User clicks the compact segment → `SetVerticalTabsViewMode(Compact)` dispatches → `TabSettings` writes the new value → settings sync triggers → panel re-renders.
5. `render_tab_group` reads the updated `VerticalTabsViewMode` and calls `render_compact_pane_row` for each pane.
6. Each compact row renders: icon + title in a single line.
7. Popup auto-closes because the click dismisses it (or the user clicks outside / presses Escape).
8. On next launch, the setting is loaded from the synced settings store, and the panel renders in compact mode immediately.

## Risks and mitigations

**Icon availability**: The Figma mock references `settings-04`, `menu-01`, and `grid-01` icons. The existing `Icon::Menu` maps to `layout-left.svg` (sidebar icon), not a hamburger. Mitigation: audit `resources/bundled/svg/` for matching SVGs; add new variants if needed. If exact icons are unavailable, use the closest existing ones (`Settings`, `Menu`, `Grid`) and iterate visually.

**Segmented control as ViewHandle**: Creating a `ViewHandle<SegmentedControl<VerticalTabsViewMode>>` requires storing it on `VerticalTabsPanelState` and wiring up subscriptions. This adds complexity for a two-button toggle. Mitigation: build the toggle inline as two `Hoverable` icon buttons inside a styled container, dispatching `WorkspaceAction::SetVerticalTabsViewMode` directly. This avoids the `ViewHandle` lifecycle overhead.

**Popup dismiss on outside click**: The existing popup patterns in the workspace (e.g., `show_new_session_dropdown_menu`) rely on `Menu` views that handle their own focus/dismiss. Our popup is simpler (no menu items). Mitigation: render a full-window transparent `Hoverable` behind the popup that dispatches `ToggleVerticalTabsSettingsPopup` on click. This is the same "click-away backdrop" pattern used elsewhere.

**Compact row height consistency**: Different pane types produce different icon heights (status badges have padding/background, plain icons don't). Mitigation: use `ConstrainedBox` to fix all icons to 16×16 and fix the row height via `ConstrainedBox::with_height` or consistent padding.

## Testing and validation

- **Visual verification**: Build and run with `cargo run`, open several tabs with different pane types (terminal, code, agent, settings, notebook), toggle between compact and expanded. Verify single-line rendering and correct icons per pane type.
- **Setting persistence**: Switch to compact, restart (`cargo run` again), verify the panel starts in compact mode.
- **Popup behavior**: Click the settings icon, verify popup appears below it. Click outside, verify it closes. Click the icon again, verify it toggles.
- **Edge cases**:
  - Empty tab group → "No tabs open" message should display in both modes.
  - Collapsed groups → remain collapsed across mode switches.
  - Minimum panel width (200px) → compact rows truncate gracefully.
  - Tab colors → tint renders correctly on compact rows.

## Follow-ups

- **Group-by options**: The popup will eventually include "Group panes by" options (Tab, Directory/Environment, Branch, Status) above the segmented control, per the Figma mock. The popup container is designed to accommodate additional content above the toggle.
- **Keyboard shortcut**: A keybinding to toggle compact/expanded mode can be added later as a new action binding.
- **Search functionality**: The search input remains inert. When implemented, it should filter in both compact and expanded modes.
- **Icon audit**: Once the feature is visually reviewed, confirm the chosen icons match the Figma mock and swap SVGs if needed.
