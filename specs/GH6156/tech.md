# Tech Spec: Windows Jump List for Tab Configs

**Issue:** [warpdotdev/warp#6156](https://github.com/warpdotdev/warp/issues/6156)

## Context

Windows jump lists are populated by the app via the Shell COM interface `ICustomDestinationList`. Warp currently sets no custom destinations or tasks, so its taskbar jump list shows only Windows defaults. The user's saved Tab Configs are already openable via `warp://tab_config/<file-stem>`, and "New Window" is already openable via `warp://action/new_window?path=~`. We will register one jump list destination per Tab Config and one static "New Window" task.

The running process already declares its AppUserModelID (AUMID) via `warpui::platform::windows::AppBuilderExt::set_app_user_model_id` in [`app/src/lib.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/lib.rs), using `ChannelState::app_id().to_string()` to match the installer's shortcut (`dev.warp.{MyAppName}` in [`script/windows/windows-installer.iss`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/script/windows/windows-installer.iss)). This satisfies the jump list prerequisite; this feature only needs to ensure the jump list is refreshed after app initialization, when the AUMID is already set.

Relevant files:
- [`crates/warp_features/src/lib.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/crates/warp_features/src/lib.rs) — `FeatureFlag` enum and flag arrays. Add `WindowsJumpList`.
- [`app/src/user_config/native.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/user_config/native.rs) — `load_tab_configs()` and the reload path that fires on startup and on `WarpConfigUpdateEvent::TabConfigs`. The natural trigger for refreshing the jump list.
- [`app/src/tab_configs/tab_config.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/tab_configs/tab_config.rs) — `TabConfig` carries `name` and `source_path`; `source_path.file_stem()` is the deeplink key.
- [`app/src/uri/mod.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/uri/mod.rs) — `handle_tab_config_uri` and `Action::NewWindow` already resolve the deeplinks jump list entries will use.
- [`app/src/platform/windows.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/platform/windows.rs) — existing small Windows-only module; home for AUMID + jump list code.
- [`crates/warp_core/src/channel/state.rs:130`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/crates/warp_core/src/channel/state.rs#L130) — `ChannelState::app_id()` returns the canonical per-channel `AppId` (e.g. `dev.warp.Warp`), matching the installer's AUMID scheme.
- [`Cargo.toml`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/Cargo.toml) — the workspace `windows` dependency already enables `Win32_System_Com`; add `Win32_UI_Shell` and `Win32_UI_Shell_PropertiesSystem`.

## Proposed changes

1. **Feature flag.** Add `WindowsJumpList` to `FeatureFlag` in `crates/warp_features/src/lib.rs` and to `DOGFOOD_FLAGS`. Gate all new code with `FeatureFlag::WindowsJumpList.is_enabled()`; use `#[cfg(windows)]` only for COM calls.
2. **Reuse the existing AUMID assignment.** No new AUMID setter is needed. `app/src/lib.rs` already calls `app_builder.set_app_user_model_id(ChannelState::app_id().to_string())` during app builder initialization, before the first jump list refresh.
3. **Add a jump list refresh module.** Create `app/src/platform/windows/jump_list.rs` (included from `windows.rs`) with:
   - `pub fn refresh_jump_list(configs: &[TabConfigEntry])` that calls `ICustomDestinationList::BeginList`, reads the available destination slot count from `pcMinSlots`, adds a "New Window" task, adds a "Tab Configs" category with up to that many sorted Tab Config entries, and commits. On COM failure, log and return; jump list population must never break startup. The function is safe to call from a background thread.
   - A helper that builds `IShellLinkW` shortcuts targeting the current executable with the deeplink URI as arguments. Percent-encode the file stem as a URI path segment and pass the full deeplink as a single quoted Shell Link argument so valid filenames cannot split argv or change URL semantics. Set the visible label via `IPropertyStore::SetValue(PKEY_Title, ...)` and the tooltip via `IShellLinkW::SetDescription`.
   - Initialize COM (`CoInitializeEx(COINIT_APARTMENTTHREADED)`) at entry and pair with `CoUninitialize`, because `IShellLinkW`/`ICustomDestinationList` are apartment-threaded and must be created and used on the same thread.
4. **Tab Config to jumplist item mapper.** Add `tab_configs_to_jump_entries(configs: &[TabConfig]) -> Vec<TabConfigEntry>` in the jump list module. Derive the file stem from `config.source_path`, use `config.name` as the label, and produce `warp://tab_config/<stem>` deeplinks. Sort alphabetically by display name. The jump-list refresh function then truncates the sorted entries to the slot count returned by `ICustomDestinationList::BeginList` (`pcMinSlots`, default 10) so the capped subset is deterministic.
5. **Trigger refresh asynchronously from the existing reload path.** In `app/src/user_config/native.rs`, inside the `ctx.spawn` callback for tab configs (both startup and `WarpConfigUpdateEvent::TabConfigs`), map the parsed configs to entries and fire a fire-and-forget background task:
   ```rust
   ctx.background_executor()
       .spawn(async move {
           tokio::task::spawn_blocking(move || refresh_jump_list(&entries)).await
       })
       .detach();
   ```
   `refresh_jump_list` initializes COM as `COINIT_APARTMENTTHREADED` at entry and uninitializes on exit, so it is safe to run on the Tokio blocking thread pool. No completion callback is needed — the jump list is best-effort.
6. **Cargo features.** Add `"Win32_UI_Shell"` and `"Win32_UI_Shell_PropertiesSystem"` to the workspace `windows` dependency.

## Testing and validation

- **Unit test (mapper):** In a `*_tests.rs` file, assert: label uses `name`, deeplink is `warp://tab_config/<stem>`, entries sort by display name, empty input yields empty output, and two configs with the same `name` but different stems both appear with distinct deeplinks.
- **Manual tests on Windows with the flag enabled:**
  - Save a Tab Config, pin Warp to the taskbar, right-click, and confirm "New Window" and the "Tab Configs" category/entry appear; click each and verify behavior.
  - Quit Warp and click each entry; confirm cold-start opens the correct window.
  - Add/delete a `*.toml` while Warp runs; confirm the jump list updates without restart.
  - With the flag disabled, confirm no custom task/category appears.
  - For a parametrized Tab Config, confirm the params modal appears on jump list click (matching the `+` menu behavior).
- **Async startup behavior:** verify the first window paints before the jump list commit finishes; a failed refresh must not block startup or surface an error UI.
- **Regression:** `./script/presubmit` passes and existing Tab Config / URI tests still pass.
