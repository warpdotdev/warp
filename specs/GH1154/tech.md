# Hide Warp Dock Icon with Menu Bar Fallback — Tech Spec
Product spec: `specs/GH1154/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/1154

## Problem
Warp's macOS AppKit layer always presents the app as a regular application, which gives it a Dock icon and Cmd-Tab presence. The current app icon setting only changes Dock tile artwork; it does not change app presentation. Implementing this feature requires a macOS presentation setting that can switch Warp between regular and accessory activation policies, plus a menu bar status item that remains available whenever the regular Dock entry point is hidden.

The implementation crosses app settings, the settings UI, root window actions, macOS AppKit bindings, and startup initialization. It should stay macOS-only and preserve current behavior by default.

## Relevant code
- `app/src/settings/app_icon.rs:1` — existing macOS-only app icon setting and storage key.
- `app/src/settings/init.rs:87` — registration of `AppIconSettings`.
- `app/src/settings_view/appearance_page.rs (2731-2859)` — existing app icon settings widget.
- `app/src/appearance.rs (46-62)` — `AppearanceManager` subscription to `AppIconSettings` changes.
- `app/src/appearance.rs (163-264)` — runtime Dock tile icon update path.
- `app/src/lib.rs (925-950)` — macOS `AppBuilder` setup for activation-on-launch, dev icon, menu bar, and Dock menu.
- `app/src/app_menus.rs (77-104)` — app menu bar and Dock menu builders.
- `app/src/root_view.rs (396-471)` — global actions for opening new windows and global hotkey actions.
- `app/src/root_view.rs (1218-1436)` — dedicated hotkey window state and show/hide flow.
- `app/src/lib.rs (2153-2160)` — current Dock reopen callback opens a normal new window.
- `crates/warpui/src/platform/mac/app.rs (89-126)` — macOS `AppExt` builder customization points.
- `crates/warpui/src/platform/mac/app.rs (216-337)` — AppKit launch setup and menu construction.
- `crates/warpui/src/platform/mac/objc/app.m (316-322)` — Dock/Finder reopen behavior.
- `crates/warpui/src/platform/mac/objc/app.m (486-495)` — `WarpApplication` creation currently sets `NSApplicationActivationPolicyRegular`.
- `crates/warpui/src/platform/mac/objc/app.h:8` — `WarpDelegate` owns AppKit delegate state and the existing Dock menu.
- `crates/warpui/src/platform/mac/menus.rs (236-356)` — Rust-to-AppKit menu item construction for main and Dock menus.
- `crates/warpui_core/src/platform/menu.rs:1` — shared `Menu` / `MenuItem` abstraction used by macOS menus.
- `app/src/terminal/keys_settings.rs (16-65)` — global hotkey settings are separate from app presentation settings and should remain separate.

## Current state
Warp's macOS startup path builds a platform app, enables activation on launch, sets a dev icon for unbundled runs, and installs the main menu and Dock menu. In Objective-C, `get_warp_app()` creates the shared `WarpApplication`, installs `WarpDelegate`, and forces `app.activationPolicy = NSApplicationActivationPolicyRegular`. This regular activation policy is what makes Warp appear in the Dock and Cmd-Tab.

The existing app icon preference is `AppIconSettings.app_icon`. It is macOS-only, not synced to cloud, and changes Dock tile art through `AppearanceManager::set_app_icon`. It is not a good model for a "Hidden" enum value because hiding the Dock icon is app presentation behavior with separate recovery requirements, not icon artwork.

Warp has two global hotkey modes today: dedicated hotkey window and show/hide all windows. These live in `KeysSettings` and are rendered in the Features page. The requested Dock visibility setting must not depend on either mode, though the menu bar Show Warp action should understand dedicated hotkey state so it can show the expected window.

There is no existing NSStatusItem or menu bar status item abstraction in `warpui`. The macOS menu abstraction already builds `NSMenu` instances from Rust `Menu` values, so the status item can reuse that menu construction path instead of inventing a second menu-item callback system.

## Proposed changes
### 1. Add a macOS Dock visibility setting
Add a boolean setting to `AppIconSettings` in `app/src/settings/app_icon.rs`:

- Name: `show_dock_icon`
- Default: `true`
- Supported platforms: `SupportedPlatforms::MAC`
- Sync: `SyncToCloud::Never`
- Storage key: `ShowDockIcon`
- TOML path: `appearance.icon.show_dock_icon`
- Description: whether Warp is shown in the macOS Dock and Cmd-Tab switcher.

Keep this as a separate field from `app_icon`. Do not add a hidden variant to `AppIcon`, because `AppIcon` still describes artwork when the Dock icon is visible.

The generated changed event should be handled alongside `AppIconState` in `AppearanceManager`. On `ShowDockIcon` changes, call a macOS platform bridge to apply the activation policy and status item visibility immediately.

### 2. Read the setting during macOS startup
In `app/src/lib.rs`, after public preferences are available and before `app_builder.run`, read the saved `ShowDockIcon` value from `prefs_for_public_settings` using the generated setting helper, following the same pre-app-read pattern used by `ForceX11`.

Extend `warpui::platform::mac::AppExt` with a builder method such as `set_show_dock_icon_on_launch(bool)` and call it from the macOS block next to `set_activate_on_launch`, `set_dev_icon`, `set_menu_bar_builder`, and `set_dock_menu_builder`.

This lets the AppKit layer apply accessory mode as early as practical during launch, reducing visible Dock flicker for users who have already hidden the Dock icon.

### 3. Add macOS platform support for activation policy and status item
Extend `crates/warpui/src/platform/mac/app.rs` and the Objective-C bridge in `crates/warpui/src/platform/mac/objc/app.m` / `app.h`.

Recommended shape:
- Add `show_dock_icon_on_launch: bool` to the macOS `App` backend, defaulting to `true`.
- Add an optional status item menu builder to the backend, similar to `dock_menu_builder`.
- During `warp_app_will_finish_launching`, build the status item menu and hand it to the Objective-C delegate.
- Add a Rust-callable platform function such as `set_dock_icon_visible(show: bool, status_menu: id)` or separate functions to update activation policy and status item visibility.
- In Objective-C, make `WarpDelegate` own a retained `NSStatusItem *statusItem` in addition to `dockMenu`.
- When `show_dock_icon` is true:
  - set `NSApp.activationPolicy = NSApplicationActivationPolicyRegular`
  - remove any status item from `[NSStatusBar systemStatusBar]`
- When `show_dock_icon` is false:
  - set `NSApp.activationPolicy = NSApplicationActivationPolicyAccessory`
  - create or reuse an `NSStatusItem`
  - set a recognizable template image or short title on `statusItem.button`
  - attach the status menu to `statusItem.menu`

Use `NSApplicationActivationPolicyAccessory` rather than editing `Info.plist` / `LSUIElement`. Runtime activation policy changes are reversible and do not mutate the app bundle, which avoids the unsupported-workaround problem described in the issue.

Keep the existing Dock menu path unchanged. The Dock menu is only useful when the app is regular; the status item menu is the replacement entry point when the app is accessory.

### 4. Reuse the menu abstraction for the status item menu
Add `app_menus::status_item_menu(ctx: &mut AppContext) -> Menu` in `app/src/app_menus.rs`. Reuse `MenuItem::Custom` callbacks so menu actions run through the same Rust `AppContext` dispatch path as existing main and Dock menu items.

The initial menu should include:
- Show Warp: dispatch a new non-toggling root action such as `root_view:show_primary_window`.
- New Window: dispatch `root_view:open_new` and `workspace:save_app`.
- Settings: dispatch the existing settings action used by the app menu, or a small wrapper that opens settings in a new or existing window.
- Quit Warp: call the same terminate path used by standard Quit so confirmation behavior remains unchanged.

Update `AppExt` with `set_status_item_menu_builder` and call it from `app/src/lib.rs` next to `set_dock_menu_builder`.

### 5. Add a non-toggling "Show Warp" action
Add a new global action in `app/src/root_view.rs`, for example `root_view:show_primary_window`, with product semantics tailored to the status item:

1. If dedicated hotkey mode is enabled:
   - If a quake window exists and is hidden, show it via `ctx.windows().show_window_and_focus_app`.
   - If a quake window exists and is already open or pending open, focus it without hiding it.
   - If no quake window exists, create it using the same creation path as `toggle_quake_mode_window`.
2. If dedicated hotkey mode is not enabled:
   - If a normal Warp window exists, activate the app and bring a normal window forward.
   - If no normal window exists, call `open_new`.

Do not reuse `toggle_quake_mode_window` directly for the status item Show Warp command, because toggling would hide the hotkey window when the user is trying to recover or foreground Warp.

If the creation path shares substantial logic with `toggle_quake_mode_window`, extract helper functions rather than duplicating the full hotkey-window construction.

### 6. Add settings UI
Update the Appearance settings page near `CustomAppIconWidget`:
- Add a macOS-only switch labelled "Show Warp in Dock" or "Show Dock icon".
- Default checked state reflects `AppIconSettings::show_dock_icon`.
- Toggle dispatch updates `AppIconSettings.show_dock_icon`.
- Include search terms such as "dock", "cmd tab", "app switcher", "menu bar", and "status bar".
- Keep the existing app icon dropdown behavior unchanged and visible only when relevant.

This placement keeps app presentation near app icon customization while avoiding the misconception that the preference only applies to global hotkeys.

### 7. Runtime updates and safety
On runtime setting changes:
- Apply activation policy first.
- If hiding the Dock icon succeeds, create/show the status item.
- If creating the status item fails, leave or restore `show_dock_icon` behavior to the safe regular-app state and log the failure.
- If restoring the Dock icon, remove the status item after setting regular activation policy.

Avoid a state where both the Dock icon and the menu bar item are absent.

For local unbundled developer runs, degrade safely. The status item can use a text fallback or the existing embedded local icon if bundled image lookup is unavailable.

## End-to-end flow
### User hides the Dock icon
1. User opens Appearance settings and turns off Show Warp in Dock.
2. `AppIconSettings.show_dock_icon` is saved.
3. `AppearanceManager` receives the generated settings change event.
4. `AppearanceManager` calls the macOS platform bridge with `show = false`.
5. AppKit switches Warp to `NSApplicationActivationPolicyAccessory`.
6. AppKit creates or shows the Warp status item and attaches the status item menu.
7. Warp disappears from the Dock and Cmd-Tab, while the status item remains visible.

### User clicks Show Warp from the menu bar
1. User clicks the Warp status item and chooses Show Warp.
2. The status item menu callback dispatches `root_view:show_primary_window`.
3. The root action checks `KeysSettings::global_hotkey_mode`.
4. If dedicated hotkey mode is enabled, Warp shows or focuses the dedicated hotkey window without toggling it closed.
5. Otherwise, Warp activates an existing normal window or opens a new normal window.

### User restores the Dock icon
1. User opens settings from the status item or from an existing window.
2. User turns on Show Warp in Dock.
3. The settings change event calls the macOS bridge with `show = true`.
4. AppKit switches Warp to `NSApplicationActivationPolicyRegular`.
5. AppKit removes the status item.
6. Warp returns to the Dock and Cmd-Tab.

## Risks and mitigations
### Risk: accessory activation changes menu and focus behavior
Accessory apps do not appear in the Dock or Cmd-Tab, and AppKit focus behavior can differ from regular apps.

Mitigation: keep the main menu and window activation paths unchanged, use existing `show_window_and_focus_app` / `activate_app` helpers, and manually validate focus for normal windows, dedicated hotkey windows, modals, and settings windows.

### Risk: user loses all visible entry points
If the Dock icon is hidden and the status item is not created, users without a hotkey may be unable to recover.

Mitigation: make the status item mandatory whenever `show_dock_icon` is false. If creating it fails, keep the app regular or restore regular activation policy and log the error.

### Risk: status item menu duplicates app menu logic
Duplicating menu callbacks could cause New Window, Settings, or Quit behavior to drift.

Mitigation: reuse `Menu` / `CustomMenuItem` and dispatch existing global actions where possible. Add small wrapper actions only when existing actions are toggles or need status-item-specific semantics.

### Risk: setting read order at startup
Settings are registered after `AppBuilder` is created, but the activation policy needs the saved value before AppKit launch.

Mitigation: read the generated setting directly from `prefs_for_public_settings` before `app_builder.run`, following the existing `ForceX11` startup pattern.

### Risk: app icon customization and status item icon share assets incorrectly
Dock icon art can be colorful and large; menu bar icons should generally be template images.

Mitigation: treat the status item icon as its own minimal template asset or text fallback. Do not reuse the selected Dock icon artwork unless design confirms it works in the menu bar.

## Testing and validation
### Unit and compile-time checks
- Add settings tests or schema validation ensuring `appearance.icon.show_dock_icon` exists, defaults to `true`, is macOS-only, and does not sync to cloud.
- Add Rust unit coverage for the new `show_primary_window` decision logic where practical by isolating mode/window-state selection from direct AppKit calls.
- Add compile coverage for `warpui` macOS code paths in the existing macOS CI job, because the status item bridge is `#[cfg(target_os = "macos")]`.

### Manual macOS validation
- Toggle Show Warp in Dock off and verify Warp disappears from the Dock and Cmd-Tab.
- Verify the Warp status item appears immediately and has the expected menu items.
- Choose Show Warp with:
  - dedicated hotkey enabled and hidden
  - dedicated hotkey enabled and already visible
  - no global hotkey and an existing normal window
  - no global hotkey and no normal windows
- Choose New Window from the status item and verify a normal window opens.
- Choose Settings from the status item and verify settings opens.
- Choose Quit Warp from the status item and verify existing quit confirmation behavior is preserved.
- Restart Warp with Show Warp in Dock disabled and verify the app starts in hidden-Dock mode with the status item present.
- Toggle Show Warp in Dock back on and verify the Dock icon and Cmd-Tab entry return and the status item is removed.
- Verify changing the selected app icon still updates Dock art when Show Warp in Dock is enabled.
- Verify local unbundled macOS development runs do not crash if status item image assets are unavailable.

## Follow-ups
- Add a checked "Show Warp in Dock" item directly to the status item menu if users need a faster way to restore the Dock icon.
- Consider a primary-click mode for the status item that performs Show Warp directly and reserves secondary click for the menu.
- Consider launch-at-login/background-start behavior as a separate feature for users who want Warp available only through hotkey or menu bar after boot.
