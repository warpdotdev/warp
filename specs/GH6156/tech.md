# Tech Spec: Windows Jump List for Tab Configs

**Issue:** [warpdotdev/warp#6156](https://github.com/warpdotdev/warp/issues/6156)

## Context

Windows builds a taskbar jump list from what an app commits through the Shell COM interface `ICustomDestinationList`: `SetAppID` → `BeginList` → `AddUserTasks` and/or `AppendCategory` → `CommitList`. Windows stores the list per AppUserModelID (AUMID) and shows it even while the app is closed. Warp commits nothing today.

Everything a task needs to launch already exists:
- **AUMID.** Warp sets an explicit process AUMID, `ChannelState::app_id()` ([`app/src/lib.rs:1295-1298`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/lib.rs#L1295-L1298)). Per the [`BeginList` docs](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icustomdestinationlist-beginlist), an app with an explicit AUMID must call `SetAppID` with it before `BeginList`.
- **Launch and forwarding.** URLs are positional arguments. In bundled builds, a second process forwards them to the running instance and exits. On a cold start, `launch()` restores the session, then handles argv URLs, then opens a window only if none exists ([`app/src/lib.rs:3155-3168`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/lib.rs#L3155-L3168)).
- **Deeplinks.** `<scheme>://action/new_window?path=` and `<scheme>://tab_config/<file-stem>` ([`handle_tab_config_uri`, `app/src/uri/mod.rs:813-863`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/uri/mod.rs#L813-L863)). The tab config name is form-decoded with `serde_urlencoded` ([`get_launch_config_path`, `:746-763`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/uri/mod.rs#L746-L763)), so `+`, `&` and `=` in a file stem must be percent-encoded.
- **Tab configs.** `WarpConfig::tab_configs()` holds the parsed configs, sorted by `load_tab_configs` in `+` menu order ([`app/src/user_config/native.rs:299-315`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/user_config/native.rs#L299-L315)). `WarpConfig` emits `WarpConfigUpdateEvent::TabConfigs` when its startup load finishes, even for an empty or missing directory ([`:45-56`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/user_config/native.rs#L45-L56)), and again after each debounced watcher reload ([`:130-143`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/user_config/native.rs#L130-L143)).
- **Labels.** The `+` menu disambiguates repeated names inline ([`app/src/workspace/view.rs:7067-7112`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/workspace/view.rs#L7067-L7112)).
- **COM on background threads.** `AntivirusInfo` runs COM work in `ctx.spawn`, behind a thread-local STA helper whose TODO asks for it to be shared ([`app/src/antivirus/windows.rs:71-102`](https://github.com/warpdotdev/warp/blob/d0d50955df8c4efcfb91091b150ef4c9a7eaa138/app/src/antivirus/windows.rs#L71-L102)).

## Proposed changes

1. **Feature flag.** Add `FeatureFlag::WindowsJumpList` to `DOGFOOD_FLAGS`, plus a `windows_jump_list` cargo feature mapped in `app/src/features.rs` for local builds.

2. **Tasks, not a category.** Commit all entries with `AddUserTasks`, as Windows Terminal does for its profiles. Tasks are always shown, unlike custom categories, which `AppendCategory` rejects with `E_ACCESSDENIED` when the recent-items privacy setting is off. Tasks aren't limited by the recent-items slot count, and users can't remove them. Windows also clears its removed-items record on every commit, so a category would need Warp to persist removals itself. Microsoft lists "launch an application in a certain mode" as a canonical task, which is what a Tab Config is. The trade-offs: the section header is Windows' localized "Tasks", and entries can't be pinned. The list only changes when the user edits Tab Configs, which stays within Microsoft's advice that task lists be mostly static.

3. **`JumpList` singleton** in `app/src/app_services/windows/jump_list.rs`. `app_services` holds "services that the application provides to the host system", including the Windows URI registration.
   - It subscribes to `WarpConfig`, matching `WarpConfigUpdateEvent` exhaustively and refreshing on `TabConfigs`. No startup refresh is needed, because the startup load always emits `TabConfigs`.
   - `refresh` builds the task list on the main thread and commits it in `ctx.spawn`. Two main-thread flags, `commit_in_flight` and `refresh_pending`, keep at most one commit running and re-read `WarpConfig` once it finishes. An older list can therefore never land after a newer one, without locks.
   - The commit runs `com_initialized()`, then `CoCreateInstance(DestinationList)`, `SetAppID(ChannelState::app_id())` and `BeginList` (its removed-items array is ignored, since tasks can't be removed). It adds one `IShellLinkW` per task, then calls `AddUserTasks` and `CommitList`, with `AbortList` on failure. Each link targets `current_exe()`, takes the quoted deeplink as its only argument, and uses the exe's icon. Its label is `PKEY_Title`, converted to `VT_LPWSTR` with `PropVariantChangeType`, because windows-rs turns `&str` into a `VT_BSTR`.
   - Failures go to `report_error!` with a fixed message. No user content is logged.
   - Registered in `initialize_app` next to the macOS GUI-only `set_app_icon` block, only for GUI launches with `WindowsJumpList` and `TabConfigs` enabled. That way `warp agent`/TUI processes never write the list.

4. **Pure mapper** `jump_list_tasks(tab_configs, scheme)`. It returns "New Window" (`<scheme>://action/new_window?path=~`), then each config with a `source_path`, in `WarpConfig` order, as `<scheme>://tab_config/<urlencoding::encode(stem)>`, labelled by `display_names`.

5. **Shared label helper.** Move the `+` menu's duplicate-name logic into `tab_configs::display_names(&[TabConfig]) -> Vec<String>`, and use it from both the menu and the mapper so their labels can't diverge.

6. **Shared COM helper.** Move `com_initialized` from `antivirus/windows.rs` into `warp_util::windows`, which already depends on `windows` with `Win32_System_Com`. Both callers use it.

7. **`windows` crate features** in `app/Cargo.toml`, with no new crates:
   - `Win32_UI_Shell`
   - `Win32_UI_Shell_Common`
   - `Win32_UI_Shell_PropertiesSystem`
   - `Win32_System_Com_StructuredStorage`
   - `Win32_System_Variant`
   - `Win32_Storage_EnhancedStorage`

## Testing and validation

- **`app/src/app_services/windows/jump_list_tests.rs`:**
  - "New Window" comes first, including with no configs (behavior 2, 7).
  - Configs keep `WarpConfig` order and `+` menu labels (3).
  - Stems are percent-encoded (4).
  - Configs without a file are skipped without shifting labels (3).
- **`app/src/tab_configs/tab_config_tests.rs`:** `display_names` suffixes repeated names in order (3).
- **`app/src/uri/uri_tests.rs`:** an encoded stem containing `+`, `&`, `=` and spaces round-trips through `get_launch_config_path` (4).
- **Manual, on Windows, with a bundled build** (forwarding needs `release_bundle`):
  1. Pin Warp and right-click it. Check "New Window" and each Tab Config, with their labels, under "Tasks" (1, 3, 13).
  2. With Warp running in the background, select a Tab Config and a parametrized one. The tab or params modal opens in the active window, and the window comes to the front (4, 5).
  3. Quit Warp and select a Tab Config, with session restore on, then off (5).
  4. While Warp runs, add, rename, break and delete Tab Config files, and check the list after each change (6, 7).
  5. Turn off "Show recently opened items…" and check the tasks still show (12).
  6. With Stable and Preview installed, check each list opens its own channel (10).
