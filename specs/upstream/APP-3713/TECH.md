# APP-3713: Tech Spec — Primary Info Selector in Vertical Tabs Settings Popup

## Problem

The vertical tabs settings popup contains a hardcoded "Group panes by" section with a single "Tab" option that has no effect. Per the product spec, this section should be replaced with a "Show first" selector that lets users choose whether terminal pane rows display the terminal command / agent conversation title or the working directory / git branch as the primary (top) line. The setting must affect both expanded and compact view modes and persist across sessions.

## Relevant code

- `app/src/workspace/tab_settings.rs (166-214)` — `VerticalTabsViewMode` enum and `TabSettings` group; the new setting will live here
- `app/src/workspace/view/vertical_tabs.rs (2004-2143)` — `render_settings_popup` and the "Group panes by" / "Tab" UI to be replaced
- `app/src/workspace/view/vertical_tabs.rs (1395-1447)` — `render_terminal_row_content` (expanded mode), assembles primary + secondary + tertiary lines
- `app/src/workspace/view/vertical_tabs.rs (1449-1479)` — `render_terminal_primary_line_for_view`, builds the primary line element from `TerminalView` data
- `app/src/workspace/view/vertical_tabs.rs (1585-1646)` — `render_terminal_secondary_line`, builds working directory + git branch line
- `app/src/workspace/view/vertical_tabs.rs (2183-2230)` — `render_compact_pane_row`, single-line compact rendering for terminal panes
- `app/src/workspace/action.rs (234-235)` — `ToggleVerticalTabsSettingsPopup` and `SetVerticalTabsViewMode` actions; new action for setting primary info will follow this pattern
- `app/src/workspace/view.rs (16988-17004)` — action handlers for the existing popup actions
- `app/src/workspace/view.rs (18467-18487)` — popup overlay rendering with `Dismiss` wrapper
- `app/src/workspace/view/vertical_tabs_tests.rs` — existing unit tests for primary line data logic
- `app/src/workspace/action_tests.rs` — tests for action `should_save_app_state_on_action`

## Current state

### Setting and persistence

`VerticalTabsViewMode` is a two-variant enum (`Compact`, `Expanded`) registered as a synced cloud setting in `TabSettings`. It uses the `implement_setting_for_enum!` macro with `SyncToCloud::Globally(RespectUserSyncSetting::Yes)` and hierarchy `"appearance.tabs"`. The new primary info setting will follow this exact pattern.

### Settings popup

`render_settings_popup` builds a fixed-width (200px) popup with:
1. A "Group panes by" header (sub-text color, 12px)
2. A "Tab" item (checkmark icon + "Tab" label, always selected)
3. A divider
4. A segmented control for compact/expanded

The "Group panes by" section is purely visual — it dispatches no actions and reads no settings.

### Expanded terminal row rendering

`render_terminal_row_content` builds a three-line column:
- **Primary**: calls `render_terminal_primary_line_for_view` → `render_terminal_primary_line`, which resolves `TerminalPrimaryLineData` through the precedence cascade (conversation title > CLI agent title > terminal title > last command > "New session"), then renders in main-text color.
- **Secondary**: calls `render_terminal_secondary_line(working_directory, git_branch)`, which renders working directory (start-clipped) + git branch (end-clipped) in sub-text color.
- **Tertiary**: kind badge + right-side badges (unchanged by this feature).

### Compact terminal row rendering

`render_compact_pane_row` calls `render_terminal_primary_line_for_view` with `Some(WarpIcon::Terminal)` as a prefix icon, producing a single-line row with the terminal icon + primary line data text. For agent terminals, the status icon comes from the `StatusText` variant of `TerminalPrimaryLineData`.

### Key render functions and their color contracts

- `render_terminal_primary_line` always uses `main_text_color`.
- `render_terminal_secondary_line` always uses `sub_text_color`.

When swapping lines, we need parameterized color rather than hardcoded main/sub text.

## Proposed changes

### 1. Add `VerticalTabsPrimaryInfo` setting

In `app/src/workspace/tab_settings.rs`, add a new enum and register it in `TabSettings`:

```rust
#[derive(Default, Debug, serde::Serialize, serde::Deserialize, PartialEq, Copy, Clone)]
pub enum VerticalTabsPrimaryInfo {
    #[default]
    Command,
    WorkingDirectory,
}
```

Register with `implement_setting_for_enum!` using the same sync/hierarchy as `VerticalTabsViewMode`:
- `SupportedPlatforms::ALL`
- `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- `hierarchy: "appearance.tabs"`

Add `vertical_tabs_primary_info: VerticalTabsPrimaryInfo` to the `TabSettings` group.

### 2. Add `SetVerticalTabsPrimaryInfo` action

In `app/src/workspace/action.rs`:
- Add variant `SetVerticalTabsPrimaryInfo(VerticalTabsPrimaryInfo)` next to `SetVerticalTabsViewMode`.
- In `should_save_app_state_on_action`, add it to the `false` arm (same as `SetVerticalTabsViewMode` — the setting is persisted via the settings framework, not workspace state).

In `app/src/workspace/view.rs`, add the handler in the same block as `SetVerticalTabsViewMode`:

```rust
SetVerticalTabsPrimaryInfo(primary_info) => {
    let primary_info = *primary_info;
    TabSettings::handle(ctx).update(ctx, |settings, ctx| {
        let _ = settings.vertical_tabs_primary_info.set_value(primary_info, ctx);
    });
    ctx.notify();
}
```

### 3. Add mouse state handles for option rows

In `VerticalTabsPanelState`, add two new `MouseStateHandle` fields for the popup option rows:

```rust
command_option_mouse_state: MouseStateHandle,
directory_option_mouse_state: MouseStateHandle,
```

Initialize with `Default::default()` in the `Default` impl.

### 4. Replace popup "Group by" section with "Show first" section

In `render_settings_popup`:

**Remove**: The `group_by_header` and `tab_item` variables and their children in the assembled column.

**Replace with**: A "Show first" header and two clickable option rows.

The header reuses the same styling as the current "Group panes by" header but with text `"Show first"`.

Each option row is a `Hoverable` wrapping a `Flex::row` with:
- A 16×16 checkmark icon (`WarpIcon::Check` in main-text color) if selected, or a 16×16 `Empty` spacer if not.
- An 8px gap.
- Label text ("Command / Conversation" or "Directory / Branch") in main-text color, 12px.

On hover: `fg_overlay_1` background, pointing hand cursor.
On click: dispatch `WorkspaceAction::SetVerticalTabsPrimaryInfo(variant)`. The popup does **not** close on click (unlike `ToggleVerticalTabsSettingsPopup`), so the user sees the checkmark move.

Read the current setting via `*TabSettings::as_ref(app).vertical_tabs_primary_info.value()` to determine which option gets the checkmark.

The divider and segmented control below remain unchanged.

### 5. Parameterize text color in terminal line renderers

The existing `render_terminal_primary_line` hardcodes `main_text_color` and `render_terminal_secondary_line` hardcodes `sub_text_color`. To support swapping, add a `text_color: WarpThemeFill` parameter to both functions so the caller controls which color each line uses.

**`render_terminal_primary_line`**: Add `text_color` param. Replace `main_text_color` usage for text rendering with the passed color. The status indicator icon color remains unchanged (it comes from `render_status_element` and is independent of text color).

**`render_terminal_secondary_line`**: Add `text_color` param. Replace `sub_text_color` usage with the passed color.

### 6. Update `render_terminal_row_content` (expanded mode)

Read the primary info setting:
```rust
let primary_info = *TabSettings::as_ref(app).vertical_tabs_primary_info.value();
```

Branch on the setting to decide line order:

- **`Command` (default)**: Call `render_terminal_primary_line_for_view` with `main_text_color` as first child, `render_terminal_secondary_line` with `sub_text_color` as second child. This is the current behavior.
- **`WorkingDirectory`**: Call `render_terminal_secondary_line` with `main_text_color` as first child, `render_terminal_primary_line_for_view` with `sub_text_color` as second child. This swaps the line order and colors.

The tertiary line is always appended last, unchanged.

### 7. Update `render_compact_pane_row` (compact mode)

Read the primary info setting. For terminal panes:

- **`Command`**: Current behavior — call `render_terminal_primary_line_for_view` with `Some(WarpIcon::Terminal)` prefix icon.
- **`WorkingDirectory`**: Render a single-line row with the terminal icon (or agent status icon) + working directory text (start-clipped). This can be done by building a `Flex::row` directly:
  - Add the kind icon (terminal icon for non-agent, status element for agent, `OzCloud` for ambient agent) — same icon logic as the current compact rendering.
  - Add the working directory text with `ClipConfig::start()`.

The icon is always determined by pane type / agent state, not by the primary info setting.

### 8. Update `render_terminal_primary_line_for_view`

Add a `text_color: WarpThemeFill` parameter. Pass it through to `render_terminal_primary_line`.

All existing call sites pass `theme.main_text_color(theme.background())` to preserve current behavior unless the caller is in the swapped path.

## End-to-end flow

1. User clicks the settings icon button in the vertical tabs control bar → `ToggleVerticalTabsSettingsPopup` is dispatched → `show_settings_popup` toggles to `true` → popup renders via `render_settings_popup`.
2. Popup shows "Show first" header with "Command / Conversation" checked (default). Segmented control below.
3. User clicks "Directory / Branch" → `SetVerticalTabsPrimaryInfo(WorkingDirectory)` is dispatched → `TabSettings.vertical_tabs_primary_info` is updated → `ctx.notify()` triggers re-render.
4. Popup stays open; checkmark moves to "Directory / Branch".
5. All terminal pane rows in the panel re-render: expanded rows swap primary/secondary lines; compact rows show working directory text.
6. Setting is persisted to cloud via the settings sync framework.
7. On next launch, `TabSettings::as_ref(app).vertical_tabs_primary_info.value()` returns `WorkingDirectory`, so rows render in the swapped order immediately.

## Risks and mitigations

- **Color regression**: Parameterizing text color introduces risk of passing the wrong color at call sites. Mitigation: only two call sites per function (one for each branch of the setting match). Unit test the data logic; visual testing catches color regressions.
- **Compact mode icon/text mismatch**: In `WorkingDirectory` compact mode, an agent terminal must still show the status icon (not the terminal icon) but with the working directory text. The icon resolution logic is already separated from the text logic in the current code, so this is straightforward.
- **Search correctness**: `terminal_pane_search_text_fragments` indexes both the primary text and working directory regardless of display order. No changes needed; search stays correct.

## Testing and validation

### Unit tests (`vertical_tabs_tests.rs`)

- Add tests for the "Show first" setting variants confirming `VerticalTabsPrimaryInfo::Command` and `VerticalTabsPrimaryInfo::WorkingDirectory` are `Default` and `Copy`/`Clone`/`PartialEq` as expected.

### Action tests (`action_tests.rs`)

- `SetVerticalTabsPrimaryInfo` should return `false` from `should_save_app_state_on_action` (same pattern as the existing `SetVerticalTabsViewMode` test).

### Manual validation

Per the product spec validation section: toggle between options in the popup, verify expanded/compact rendering, agent panes, persistence across restart, search correctness, and non-terminal pane immunity.

## Follow-ups

- When group-by functionality is implemented in the future, the "Show first" section and the group-by section can coexist in the same popup. The popup layout would then be: "Show first" options → divider → "Group by" options → divider → segmented control.
- A keyboard shortcut for toggling the primary info setting could be added later.
