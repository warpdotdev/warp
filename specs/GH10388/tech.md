# GH-10388: Tech Spec - Hide Mouse Cursor While Typing on macOS

## Context

See `specs/GH10388/product.md` for the desired behavior. The implementation touches the existing settings, Settings UI, window creation, and macOS host-view paths:

- `app/src/settings/input.rs:35` defines `InputSettings` and the metadata used for TOML, platform support, visibility, and sync.
- `app/src/settings_view/features_page.rs:216`, `features_page.rs:565`, and `features_page.rs:2602` show the existing Terminal Input command-palette, action, and widget patterns.
- `crates/warpui_core/src/core/mod.rs:164`, `crates/warpui_core/src/core/app.rs:2299`, and `crates/warpui_core/src/platform/mod.rs:96` thread window options into platform windows.
- `app/src/root_view.rs:1178` creates regular window options; nearby restored, transferred, and Quake Mode paths build their own `AddWindowOptions`.
- `crates/warpui/src/platform/mac/objc/host_view.m:161` handles AppKit key-down events, and `crates/warpui/src/platform/mac/window.rs:482` stores per-window macOS state.

The prototype branch `gh-10388/hide-cursor-while-typing` already validated this approach.

## Proposed changes

1. Add `InputSettings::hide_cursor_while_typing`:
   - TOML path: `terminal.input.hide_cursor_while_typing`
   - Default: `true`
   - Supported platforms: `SupportedPlatforms::MAC`
   - Sync: `SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes)`
   - Public setting

2. Add a Settings -> Features -> Terminal Input toggle:
   - Label: `Hide mouse cursor while typing`
   - `FeaturesPageAction::ToggleHideCursorWhileTyping`
   - Matching command-palette toggle binding, context flag, widget, and telemetry action
   - Platform-gated through the setting metadata

3. Thread the value into platform windows:
   - Add `hide_cursor_while_typing` to `AddWindowOptions` and `WindowOptions`.
   - Initialize all app window creation paths from `InputSettings`.
   - Add a default no-op `set_hide_cursor_while_typing(bool)` method to the platform `Window` trait.

4. Implement the macOS behavior:
   - Store the value in macOS `WindowState`.
   - Add an Obj-C-callable Rust helper that returns the window state's current value.
   - In `host_view.m` `keyDownImpl`, call `[NSCursor setHiddenUntilMouseMoves:YES]` only when the setting is enabled.

5. Apply setting changes live:
   - Subscribe each `RootView` to `InputSettingsChangedEvent::HideCursorWhileTyping`.
   - On change, update that root view's platform window through `set_hide_cursor_while_typing`.

## Testing and validation

Add a focused settings metadata test for default value, TOML path, public/private status, macOS-only support, and per-platform sync mode.

Run:

- `cargo test -p warp settings::input::tests::hide_cursor_while_typing_metadata_matches_macos_toggle_contract`
- `cargo test -p warp settings::schema_validation_tests::file_defaults_validate_against_schema`

Manual macOS validation:

1. Default on: typing hides the cursor and mouse movement restores it.
2. Toggle off: typing leaves the cursor visible.
3. Toggle changes apply to existing windows without restart.
4. Newly opened regular and Quake Mode windows use the current setting.
5. Basic text input, IME/dead-key composition, selection, dragging, scrolling, and hover behavior still work.
