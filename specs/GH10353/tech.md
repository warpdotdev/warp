# TECH.md — Opt-in native left-drag selection in TUIs

Issue: https://github.com/warpdotdev/warp/issues/10353
Product spec: `specs/GH10353/product.md`

## Problem

Warp already decides whether mouse input in a full-screen or mouse-reporting terminal application should be intercepted by Warp or forwarded to the application. Today, `should_intercept_mouse` primarily treats `Shift` and shared-session reader state as native-selection escape hatches, then forwards mouse input when SGR mouse reporting and a mouse-tracking mode are active and `terminal.mouse_reporting_enabled` is true.

The requested behavior needs a narrower intercept path: when the new opt-in setting is enabled, bare left-button down, drag, and up events should be handled by Warp's native selection path even while the TUI is using mouse reporting. Non-left and non-bare mouse events should continue through the current reporting logic.

## Relevant code

- `app/src/terminal/alt_screen_reporting.rs` — defines the `AltScreenReporting` settings group that currently contains `mouse_reporting_enabled`, `scroll_reporting_enabled`, and `focus_reporting_enabled`.
- `app/src/terminal/alt_screen/mod.rs` — contains `should_intercept_mouse` and `should_intercept_scroll`, the central decision helpers for mouse and scroll interception in alt-screen contexts.
- `app/src/terminal/alt_screen/alt_screen_element.rs (264-438)` — dispatches alt-screen left mouse down, right mouse down, mouse up, and mouse drag behavior through `should_intercept_mouse`.
- `app/src/terminal/block_list_element.rs (1584-1730)` — forwards left mouse down and up events to active long-running command blocks when the block is eligible for TUI mouse reporting.
- `app/src/terminal/block_list_element.rs (2005-2033)` — forwards left drag events to active long-running command blocks when eligible.
- `app/src/settings_view/features_page.rs (135-470)` — registers command-palette toggle action pairs for Features settings.
- `app/src/settings_view/features_page.rs (565-608)` — defines `FeaturesPageAction`, including existing AltScreenReporting toggles.
- `app/src/settings_view/features_page.rs (871-879)` — emits telemetry for existing mouse, scroll, and focus reporting toggles.
- `app/src/settings_view/features_page.rs (1487-1507)` — handles existing AltScreenReporting toggle actions.
- `app/src/settings_view/features_page.rs (2685-2699)` — adds the AltScreenReporting widgets to the Features → Terminal category.
- `app/src/settings_view/features_page.rs (6440-6571)` — renders existing mouse, scroll, and focus reporting toggle widgets.
- `app/src/settings_view/mod.rs (385-386)` — defines existing context flags for scroll and focus reporting.
- `app/src/workspace/view.rs (19732-19737)` — adds toggle-setting context flags based on current setting values.
- `app/src/workspace/global_actions.rs (64-116)` and `app/src/app_menus.rs (398-432)` — existing global actions and menu items for mouse, scroll, and focus reporting. These can be mirrored if product wants a menu item, but the current product requirement only calls for settings, command palette, and context flag integration.
- `app/src/terminal/view_tests.rs (1240-1254)` — existing alt-screen SGR mouse selection test that asserts current `should_intercept_mouse` behavior.

The contributor reference branch at `spalagu/warp:feat/left-drag-select-default` demonstrates the expected rough scope: a new AltScreenReporting setting, a `should_intercept_mouse` signature change that receives left-button context, seven call-site updates, settings UI exposure, a context flag, and an existing test update.

## Current state

`AltScreenReporting` is a settings group with three synced, public, all-platform boolean settings:

- `terminal.mouse_reporting_enabled`, default `true`
- `terminal.scroll_reporting_enabled`, default `true`
- `terminal.focus_reporting_enabled`, default `true`

`should_intercept_mouse(model, shift, ctx)` returns `true` when Warp should handle the mouse event natively. It currently:

1. immediately intercepts for shared-session readers or `Shift`
2. checks whether alt-screen or terminal mouse tracking is active
3. reads `terminal.mouse_reporting_enabled`
4. forwards instead of intercepting when SGR mouse mode, mouse tracking, and mouse reporting are all enabled

Alt-screen left down begins native selection when `should_intercept_mouse` returns `true`; otherwise it clears any alt selection and forwards `TerminalAction::AltMouseAction`. Mouse drag and mouse up similarly update or end native selection only when the element is already selecting, and forward to the TUI when interception is false.

Active long-running command blocks use similar reporting decisions in `BlockListElement` so mouse input can reach a TUI running in the block list.

Settings exposure for AltScreenReporting is split across:

- the setting definition in `alt_screen_reporting.rs`
- `FeaturesPageAction`
- command-palette `ToggleSettingActionPair` entries
- telemetry mapping
- the `FeaturesPageView` action handler
- widgets in the Terminal category
- context flags in `settings_view::flags` and `Workspace::add_toggle_setting_context_flags`

## Proposed changes

### 1. Add the setting

Extend `AltScreenReporting` in `app/src/terminal/alt_screen_reporting.rs` with:

- setting name: `native_left_drag_select_enabled`
- setting type: `NativeLeftDragSelectEnabled`
- Rust type: `bool`
- default: `false`
- supported platforms: `SupportedPlatforms::ALL`
- sync behavior: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`
- private: `false`
- TOML path: `terminal.native_left_drag_select_enabled`
- description: explain that bare left-button selection in mouse-reporting full-screen apps is handled by Warp's native selection so `Cmd+C` can copy, while other mouse events keep the normal reporting behavior

This should follow the exact settings-group pattern used by the existing `mouse_reporting_enabled`, `scroll_reporting_enabled`, and `focus_reporting_enabled` fields.

### 2. Refine mouse interception input

Change `should_intercept_mouse` in `app/src/terminal/alt_screen/mod.rs` so callers can specify whether the event is a bare left-button event that is eligible for the new native-selection override.

Recommended signature:

- `model: &TerminalModel`
- `shift: bool`
- `bare_left_button: bool`
- `ctx: &AppContext`

The helper should preserve existing early interception for shared-session readers and `Shift`. After loading `AltScreenReporting`, it should return `true` for `bare_left_button && native_left_drag_select_enabled` before applying the normal SGR mouse forwarding check.

The "bare" part matters because the product spec preserves non-Shift modifier gestures. Callers should pass `bare_left_button = true` only for left-button down, up, and drag events with no modifiers other than the existing `Shift` path. If the implementation instead uses a more generic `is_left_button` boolean, the helper must also receive enough modifier context to avoid newly intercepting non-Shift modifier-click or modifier-drag gestures.

`should_intercept_scroll` should call the updated helper with `bare_left_button = false` so scroll reporting remains governed by the existing scroll and mouse reporting settings.

### 3. Update alt-screen call sites

Update `app/src/terminal/alt_screen/alt_screen_element.rs` call sites:

- `left_mouse_down` passes `bare_left_button = true` only for unmodified left-button down. When interception is true, it keeps dispatching `TerminalAction::AltSelect(SelectAction::Begin { ... })`; when false, it keeps dispatching `MaybeClearAltSelect` and `AltMouseAction`.
- `mouse_dragged` passes `bare_left_button = true` only for an unmodified left-button drag. When a native selection is active, selection update behavior remains unchanged.
- `mouse_up` passes `bare_left_button = true` only for an unmodified left-button release. Ending the native selection should remain tied to `self.is_terminal_selecting`.
- `right_mouse_down` passes `bare_left_button = false` so right-click behavior is unchanged.

The implementation should avoid changing link hover, secret hover, mouse motion, scroll, context-menu, or selection rendering paths.

### 4. Update active block-list call sites

Update `app/src/terminal/block_list_element.rs` call sites that currently forward left mouse input to active long-running blocks:

- left mouse down forwarding check around the active long-running block path
- left mouse up forwarding check
- left drag forwarding check

Each should pass `bare_left_button = true` only for unmodified left-button events. Existing native block text selection, block selection, find-bar behavior, rich-content behavior, and snackbar hit testing should not change.

### 5. Add Settings → Features → Terminal UI

Update `app/src/settings_view/features_page.rs` to mirror existing AltScreenReporting settings:

- add `NativeLeftDragSelectEnabled` to the AltScreenReporting imports
- add `FeaturesPageAction::ToggleNativeLeftDragSelect`
- add telemetry mapping with action name `ToggleNativeLeftDragSelect` and the current setting value
- handle the action by calling `toggle_and_save_value(ctx)` on `reporting.native_left_drag_select_enabled` and notifying the view
- add a `NativeLeftDragSelectWidget` near the other Terminal reporting widgets when the setting is supported on the current platform
- render the widget with label "Native Left-Drag Selection"
- use `LocalOnlyIconState::for_setting(NativeLeftDragSelectEnabled::storage_key(), NativeLeftDragSelectEnabled::sync_to_cloud(), ...)`
- set search terms that include "native left drag select", "left drag", "native selection", and "mouse reporting"

Unlike `ScrollReportingWidget`, the new widget should not be disabled when `mouse_reporting_enabled` is false. If mouse reporting is disabled, native selection is already the effective behavior; keeping the toggle independently editable lets the user's preference persist for when mouse reporting is re-enabled.

### 6. Add command-palette and keybinding context integration

Update `init_actions_from_parent_view` in `features_page.rs` with a `ToggleSettingActionPair`:

- command text: "native left drag select" or "native left-drag selection"
- action: `SettingsAction::FeaturesPageToggle(FeaturesPageAction::ToggleNativeLeftDragSelect)`
- context: same parent settings context as other feature toggles
- context flag: a new `NATIVE_LEFT_DRAG_SELECT_CONTEXT_FLAG`
- platform support: `AltScreenReporting::as_ref(app).native_left_drag_select_enabled.is_supported_on_current_platform()`

Add `NATIVE_LEFT_DRAG_SELECT_CONTEXT_FLAG` in `app/src/settings_view/mod.rs` alongside `SCROLL_REPORTING_CONTEXT_FLAG` and `FOCUS_REPORTING_CONTEXT_FLAG`, with a stable string such as `Native_Left_Drag_Select`.

Update `Workspace::add_toggle_setting_context_flags` in `app/src/workspace/view.rs` to insert that flag when `native_left_drag_select_enabled` is true.

No app menu entry is required by the product spec. If maintainers want parity with the View menu's mouse, scroll, and focus reporting entries, add it as a follow-up or explicitly expand the product spec first.

### 7. Keep global action changes scoped

The repository has global actions for mouse, scroll, and focus reporting in `app/src/workspace/global_actions.rs`, plus View menu items in `app/src/app_menus.rs`. The product spec does not require a View menu item, and the command-palette path can be satisfied through `ToggleSettingActionPair`, so these files do not need to change for the initial implementation.

If implementation chooses to add a global action for symmetry, it should use the same toggle-and-save pattern as the existing AltScreenReporting actions and should not replace the command-palette integration.

## End-to-end flow

1. User enables "Native Left-Drag Selection" in Settings → Features → Terminal or via the command palette.
2. Warp persists `terminal.native_left_drag_select_enabled = true` through the settings system.
3. The workspace context gains `Native_Left_Drag_Select`, enabling keybinding predicates to reflect the current setting state.
4. A TUI enables SGR mouse reporting and a mouse tracking mode.
5. User performs a bare left-button down inside the alt-screen grid.
6. `AltScreenElement::left_mouse_down` computes the grid point and selection type, then calls `should_intercept_mouse(..., bare_left_button = true, ...)`.
7. `should_intercept_mouse` sees the setting is enabled and returns `true`.
8. The alt-screen element dispatches `TerminalAction::AltSelect(SelectAction::Begin { ... })` instead of `TerminalAction::AltMouseAction`.
9. User drags and releases the left button; drag updates and release ends Warp's native selection.
10. User presses `Cmd+C`; Warp copies the native selection through existing copy handling.

For right-click, scroll, middle-click, mouse motion, or non-Shift modifier mouse gestures, callers pass `bare_left_button = false`, so the existing mouse reporting decision path stays in control.

## Risks and mitigations

- Risk: accidentally intercepting all left-button clicks, including modifier-click gestures that users expect the TUI to receive. Mitigation: model the new helper parameter as `bare_left_button`, or pass full modifier context and gate the override on unmodified left-button events only.
- Risk: breaking right-click context menus or TUI right-click reporting. Mitigation: right-click call sites pass `bare_left_button = false`; add manual validation for both native and reported right-click behavior.
- Risk: breaking scroll behavior by routing scroll through the new setting. Mitigation: `should_intercept_scroll` always passes `bare_left_button = false` and keeps checking `scroll_reporting_enabled`.
- Risk: inconsistent behavior between alt-screen panes and active long-running blocks. Mitigation: update both `AltScreenElement` and `BlockListElement` forwarding checks and include both in manual validation.
- Risk: settings UI toggle exists but command palette or context flags are missing. Mitigation: follow the existing AltScreenReporting toggle pattern in `features_page.rs`, `settings_view::flags`, and `Workspace::add_toggle_setting_context_flags`.
- Risk: default-on behavior would regress TUI workflows that rely on drag forwarding. Mitigation: default the setting to `false` and add tests covering disabled behavior.

## Testing and validation

Automated validation:

- Add or update unit coverage for `should_intercept_mouse`:
  - SGR mouse + tracking + mouse reporting enabled + setting disabled + bare left button returns `false`.
  - Same state + setting enabled + bare left button returns `true`.
  - Same state + setting enabled + non-left or non-bare input returns `false`.
  - `shift = true` still returns `true`.
  - shared-session reader behavior still returns `true`.
- Update existing alt-screen selection test expectations in `app/src/terminal/view_tests.rs` for the new helper signature.
- If there is an existing settings test covering Features actions or context flags, extend it to cover `ToggleNativeLeftDragSelect` and `Native_Left_Drag_Select`.
- Run `cargo fmt`.
- Run `cargo check -p warp --bin warp-oss --features gui`.
- Run the narrow terminal/settings tests touched by the implementation, or the repository's preferred PR verification command if available.

Manual validation:

- On macOS, open a TUI with mouse reporting enabled, such as Claude Code, vim, tmux, or htop.
- With `terminal.native_left_drag_select_enabled = false`, verify bare left-drag is still forwarded to the TUI and Warp does not create a native selection.
- Enable the setting from Settings → Features → Terminal.
- Verify bare left-drag creates a Warp native selection and `Cmd+C` copies selected text.
- Verify `Shift`-drag still creates a native selection.
- Verify right-click, scroll-wheel or trackpad scroll, middle-click, and non-Shift modifier mouse gestures behave as they did before.
- Verify the command palette finds and toggles the setting.
- Verify the setting persists through restart or settings reload.

## Follow-ups

- Consider a transient modifier override, such as an `Option`-key bypass, for users who want the inverse per-gesture behavior while keeping their persistent default.
- Consider adding a View menu item for the setting if maintainers want complete menu parity with mouse, scroll, and focus reporting.
- Consider docs updates for the full-screen apps documentation page that currently explains mouse and scroll reporting and the `Shift` selection bypass.
