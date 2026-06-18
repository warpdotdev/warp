# Run Warp at startup (Windows) — Tech Spec
Linear: [CODE-1786](https://linear.app/warpdotdev/issue/CODE-1786/windows-run-warp-at-startup)
See `PRODUCT.md` for user-visible behavior.
## Context
The macOS "Start Warp at login" feature is already plumbed end-to-end. This ticket extends that plumbing to Windows by adding a new registration backend and broadening the existing setting/UI's platform gate; no new user-visible surface is being designed.
Relevant code today:
- `app/src/terminal/general_settings.rs:36-55` — `add_app_as_login_item` (`LoginItem`) and `app_added_as_login_item` (`AppAddedAsLoginItem`), both gated on `SupportedPlatforms::MAC` with defaults `true` / `false` respectively. The second setting is the "already registered" bookkeeping that prevents clobbering a manual unregister.
- `app/src/lib.rs:2229-2239` — startup wiring. Subscribes to `GeneralSettingsChangedEvent::LoginItem` and calls `maybe_register_app_as_login_item` on change + once at launch. Entire block is `#[cfg(target_os = "macos")]`.
- `app/src/lib.rs:2279-2362` — `maybe_register_app_as_login_item`. macOS-only. Guards on `WARP_INTEGRATION`, skips when bundle identifier is missing or equals `dev.warp.Warp-Local`, runs `SMAppService register/unregisterAndReturnError:` off the UI thread, then writes the result back to `app_added_as_login_item`.
- `app/src/settings_view/features_page.rs` — toggle UI:
  - action enum: `FeaturesPageAction::ToggleLoginItem` (~l618)
  - telemetry: `ToggleLoginItem` branch (~l975-978)
  - handler: `ToggleLoginItem => ...` (~l1855-1857)
  - widget: `LoginItemWidget` (~l4488-4533), rendered only when `add_app_as_login_item.is_supported_on_current_platform()` returns true (~l2481-2486). Label is hard-coded to `"Start Warp at login (requires macOS 13+)"`.
- `crates/settings/src/lib.rs:161-219` — `SupportedPlatforms` enum and `matches_current_platform`. Supports `OR(…, …)` so `MAC` + `WINDOWS` can be expressed without adding a new variant.
- `crates/warp_core/src/channel/state.rs:119-125, 40` — `ChannelState::app_id()` returns the current channel's `AppId` (e.g. `dev.openwarp.OpenWarp`, `dev.warp.Warp`, `dev.warp.WarpPreview`, `dev.warp.WarpDev`). We can reuse `application_name()` for the channel-specific registry value name on Windows.
- `app/Cargo.toml:356-377` — Windows-only dependency block. `winreg`, `windows-registry`, and `windows` are already pulled in and available for a new module. `winreg` is already used for reading registry values (`crates/warpui/src/windowing/winit/windows/registry.rs`), so we should follow that pattern for consistency.
## Proposed changes
### 1. Loosen the setting's platform gate
Change both settings in `app/src/terminal/general_settings.rs` from `SupportedPlatforms::MAC` to `SupportedPlatforms::OR(Box::new(SupportedPlatforms::MAC), Box::new(SupportedPlatforms::WINDOWS))`. Defaults stay the same (`true` / `false`). The `toml_path` / description stay unchanged since the TOML key is already platform-neutral (`general.login_item`).
No migration is required: for existing Windows users, the setting simply starts populating with its default value the next time preferences are loaded.
### 2. Add a Windows registration backend
Create `app/src/login_item/mod.rs` (or reuse an existing "platform adapters" spot — see "Module placement" below) to own the cross-platform entry point, replacing the current macOS-only free function.
```rust path=null start=null
// app/src/login_item/mod.rs
pub fn maybe_register_app_as_login_item(ctx: &mut AppContext) {
    if std::env::var("WARP_INTEGRATION").is_ok() {
        log::debug!("Not registering as a login item in integration tests");
        return;
    }
    #[cfg(target_os = "macos")]
    macos::maybe_register(ctx);
    #[cfg(target_os = "windows")]
    windows::maybe_register(ctx);
}
```
Move the existing macOS body into `login_item/macos.rs` unchanged.
Add `login_item/windows.rs` with the Windows implementation:
- Pull the desired state + the already-registered flag off `GeneralSettings` the same way the macOS path does.
- Short-circuit if `add_app_as_login_item && app_added_as_login_item` (the existing "don't clobber a manual unregister" contract from `lib.rs:2290-2296`).
- Resolve the executable path via `std::env::current_exe()?` then `dunce::canonicalize`. `std::fs::canonicalize` on Windows always returns a Win32 verbatim (`\\?\`) path, which is ugly in Settings → Apps → Startup / Task Manager and trips up some third-party tools that parse the `Run` value; `dunce` strips the prefix when safe and leaves it alone for real UNC / long paths. Bail with a debug log if resolution fails.
- Skip registration for dev/local builds. The cleanest check is `ChannelState::is_release_bundle()` (see `crates/warp_core/src/channel/state.rs:77-79`); use it as the Windows equivalent of the macOS "bundle identifier missing / `dev.warp.Warp-Local`" guard. Explicitly keep the behavior that enabling the toggle in a dev build persists the preference but does not write the registry value, since the preference is synced via user settings and would bleed into release runs otherwise — we only skip the I/O, not the preference update. (Matches macOS: toggling in a non-bundled build is a no-op on the system side.)
- Off-thread the registry I/O via `ctx.spawn`, mirroring the macOS path and the existing async signature `|settings, app_added_as_login_item, ctx| { ... }`. Return `bool` for "registered successfully" so the existing completion handler can set `app_added_as_login_item`.
- Registry work itself:
  ```rust path=null start=null
  use winreg::enums::HKEY_CURRENT_USER;
  use winreg::RegKey;
  const RUN_SUBKEY: &str =
      r"Software\Microsoft\Windows\CurrentVersion\Run";
  fn value_name() -> String {
      // e.g. "Warp", "WarpPreview", "WarpDev", "OpenWarp"
      ChannelState::app_id().application_name().to_owned()
  }
  fn register(exe: &Path) -> std::io::Result<()> {
      let hkcu = RegKey::predef(HKEY_CURRENT_USER);
      let (run_key, _) = hkcu.create_subkey(RUN_SUBKEY)?;
      // Quote the path so spaces (e.g. "C:\Program Files\Warp\warp.exe") survive parsing.
      let value = format!("\"{}\"", exe.display());
      run_key.set_value(&value_name(), &value)
  }
  fn unregister() -> std::io::Result<()> {
      let hkcu = RegKey::predef(HKEY_CURRENT_USER);
      let run_key = hkcu.open_subkey_with_flags(
          RUN_SUBKEY,
          winreg::enums::KEY_SET_VALUE,
      )?;
      match run_key.delete_value(value_name()) {
          Ok(()) => Ok(()),
          Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
          Err(e) => Err(e),
      }
  }
  ```
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` is the user-scope per-login key; it does not require admin, and is what Windows 10/11's **Settings → Apps → Startup** and **Task Manager → Startup apps** surface. This satisfies Behavior invariants 3–4, 7, 11.
- The per-channel value name from `application_name()` keeps Dev/Preview/Stable isolated (Behavior 7): `Warp`, `WarpPreview`, `WarpDev`, `OpenWarp`.
- Behavior 10 ("moving the install") is partially covered: the short-circuit `add_app_as_login_item && app_added_as_login_item` intentionally prevents re-registration on every launch, so a moved install keeps the stale path until the user toggles the setting off and back on (which rewrites against the new `current_exe()`). Automatic detection + rewrite when the stored path differs is tracked as a follow-up.
### 3. Rewire startup
Replace the existing `#[cfg(target_os = "macos")]` block in `app/src/lib.rs:2229-2239` with a block gated on `cfg(any(target_os = "macos", target_os = "windows"))`, calling the new cross-platform `login_item::maybe_register_app_as_login_item`. The subscription to `GeneralSettingsChangedEvent::LoginItem` stays identical. Delete the old `maybe_register_app_as_login_item` body from `lib.rs:2279-2362`.
### 4. Update the Features page UI
Two tiny changes in `app/src/settings_view/features_page.rs`:
- `LoginItemWidget::render` (~l4509): change the hard-coded label to a platform-dependent string, e.g.
  ```rust path=null start=null
  #[cfg(target_os = "macos")]
  let label = "Start Warp at login (requires macOS 13+)";
  #[cfg(target_os = "windows")]
  let label = "Start Warp at login";
  ```
  Anything else — action enum, telemetry, handler, and widget-registration path — already flows through `is_supported_on_current_platform()` on the setting itself, so broadening the setting in step 1 automatically turns the toggle on for Windows.
- Update `search_terms` for the widget (~l4497) to keep the macOS keyword but add "windows", so the settings search surface finds it on both OSes.
### Module placement
We don't have an existing `login_item` module. A new `app/src/login_item/{mod,macos,windows}.rs` layout is the smallest, most obvious split and matches other OS-sharded areas (`app/src/terminal/local_tty/windows/*`, `app/src/util/file/external_editor/{mod,windows}.rs`, `app/src/antivirus/windows.rs`, `app/src/terminal/audible_bell/{mod,windows}.rs`). Add `mod login_item;` in `app/src/lib.rs` and re-export `maybe_register_app_as_login_item` there.
## Testing and validation
Verification maps back to the numbered invariants in `PRODUCT.md`.
Unit tests (Windows, gated with `#[cfg(target_os = "windows")]`):
- **Invariant 3 (enable registers).** Drive `register()` against a temporary `HKCU` subkey (or wrap the subkey path in a trait and use a fake in tests) and assert the value is `"\"<path>\""` under the expected name.
- **Invariant 4 (disable unregisters, idempotent).** Call `unregister()` twice in a row; both must succeed. Calling `unregister()` when the value was never set must succeed.
- **Invariant 7 (per-channel isolation).** With different `ChannelState` app IDs, `value_name()` returns distinct strings (`Warp`, `WarpPreview`, `WarpDev`). Registering under one does not touch another.
- **Invariant 10 (path move).** Register with path A, then with path B; the stored value is B. No leftover value under a different name.
Because the real `Software\Microsoft\Windows\CurrentVersion\Run` hive is shared user state that should not be mutated by tests, the registry helpers should accept an injectable subkey path (e.g. `register_with_subkey(subkey, …)`) and the tests should drive them against `Software\Warp\TestRun\<uuid>` under HKCU, cleaning up on drop. Follow the `crates/warpui/src/windowing/winit/windows/registry.rs` style for the read-side helper for consistency.
Cross-platform tests (run on every OS):
- **Invariant 1/13 (platform gate).** Assert `LoginItem::supported_platforms().matches_current_platform()` is true on mac+windows, false on wasm. This is already covered structurally by `SupportedPlatforms::OR`; a small regression test in `general_settings`/`features_page` confirms the intent stays stable.
- **Invariant 9 (integration test guard).** With `WARP_INTEGRATION` set, calling `maybe_register_app_as_login_item` does no registry I/O. Can be asserted by calling with a mock subkey provider that panics if invoked.
Manual validation (Windows):
1. **Invariants 1, 2, 11.** Install a release-bundle Windows build, verify **Features → General** shows *Start Warp at login*, toggle default is on, and `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Warp` exists with the installed exe path. Confirm the entry is visible in **Settings → Apps → Startup** and **Task Manager → Startup apps** with the display name "Warp".
2. **Invariant 3–4.** Toggle off; confirm the registry value is gone and Windows UIs no longer show the entry. Toggle back on; confirm value is rewritten.
3. **Invariant 5.** With toggle on, delete the registry value (or disable from Task Manager). Relaunch Warp; verify the value is **not** recreated and `app_added_as_login_item` stays true. Toggle off then on in-app; verify the value *is* recreated.
4. **Invariant 6.** Sign out and back into Windows. Warp launches without stealing focus; the global hotkey surfaces it as expected.
5. **Invariant 7.** Install both Stable and Preview channels; enable the setting in each; confirm both registry values (`Warp`, `WarpPreview`) coexist and unregistering one doesn't affect the other.
6. **Invariant 8.** Run `cargo run` from a dev checkout with the toggle on; confirm no registry value is written and the preference updates still persist.
7. **Invariant 10.** Move/rename the install directory, relaunch Warp, toggle off+on; confirm the registry value points at the new path.
Telemetry (**Invariant 12**): filter the dashboard for `FeaturesPageAction { action: "ToggleLoginItem" }` and confirm Windows events arrive alongside macOS ones post-rollout.
## Risks and mitigations
- **Silently re-enabling a user's manual removal.** Mitigated by reusing the existing `app_added_as_login_item` bookkeeping contract — Windows code *must* respect it the same way macOS does. Covered by Invariant 5 and the relevant manual test.
- **Path quoting bugs.** Windows start entries are fragile with paths containing spaces. Store the value as a single quoted string, and add a regression test for a path with spaces.
- **Verbatim path prefix.** `std::fs::canonicalize` returns `\\?\`-prefixed paths on Windows, which render oddly in Settings → Apps → Startup and confuse some third-party launchers. Use `dunce::canonicalize` so the stored `Run` value is a plain drive-letter path for the common case, while still tolerating real UNC / long paths.
- **Shared registry key across tests/processes.** Use injectable subkey paths in unit tests so the real `Run` key is never touched by CI.
- **Dev-build regressions.** Gate actual registry I/O on `ChannelState::is_release_bundle()` to avoid developer machines auto-launching random `target/debug/warp.exe` artifacts.
## Follow-ups
- Linux autostart (`~/.config/autostart/warp.desktop`) is a natural next step with the same setting and the same UX, but is out of scope for this ticket.
- Optional future polish: automatically rewrite the stored path when `current_exe()` differs from the registered value, so users never need to re-toggle after a move.
