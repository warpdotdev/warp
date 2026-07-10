# Tech Spec: Windows Jump List for Tab Configs

**Issue:** [warpdotdev/warp#6156](https://github.com/warpdotdev/warp/issues/6156)

## Context

Windows jump lists are populated by the app via the Shell COM interface `ICustomDestinationList`. Warp currently sets no custom destinations or tasks, so its taskbar jump list shows only Windows defaults. The user's saved Tab Configs are already openable via `<scheme>://tab_config/<file-stem>`, and "New Window" is already openable via `<scheme>://action/new_window?path=~`, where `<scheme>` is the running channel's URI scheme from `ChannelState::url_scheme()`. We will register one jump list destination per Tab Config and one static "New Window" task.

The running process already declares its AppUserModelID (AUMID) via `warpui::platform::windows::AppBuilderExt::set_app_user_model_id` in [`app/src/lib.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/lib.rs), using `ChannelState::app_id().to_string()` to match the installer's shortcut (`dev.warp.{MyAppName}` in [`script/windows/windows-installer.iss`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/script/windows/windows-installer.iss)). This satisfies the jump list prerequisite; this feature only needs to ensure the jump list is refreshed after app initialization, when the AUMID is already set.

Relevant files:
- [`crates/warp_features/src/lib.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/crates/warp_features/src/lib.rs) — `FeatureFlag` enum and flag arrays. Add `WindowsJumpList`.
- [`app/src/user_config/mod.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/user_config/mod.rs) — `WarpConfig` `SingletonEntity` and `WarpConfigUpdateEvent` enum. The new `JumpListManager` subscribes to `WarpConfigUpdateEvent::TabConfigs` here.
- [`app/src/tab_configs/tab_config.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/tab_configs/tab_config.rs) — `TabConfig` carries `name` and `source_path`; `source_path.file_stem()` is the deeplink key.
- [`app/src/uri/mod.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/uri/mod.rs) — `handle_tab_config_uri` and `Action::NewWindow` already resolve the deeplinks jump list entries will use.
- [`app/src/platform/windows.rs`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/app/src/platform/windows.rs) — existing small Windows-only module; home for AUMID + jump list code.
- [`crates/warp_core/src/channel/state.rs:130`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/crates/warp_core/src/channel/state.rs#L130) — `ChannelState::app_id()` returns the canonical per-channel `AppId` (e.g. `dev.warp.Warp`), matching the installer's AUMID scheme.
- [`Cargo.toml`](https://github.com/warpdotdev/warp/blob/05927696c07c3ddcfb89ac24113fc202e41dc71/Cargo.toml) — the workspace `windows` dependency already enables `Win32_System_Com`; add `Win32_UI_Shell` and `Win32_UI_Shell_PropertiesSystem`.

## Proposed changes

1. **Feature flag.** Add `WindowsJumpList` to `FeatureFlag` in `crates/warp_features/src/lib.rs` and to `DOGFOOD_FLAGS`. Gate all new code with `FeatureFlag::WindowsJumpList.is_enabled()`; use `#[cfg(windows)]` only for COM calls. When the flag is disabled, call `ICustomDestinationList::DeleteList` (or commit an empty custom list) on startup to remove any previously committed destinations/tasks so the jump list does not show stale entries.
2. **Reuse the existing AUMID assignment.** No new AUMID setter is needed. `app/src/lib.rs` already calls `app_builder.set_app_user_model_id(ChannelState::app_id().to_string())` during app builder initialization, before the first jump list refresh.
3. **Add a jump list refresh module.** Create `app/src/platform/windows/jump_list.rs` (included from `windows.rs`) with:
   - `pub fn refresh_jump_list(configs: &[TabConfigEntry])` that calls `ICustomDestinationList::BeginList`, reads the available destination slot count from `pcMinSlots`, and obtains the removed-destinations `IObjectCollection`. Filter out any Tab Config entries whose identifiers appear in the removed collection so user-removed destinations are not re-added. Add a "New Window" task, add a "Tab Configs" category with the remaining entries up to the slot count, and commit. On COM failure, log and return; jump list population must never break startup. The function is safe to call from a background thread.
   - A helper that builds `IShellLinkW` shortcuts targeting the current executable with the deeplink URI as arguments. Percent-encode the file stem as a URI path segment and pass the full deeplink as a single quoted Shell Link argument so valid filenames cannot split argv or change URL semantics. Set the visible label via `IPropertyStore::SetValue(PKEY_Title, ...)` and the tooltip via `IShellLinkW::SetDescription`.
   - Initialize COM (`CoInitializeEx(COINIT_APARTMENTTHREADED)`) at entry and pair with `CoUninitialize`, because `IShellLinkW`/`ICustomDestinationList` are apartment-threaded and must be created and used on the same thread.
4. **Tab Config to jumplist item mapper.** Add `tab_configs_to_jump_entries(configs: &[TabConfig]) -> Vec<TabConfigEntry>` in the jump list module. Derive the file stem from `config.source_path`, use `config.name` as the label, and produce `<scheme>://tab_config/<stem>` deeplinks using `ChannelState::url_scheme()`. Sort alphabetically by display name. The jump-list refresh function then truncates the sorted entries to the slot count returned by `ICustomDestinationList::BeginList` (`pcMinSlots`, default 10) so the capped subset is deterministic.
5. **Decouple jump list refresh into a `SingletonEntity`.** Create `JumpListManager` as a `SingletonEntity` that subscribes to `WarpConfigUpdateEvent::TabConfigs`. This keeps `WarpConfig` free of any jump-list coupling — the manager independently reacts to config events and refreshes the jump list.

   The `JumpListManager` lives in `app/src/platform/windows/jump_list.rs` (or a new file under `app/src/`) and follows the same pattern as `AppearanceManager`:

   ```rust
   pub struct JumpListManager { /* no persistent state needed */ }

   impl JumpListManager {
       pub fn new(ctx: &mut ModelContext<Self>) -> Self {
           ctx.subscribe_to_model(&WarpConfig::handle(ctx), |_, _, event, ctx| {
               if let WarpConfigUpdateEvent::TabConfigs = event {
                   let configs = WarpConfig::handle(ctx).as_ref(ctx).tab_configs().clone();
                   let entries = tab_configs_to_jump_entries(&configs);
                   let _ = tokio::task::spawn_blocking(move || refresh_jump_list(&entries));
               }
           });
           Self {}
       }
   }

   impl Entity for JumpListManager { type Event = (); }
   impl SingletonEntity for JumpListManager {}
   ```

   On app startup, access `JumpListManager::handle(ctx)` once during initialization (in `app/src/lib.rs`) to trigger the lazy constructor and the initial jump list commit. Subsequent refreshes are event-driven via the subscription.

   `refresh_jump_list` initializes COM as `COINIT_APARTMENTTHREADED` at entry and uninitializes on exit, so it is safe to run on the Tokio blocking thread pool. No completion callback is needed — the jump list is best-effort.
6. **Cargo features.** Add `"Win32_UI_Shell"` and `"Win32_UI_Shell_PropertiesSystem"` to the workspace `windows` dependency.

## Testing and validation

- **Unit test (mapper):** In a `*_tests.rs` file, assert: label uses `name`, deeplink uses `ChannelState::url_scheme()` and the expected `<scheme>://tab_config/<stem>` form, entries sort by display name, empty input yields empty output, and two configs with the same `name` but different stems both appear with distinct deeplinks.
- **Manual tests on Windows with the flag enabled:**
  - Save a Tab Config, pin Warp to the taskbar, right-click, and confirm "New Window" and the "Tab Configs" category/entry appear; click each and verify behavior.
  - Quit Warp and click each entry; confirm cold-start opens the correct window.
  - Add/delete a `*.toml` while Warp runs; confirm the jump list updates without restart.
  - With the flag disabled after a previous enabled run, confirm any previously committed custom tasks/categories are cleared from the jump list.
  - For a parametrized Tab Config, confirm the params modal appears on jump list click (matching the `+` menu behavior).
  - Remove a Tab Config from the jump list via the Windows UI, then trigger a refresh; confirm the removed entry stays removed.
- **Async startup behavior:** verify the first window paints before the jump list commit finishes; a failed refresh must not block startup or surface an error UI.
- **Regression:** `./script/presubmit` passes and existing Tab Config / URI tests still pass.
