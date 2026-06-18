# APP-3945: Channel-aware Warp home watching Technical Spec

## Problem
Warp's hot-reload behavior for user-managed files depends on several consumers observing filesystem changes from the current channel's Warp home. This work centralizes those updates behind `WarpDataDirectoryWatcher`, but it must also preserve a platform-specific requirement: public settings live under `config_local_dir()` while most other Warp home content lives under `data_dir()`.

The technical problem is to keep one Warp-specific watcher abstraction while:
- filtering `.warp*/worktrees` only for the data-directory tree
- watching `config_local_dir()` in addition to `data_dir()` when those roots differ
- preserving the existing downstream event contracts used by `WarpConfig`, Warp home MCP watching, and Warp home skill watching
- avoiding new generic filtering APIs in `repo_metadata::DirectoryWatcher`

## Relevant code
- `app/src/warp_data_directory_watcher.rs` — Warp-specific singleton watcher that owns a `BulkFilesystemWatcher`, registers Warp home roots, and emits `WarpDataDirectoryWatcherEvent::FilesChanged`.
- `app/src/lib.rs` — startup wiring that prepares the Warp watch roots before registering `WarpDataDirectoryWatcher`.
- `app/src/user_config/native.rs` — `WarpConfig` subscription that maps watcher updates to `Themes`, `Workflows`, `LaunchConfigs`, `TabConfigs`, and `Settings` events.
- `app/src/settings/init.rs` — settings-file hot-reload pipeline that reacts to `WarpConfigUpdateEvent::Settings`.
- `app/src/ai/mcp/file_mcp_watcher.rs` — Warp home MCP watcher subscriber that depends on `WarpDataDirectoryWatcher`.
- `app/src/ai/mcp/mod.rs` — helper that resolves the Warp MCP home config path through the shared Warp data directory helpers.
- `app/src/ai/skills/file_watchers/skill_watcher.rs` — Warp home skill watcher subscriber that depends on `WarpDataDirectoryWatcher`.
- `app/src/ai/skills/file_watchers/utils.rs` — skill path parsing helpers, including the Warp-home special case.
- `app/src/ai/skills/skill_utils.rs` — helper for resolving a skill root from a changed file path.
- `crates/ai/src/skills/skill_provider.rs` — provider/scope classification for channel-aware Warp home skill paths.
- `crates/integration/src/test/settings_file_hot_reload.rs` — end-to-end settings hot-reload coverage.

## Current state
`WarpDataDirectoryWatcher` is a Warp-specific singleton that owns its own `BulkFilesystemWatcher`, similar to `HomeDirectoryWatcher`. It does not depend on `DirectoryWatcher`, and it does not require any per-directory filter plumbing in `repo_metadata`.

At startup:
- setup code prepares `data_dir()` before `WarpDataDirectoryWatcher` is registered
- `WarpDataDirectoryWatcher` registers `data_dir()` recursively with a `WatchFilter` that excludes `<data_dir>/worktrees`
- setup code prepares `config_local_dir()` when it differs from `data_dir()`
- `WarpDataDirectoryWatcher` registers `config_local_dir()` recursively when it differs from `data_dir()`

The watcher receives `BulkFilesystemWatcherEvent` values, converts them into `RepositoryUpdate`, and emits `WarpDataDirectoryWatcherEvent::FilesChanged` directly. It does not apply a second worktree filter after registration-time filtering.

`WarpConfig` subscribes to that event stream and reloads themes, workflows, launch configs, and tab configs on background tasks when the update touches the relevant paths. Settings continue to flow through `WarpConfigUpdateEvent::Settings`.

`FileMCPWatcher` subscribes to the same singleton for Warp home MCP updates while continuing to use the existing home-directory and repository watching paths for non-Warp providers. The duplicated startup parse path and the duplicated single-config incremental update path are now shared with the non-Warp logic.

`SkillWatcher` subscribes to the same singleton for Warp home skill updates while continuing to use the existing home-directory and repository watching paths for non-Warp providers and project skills. The initial Warp home skill load and repository-scan load now share the same directory-read helper.

Because Warp home skills now live under the channel-aware `data_dir()/skills`, helper code also needs to recognize that path when determining provider, scope, and enclosing skill directory.

## Chosen design

### 1. Dedicated Warp watcher ownership
`WarpDataDirectoryWatcher` owns a `BulkFilesystemWatcher` directly instead of layering on top of `DirectoryWatcher`.

This keeps the abstraction boundary simple:
- project repositories still use `DirectoryWatcher`
- Warp-owned home/config directories use `WarpDataDirectoryWatcher`

That separation avoids broadening the generic repo watcher API for a Warp-specific use case.

### 2. Watch root preparation and registration
Each Warp-owned root is prepared before watcher registration, but that preparation happens outside the watcher constructor:
- call `create_dir_all()` during startup/setup
- keep watcher registration separate from root creation
- log failures before registration rather than implicitly recovering inside watcher construction

`data_dir()` is registered with a watcher-level `WatchFilter` that excludes `<data_dir>/worktrees`.

`config_local_dir()` is registered with `WatchFilter::accept_all()` when it is distinct from `data_dir()`.

### 3. Update normalization
`BulkFilesystemWatcherEvent` is converted into `RepositoryUpdate` so existing subscribers can keep using the same helper logic and event vocabulary.

`filter_repository_update()` is retained as a downstream helper to:
- keep only the paths relevant to a downstream consumer
- convert cross-boundary moves into add/delete updates when a move crosses the watched prefix boundary

### 4. Downstream consumers
`WarpConfig` remains subscription-based and does not reintroduce a direct `notify` watcher.

It continues to:
- reload themes, workflows, launch configs, and tab configs when updates touch the relevant directories
- perform those file-backed reloads via `ctx.spawn(...)` so disk reads happen off the model thread
- emit `WarpConfigUpdateEvent::Settings` when updates touch `user_preferences_toml_file_path()`

`FileMCPWatcher` subscribes to Warp watcher events for `data_dir()/.mcp.json` and keeps the existing logic for non-Warp providers. Warp remains on the Warp-specific watcher source, but shared helpers now cover:
- the startup parse path for a single config file
- the single-config incremental update path used by both Warp and non-Warp providers

`SkillWatcher` subscribes to Warp watcher events for `data_dir()/skills` and keeps the existing logic for non-Warp home providers and project repositories. The Warp home initialization path and repository scan path share the same helper for reading skill directories and emitting updates.

### 5. Channel-aware helper paths
The helper path logic is updated so Warp home skills and MCP config resolve through the active channel's data directory:
- Warp MCP home config resolves to `warp_data_mcp_config_file_path()`
- Warp home skills resolve to `data_dir()/skills`
- provider and scope classification recognize those paths as Warp home paths rather than project paths

## End-to-end flow
1. Warp startup prepares `data_dir()` and, when distinct, `config_local_dir()` before constructing `WarpDataDirectoryWatcher`.
2. `WarpDataDirectoryWatcher` registers `data_dir()` with a watcher-level filter that excludes `<data_dir>/worktrees`.
3. If `config_local_dir()` differs from `data_dir()`, `WarpDataDirectoryWatcher` registers that root with no extra filter.
4. The underlying `BulkFilesystemWatcher` emits `BulkFilesystemWatcherEvent`.
5. `WarpDataDirectoryWatcher` converts that event into `RepositoryUpdate` and emits `FilesChanged(update)`.
6. `WarpConfig`, `FileMCPWatcher`, and `SkillWatcher` inspect the update and react only when their owned paths were touched.
7. For themes, workflows, launch configs, and tab configs, `WarpConfig` reloads from disk on background tasks before applying the new state on the model thread.
8. For `settings.toml`, `WarpConfig` emits `WarpConfigUpdateEvent::Settings`, and `settings::init()` reloads public settings from disk.

## Risks and mitigations
- Distinct-root platforms regress settings hot reload.
  - Mitigation: register `config_local_dir()` separately when it differs from `data_dir()`.
- Worktree events leak back into Warp home consumers.
  - Mitigation: apply a watcher-level filter to `data_dir()` and let downstream consumers do any additional prefix filtering they need.
- Fresh installs fail to register watchers because the root directory is missing.
  - Mitigation: prepare each watch root during startup before registration.
- Path comparisons miss updates because watcher event paths and logical config paths may differ by symlink or canonical form.
  - Mitigation: `WarpConfig` continues checking both raw and canonicalized paths in `update_touches_dir()` and `update_touches_path()`.
- Warp home skill or MCP paths are misclassified as project paths.
  - Mitigation: centralize Warp-home path handling in the MCP and skills helper utilities.

## Testing and validation
- `cargo test -p warp --lib filter_repository_update_by_prefix_keeps_only_matching_paths`
  - validates the downstream filtering helper that skills consumers rely on
- `cargo test -p integration --test integration settings_file_hot_reload -- --nocapture`
  - verifies the end-to-end settings hot-reload behavior by rewriting `settings.toml` and asserting in-memory settings updates
- Code review validation:
  - `data_dir()` is watched with the `worktrees` exclusion
  - `config_local_dir()` is watched only when distinct
  - `WarpDataDirectoryWatcher` owns its own filesystem watcher instead of extending `DirectoryWatcher`
  - watch-root preparation happens before watcher registration instead of inside the watcher constructor
  - Warp home MCP and skill helpers resolve through the channel-aware data directory

## Follow-ups
- Add a focused test for `WarpDataDirectoryWatcher` root registration when `data_dir()` and `config_local_dir()` differ.
- Consider whether the remaining Warp-home helper changes in the MCP and skills layers can be reduced further without regressing path classification.
