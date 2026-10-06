# File Manager host labels during SSH — Tech Spec
Product spec: `specs/GH16333/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/16333

## Context
The File Manager / Project Explorer already models local and remote roots separately, but its row label is path-derived and does not surface origin information.

Relevant code at researched commit `f571865ca3f2bd6e769867b468d620791865592a`:

- [`app/src/code/file_tree/view.rs:218 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view.rs#L218) — `RootDirectory` stores per-root state.
- [`app/src/code/file_tree/view.rs:231-L240 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view.rs#L231-L240) — `remote_host_id: Option<HostId>` is already the local-vs-remote discriminator for roots.
- [`app/src/code/file_tree/view.rs:370-L467 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view.rs#L370-L467) — `insert_or_update_remote_roots` creates or updates remote roots from `RemoteRepositoryIdentifier` and preserves their `HostId`.
- [`app/src/code/file_tree/view.rs:909-L983 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view.rs#L909-L983) — `set_remote_root_directories` inserts remote root placeholders before metadata arrives.
- [`app/src/code/file_tree/view.rs:1006-L1072 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view.rs#L1006-L1072) — `set_root_directories` manages local roots while preserving existing remote roots.
- [`app/src/workspace/view/left_panel.rs:397-L465 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/workspace/view/left_panel.rs#L397-L465) — working-directory changes are split into local `PathBuf`s and remote `RemoteRepositoryIdentifier`s before calling `FileTreeView::set_root_directories` and `set_remote_root_directories`.
- [`app/src/workspace/view/left_panel.rs:733-L800 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/workspace/view/left_panel.rs#L733-L800) — active-pane changes perform the same local/remote split and are another handoff point for File Manager roots.
- [`app/src/terminal/view.rs:24361-L24408 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/terminal/view.rs#L24361-L24408) — `pwd_as_local_or_remote` constructs `LocalOrRemotePath::Remote(RemotePath::new(host_id, path))` for remote session CWDs.
- [`app/src/code/file_tree/view/render.rs:12-L57 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view/render.rs#L12-L57) — `FileTreeItem::to_render_state` currently derives directory display text from `directory.path.file_name()` only.
- [`app/src/code/file_tree/view.rs:1931-L2025 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/code/file_tree/view.rs#L1931-L2025) — `render_item` uses `RenderState.display_name` for visible text and interaction IDs.
- [`crates/warp_util/src/remote_path.rs:8-L15 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/crates/warp_util/src/remote_path.rs#L8-L15) — `RemotePath` preserves the `HostId` plus remote path for remote file identity.
- [`crates/remote_server/src/manager.rs:1798-L1804 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/crates/remote_server/src/manager.rs#L1798-L1804) — `RemoteServerManager::host_label` returns a user-facing connection label for a connected host.
- [`app/src/terminal/writeable_pty/remote_server_controller.rs:566-L616 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/terminal/writeable_pty/remote_server_controller.rs#L566-L616) — SSH connection labels are derived from `user`, bootstrap hostname, and parsed SSH host.
- [`app/src/terminal/model/session.rs:903-L925 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/app/src/terminal/model/session.rs#L903-L925) — `Session::hostname()` exposes the bootstrapped session hostname.
- [`crates/warp_terminal/src/model/session.rs:1-L18 @ f571865c`](https://github.com/warpdotdev/warp/blob/f571865ca3f2bd6e769867b468d620791865592a/crates/warp_terminal/src/model/session.rs#L1-L18) — `get_local_hostname()` returns the client-machine hostname.

The existing data model already has enough identity to avoid changing path identity, remote file opening, or remote metadata protocols. The implementation should decorate root-row display text, not change root keys or file identities.

## Proposed changes

### 1. Add origin label state to `FileTreeView`
Add small display-only state to `FileTreeView`:

- `local_origin_label: Option<String>` or a cached `String` initialized from `get_local_hostname()` with fallback `Local`.
- A helper to resolve a remote origin label from `remote_host_id`, preferably through `RemoteServerManager::host_label(host_id)`, with fallback `Remote`.

Keep this state out of `FileTreeIdentifier`, `RootDirectory.entry`, `FileTreeEntry`, and `LocalOrRemotePath`. Paths and host-scoped identity should remain unchanged.

If calling `get_local_hostname()` during `FileTreeView::new` is acceptable, cache the result once and log/fallback on error. If construction-time hostname lookup is undesirable for tests or WASM builds, hide it behind a small helper with `cfg` fallback and inject/override it in tests.

Do not use raw `HostId` as user-facing text. `HostId` is an opaque deduplication key carried by `RemotePath` and `RemoteRepositoryIdentifier`; it is useful for model routing but is not guaranteed to be the SSH hostname the user recognizes.

### 2. Compute labels only for mixed local/remote root sets
Add a helper on `FileTreeView`, for example:

- `fn should_show_origin_labels(&self) -> bool`
  - returns true when `displayed_root_directories()` contains at least one root where `RootDirectory::is_remote()` is false and at least one root where it is true.

Add another helper:

- `fn display_name_for_item(&self, id: &FileTreeIdentifier, base_name: String, app: &AppContext) -> String`
  - returns `base_name` unchanged for non-root items (`id.index != 0` or item depth > 0).
  - returns `base_name` unchanged when `should_show_origin_labels()` is false.
  - for local root headers, returns `format!("{base_name} ({local_origin_label})")`.
  - for remote root headers, returns `format!("{base_name} ({remote_origin_label})")`.

Prefer checking root-ness through item depth (`depth == 0`) rather than assuming index `0` forever, but `FileTreeIdentifier.index == 0` is currently true for root headers after flattening. If exposing depth requires a method on `FileTreeItem`, add `fn depth(&self) -> usize` or include an `is_root` flag in the render state.

### 3. Keep render-state conversion path-scoped, then decorate in the view
Do not make `FileTreeItem::to_render_state` query app models or remote-server state. It should continue to compute the base file or folder display name from metadata. Decorate the returned `RenderState.display_name` in `FileTreeView::render_item` where the view has access to `id`, root state, and `AppContext`.

The least invasive implementation is:

1. Change `render_item(&self, id, appearance)` to accept `app: &AppContext` or a precomputed label context from the `UniformList` closure.
2. Build the base `render_state` via `item.to_render_state(...)`.
3. If the item is a root header and mixed roots are visible, replace `render_state.display_name` with the decorated text before creating `item_position_id` and before calling `render_item_with_hover`.

Because `render_item_with_hover` consumes `RenderState`, mutate the display name before `RenderState` is moved into the hover closure.

### 4. Add tooltip or accessibility fallback for long labels
The current row rendering clips text through `Shrinkable` and `Clipped`. If there is already a standard tooltip pattern for file-tree rows, reuse it. If not, add a lightweight hover tooltip for root rows only when origin labels are shown, using the full decorated label or full path plus origin. Keep child rows unchanged to avoid adding a tooltip to every file-tree item as part of this feature.

This is a product nice-to-have but should not block the core fix if the left-panel text already clips acceptably. If deferred, capture it as a follow-up in the implementation PR and ensure the visible suffix is still present for normal-width panels.

### 5. Refresh on remote label availability without resetting tree state
`RemoteServerManager::host_label(host_id)` is derived when `connect_session` stores the session label. Remote roots may exist before a label is available. The render path can query `host_label` dynamically, so a normal view notification should update the label without changing root keys.

If `FileTreeView` does not already re-render when the remote-server manager emits relevant events, subscribe while active to `RemoteServerManagerEvent::SessionConnected`, `SessionDisconnected`, and `HostDisconnected` and call `ctx.notify()` when any displayed remote root's `HostId` matches. Do not rebuild flattened items solely to update labels.

### 6. Keep local/remote actions unchanged
No changes are needed to:

- `select_and_execute_item_at_id`, which already uses `root_dir.is_remote()` and `remote_host_id` for remote file opens.
- `context_menu_items`, which already gates unsupported remote actions.
- `set_remote_root_directories`, which already inserts remote placeholders with `remote_host_id`.
- `set_root_directories`, which already preserves remote roots while updating local roots.

The implementation should avoid using decorated labels in path operations. `item.path()`, `FileTreeIdentifier.root`, and `RemotePath` remain the source of truth.

## Testing and validation

### Unit tests
Add or extend tests in `app/src/code/file_tree/view/view_tests.rs`:

1. Mixed local/remote roots show origin labels on top-level roots.
   - Set a local root and a remote root with the same basename.
   - Assert the root-row display labels include distinct local and remote origin text.
   - Prefer a small helper that exercises the new label helper directly rather than brittle pixel snapshots.

2. Local-only roots keep current labels.
   - Set one or more local roots.
   - Assert root display names equal the directory basenames with no hostname suffix.

3. Remote-only roots keep current labels unless mixed roots are present.
   - Insert a remote root with `remote_host_id` but no local roots.
   - Assert the display label remains the basename.

4. Child directories are not decorated.
   - Expand a root that has child directories.
   - Assert only the root header gets the origin suffix.

5. Missing labels fall back safely.
   - Simulate local hostname lookup failure or empty local label and assert `Local` fallback.
   - Simulate a remote `HostId` with no `RemoteServerManager::host_label` and assert `Remote` fallback.

6. Labels do not alter identity.
   - Select or execute a decorated local root child and assert emitted paths are unchanged.
   - Select or execute a decorated remote file and assert the emitted `LocalOrRemotePath::Remote` still carries the original `HostId` and path.

If constructing remote roots in existing tests is cumbersome, add a narrow `#[cfg(test)]` helper on `FileTreeView` to insert a `RootDirectory` with `remote_host_id` and a simple `FileTreeEntry`, matching the existing private-test style in this module.

### Manual validation
1. On Windows, open a local repository or directory named `COHE`.
2. SSH into a host that has a remote directory also named `COHE` and ensure the remote server feature is active so a remote File Manager tree appears.
3. Confirm the File Manager shows two root labels like `COHE (local-hostname)` and `COHE (remote-hostname)` or an equivalent design-approved format.
4. Expand/collapse both roots and open a local and remote file to confirm routing is unchanged.
5. Narrow the left panel and verify truncation is acceptable and the origin remains discoverable.

## Parallelization
Parallel implementation is not recommended for this feature. The change is small and tightly coupled to `FileTreeView` rendering and its unit tests; splitting it across agents would add merge overhead around the same file. A single implementer should update the view helper, render call site, and tests on one branch.

## Risks and mitigations
- **Risk: exposing opaque host IDs to users.** Use `RemoteServerManager::host_label(host_id)` or `Session::hostname()` where available; reserve `Remote` as the fallback instead of displaying raw `HostId`.
- **Risk: label changes reset tree state.** Keep labels render-only and avoid changing `displayed_directories`, root keys, flattened paths, or selection identifiers.
- **Risk: layout noise in local-only workflows.** Gate suffixes on mixed local/remote roots so existing local-only File Manager behavior is unchanged.
- **Risk: tests become dependent on actual machine hostname.** Inject or override the local-origin label in tests so assertions use deterministic values.

## Follow-ups
- If design review prefers chips or a two-column visual treatment instead of `name (origin)` text, keep the same label helper and swap only the row rendering presentation.
- If multiple remote hosts become common in one pane, consider adding color/icon affordances in addition to text labels, but text labels should remain the accessibility source of truth.
