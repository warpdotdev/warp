# File-based MCP config edit actions — Tech Spec
Product spec: `specs/GH9882/product.md`
GitHub issue: https://github.com/warpdotdev/warp-external/issues/9882

## Context
File-based MCP discovery already tracks enough metadata to find the config file that defines a detected server, but the settings UI does not turn that metadata into an editor-open action.

Relevant current code:
- `app/src/ai/mcp/mod.rs:43` — `home_config_file_path(provider)` returns the global provider config path for the current machine.
- `app/src/ai/mcp/mod.rs:69` — `MCPProvider` enumerates the file-based providers.
- `app/src/ai/mcp/mod.rs:88` — `MCPProvider::home_config_path()` defines provider-specific global relative paths.
- `app/src/ai/mcp/mod.rs:99` — `MCPProvider::project_config_path()` defines provider-specific project relative paths.
- `app/src/ai/mcp/file_mcp_watcher.rs:565` — `providers_in_scope(root_path, watched_dir)` constructs the concrete config paths scanned by provider/root.
- `app/src/ai/mcp/file_mcp_watcher.rs:547` — config parsing emits `FileMCPWatcherEvent::ConfigParsed { root_path, provider, servers }`.
- `app/src/ai/mcp/file_based_manager.rs:27` — `file_based_servers_by_root` maps `root_path -> provider -> server hash set`.
- `app/src/ai/mcp/file_based_manager.rs:388` — `directory_paths_for_installation_and_provider(uuid, provider)` exposes root paths for display chips, but not full config file paths.
- `app/src/settings_view/mcp_servers/list_page.rs:587` — `ServerCardEvent::Edit` is currently forwarded as `MCPServersListPageViewEvent::Edit` for every card type.
- `app/src/settings_view/mcp_servers/list_page.rs:1571` — file-based title chips are derived from `FileBasedMCPManager::directory_paths_for_installation_and_provider`.
- `app/src/settings_view/mcp_servers/list_page.rs:1628` — `create_file_based_spawned_card` builds promoted file-based cards and explicitly overrides `show_edit_config_icon_button: false`.
- `app/src/settings_view/mcp_servers/server_card.rs:171` — installed/running card statuses normally enable `show_edit_config_icon_button`.
- `app/src/settings_view/mcp_servers/server_card.rs:275` — error cards enable `show_edit_config_text_button`.
- `app/src/settings_view/mcp_servers/server_card.rs:807` and `app/src/settings_view/mcp_servers/server_card.rs:841` — both edit icon and `Edit config` text button dispatch `ServerCardAction::Edit(item_id)`.
- `app/src/settings_view/mcp_servers/server_card.rs:969` — `ServerCardAction::Edit` emits `ServerCardEvent::Edit`.
- `app/src/settings_view/mcp_servers_page.rs:355` — the MCP settings page handles `MCPServersListPageViewEvent::Edit` by opening the templatable MCP edit page.
- `app/src/workspace/view.rs:5588` — project-rules `AIFactViewEvent::OpenFile` opens local files directly in Warp with `Workspace::open_code`.

The required change is small but crosses three boundaries: file-based discovery metadata, MCP card event routing, and workspace editor opening.

## Proposed changes
### 1. Add a file-based config path resolver to `FileBasedMCPManager`
Add a public helper on `FileBasedMCPManager`, behind the existing `local_fs` implementation, that maps a file-based installation UUID to the concrete config files known to define it. Suggested shape:

```rust path=null start=null
pub fn config_paths_for_installation(&self, uuid: Uuid) -> Vec<PathBuf>
```

Implementation details:
- Reuse `get_hash_by_uuid(uuid)` to find the installation hash.
- Iterate `file_based_servers_by_root` and select `(root_path, provider)` pairs whose hash set contains that hash.
- Convert each `(root_path, provider)` pair to the same concrete config path that `FileMCPWatcher` scans:
  - For project roots, use `root_path.join(provider.project_config_path())`.
  - For non-Warp global roots, use `root_path.join(provider.home_config_path())`.
  - For global Warp, use `warp_core::paths::warp_home_mcp_config_file_path()` or the same path exposed by `warp_managed_mcp_config_path()`, not `root_path.join(provider.home_config_path())`, because the stored root is Warp's data directory rather than the home directory.
- Sort and dedupe the returned paths for deterministic behavior. A deterministic `(provider, root_path, config_path)` target struct is also acceptable if it makes testing or future UI easier, but the current UI only needs the path.

Keep the resolver in `FileBasedMCPManager` rather than `MCPServersListPageView`; the manager owns the meaning of `file_based_servers_by_root` and already contains the global-vs-project root logic used by `is_global_server`, `is_global_warp_server`, and `spawn_root_for_installation`.

### 2. Route file-based edit events to file opening instead of the edit page
Change `MCPServersListPageView::handle_server_card_event` so `ServerCardEvent::Edit(item_id)` branches by card type:
- `ServerCardItemId::FileBasedMCP(uuid)` calls a new `open_file_based_config(uuid, ctx)` helper and does not emit `MCPServersListPageViewEvent::Edit`.
- Existing templatable/gallery edit routing remains unchanged and still emits `MCPServersListPageViewEvent::Edit`.

`open_file_based_config` should:
1. Ask `FileBasedMCPManager::as_ref(ctx).config_paths_for_installation(uuid)` for targets.
2. Pick the first path from the deterministic list.
3. If no path exists, show an error toast such as `Could not find the config file for this MCP server.` and return.
4. If the path no longer exists or cannot be opened, show an error toast such as `Could not open MCP config file: <display path>` and return. Use safe logging for full path details if needed.
5. Find the current `Workspace` view the same way `open_logs_for_server` does in `app/src/settings_view/mcp_servers/list_page.rs:552`.
6. Call `workspace.open_code(path.clone(), None, CodeSource::Link { path, range_start: None, range_end: None }, ctx)`.

This keeps the source of the action in the MCP list page, while using the workspace's in-Warp editor path instead of the helper that can route to the user's configured default editor. The action is meant for quick config edits from settings, so it should always open inside Warp rather than handing off to an external editor.

### 3. Enable the existing edit affordance for promoted file-based cards
Update `create_file_based_spawned_card` in `app/src/settings_view/mcp_servers/list_page.rs:1628`:
- Remove the hard-coded `show_edit_config_icon_button: false` override for file-based cards.
- Replace it with `show_edit_config_icon_button: has_config_target`, where `has_config_target` is `!FileBasedMCPManager::as_ref(ctx).config_paths_for_installation(uuid).is_empty()`.
- Keep `show_share_icon_button: false`; file-based servers are still not shareable.
- Keep `show_log_out_icon_button: uses_oauth`.

Because `ServerCardStatus::Error` already sets `show_edit_config_text_button: true`, the same event-routing fix in step 2 makes the error text button work. If the implementation decides to hide the text button when no config target exists, add an option override for file-based error cards only after confirming it does not affect templatable error cards.

### 4. Preserve existing non-file-based behavior
Do not change `MCPServersSettingsPageView::handle_list_view_event` for templatable edits except for any enum additions needed by the chosen implementation. The default `Edit` event continues to navigate to:

```rust path=null start=null
MCPServersSettingsPage::Edit { item_id: Some(*mcp_item_id) }
```

Do not change `ServerCardView`'s edit dispatch. Both the icon and text button should continue to emit `ServerCardEvent::Edit`; the parent list page decides what that edit means for each card type.

### 5. User-visible error handling
Add or reuse a toast helper in `MCPServersListPageView` for failures. `MCPServersSettingsPageView` already has `add_error_toast`, but the list page does not expose it. A small list-page helper can use `ToastStack::handle(ctx)` and `DismissibleToast::error(...)`, matching other settings surfaces.

Expected failure cases:
- The file-based installation UUID is no longer tracked.
- The installation has no hash.
- No `(root_path, provider)` mapping references the hash.
- A stale path was resolved but the file no longer exists.
- No workspace view is available in the current window.

All of these should log enough for developers to debug, but the user-facing result should be a concise non-blocking error toast, never a silent no-op.

## End-to-end flow
1. `FileMCPWatcher` scans a supported config file and emits `ConfigParsed { root_path, provider, servers }`.
2. `FileBasedMCPManager::apply_parsed_servers` stores the server installation by hash and records the hash under `file_based_servers_by_root[root_path][provider]`.
3. `MCPServersListPageView::create_file_based_spawned_card` sees the installation has a templatable-manager state, promotes it into the installed section, and enables the edit icon when `config_paths_for_installation(uuid)` is non-empty.
4. The user clicks either the hover edit icon or the error-state `Edit config` button.
5. `ServerCardView` emits `ServerCardEvent::Edit(ServerCardItemId::FileBasedMCP(uuid))`.
6. `MCPServersListPageView` resolves the config path through `FileBasedMCPManager` and asks `Workspace::open_code` to open the local file directly in Warp.
7. The file opens in Warp's editor pane. Saving the file later is handled by the existing editor and file-based MCP watcher paths.

## Testing and validation
### Unit tests
- Extend `app/src/ai/mcp/file_based_manager_tests.rs` with tests for `config_paths_for_installation`:
  - Project-scoped Claude server resolves to `<repo>/.mcp.json`.
  - Project-scoped Warp server resolves to `<repo>/.warp/.mcp.json`.
  - Project-scoped Codex server resolves to `<repo>/.codex/config.toml`.
  - Project-scoped Other Agents server resolves to `<repo>/.agents/.mcp.json`.
  - Global Claude/Codex/Other Agents servers resolve under the home directory with `home_config_path()`.
  - Global Warp servers resolve to Warp's managed global MCP config file, not `~/.warp/.warp/.mcp.json`.
  - A server referenced by multiple roots/providers returns a sorted, deduped list.
  - Unknown UUID returns an empty list.

- If practical in the existing test harness, add settings-view tests around `create_file_based_spawned_card` or an extracted pure helper to verify:
  - File-based cards with a config target enable `show_edit_config_icon_button`.
  - File-based cards without a config target do not enable a broken icon.
  - Templatable card options are unchanged.

### Manual validation
- With `FeatureFlag::FileBasedMcp` enabled, add a project `.mcp.json` with a valid file-based server. Start it from MCP settings, hover the promoted card, click the edit icon, and confirm the project `.mcp.json` opens in Warp's editor.
- Repeat for `.warp/.mcp.json`, `.codex/config.toml`, and `.agents/.mcp.json` project configs.
- Add a global provider config (`~/.claude.json`, `~/.codex/config.toml`, `~/.agents/.mcp.json`, or Warp's global `.mcp.json`), let the server spawn or start it, click edit, and confirm the global file opens.
- Force a server startup error and click `Edit config`; confirm it opens the same defining file rather than navigating to the MCP edit page.
- Delete or rename the config file after the card is visible, then click edit; confirm a non-blocking error toast appears and the app does not panic.
- Verify templatable MCP cards still open the MCP edit page, gallery cards still install, logs still open in a terminal pane, and file-based start/stop toggles still spawn/shutdown servers.

### Build validation
- Run the narrow Rust tests for file-based MCP manager if available in the local harness.
- Run `cargo fmt`/repository formatting for touched Rust files during implementation.
- Run the repository's presubmit or the closest MCP/settings test subset before shipping the implementation.

## Risks and mitigations
### Risk: wrong global Warp path
Warp global file-based MCP config uses Warp-managed paths and stores `warp_data_dir()` as the root, unlike third-party providers that use the home directory. A naive `root.join(provider.home_config_path())` would produce an invalid nested path.

Mitigation: put root/provider-to-config-path conversion in `FileBasedMCPManager`, add explicit global Warp unit coverage, and reuse existing `warp_core::paths::warp_home_mcp_config_file_path()` / `warp_managed_mcp_config_path()` behavior.

### Risk: duplicate server definitions make the target surprising
`FileBasedMCPManager` deduplicates servers by hash, so one displayed card can be referenced by multiple config files.

Mitigation: product accepts deterministic first-target behavior for this iteration. Sort and dedupe targets, and keep the helper returning all paths so a future picker can be added without reworking manager state.

### Risk: accidentally routing templatable edits to file opening
The same `ServerCardEvent::Edit` is used by all card types.

Mitigation: branch on `ServerCardItemId` in `MCPServersListPageView`, route only `FileBasedMCP` through file opening, and leave existing `MCPServersListPageViewEvent::Edit` behavior intact for all other item IDs.

### Risk: non-local builds
File opening and file-based manager implementations are gated by `local_fs` in several places.

Mitigation: keep new imports and helper calls behind existing `#[cfg(feature = "local_fs")]` patterns or provide no-op/empty dummy-manager equivalents for non-local builds.

## Follow-ups
- Add a provider/scope picker if users report that deterministic first-target behavior is confusing for deduplicated servers.
- Consider adding an explicit `Open config` affordance to not-yet-spawned file-based template cards in the detected-provider sections, as a separate UX decision.
