# Remote DiffStateModel

Linear: [APP-4351](https://linear.app/warpdotdev/issue/APP-4351/update-diffstatemodel-api)

## Context

`DiffStateModel` (`app/src/code_review/diff_state.rs`) is a per-repo model that owns a `Repository` handle, fs watcher subscription, diff loading, metadata refresh, and mode selection. It emits `DiffStateModelEvent` with four variants: `CurrentBranchChanged`, `NewDiffsComputed`, `SingleFileUpdated`, `MetadataRefreshed`. `CodeReviewView` subscribes to these events and renders diffs identically regardless of how they were produced.

`WorkingDirectoriesModel` (`app/src/pane_group/working_directories.rs`) stores `diff_state_models: HashMap<PathBuf, ModelHandle<DiffStateModel>>` and lazily creates models via `get_or_create_diff_state_model`. `CodeReviewView` holds a `ModelHandle<DiffStateModel>` obtained from this map.

The remote server protocol (`crates/remote_server/proto/remote_server.proto`) uses length-prefixed protobuf over SSH stdio. Push events flow through `ClientEvent` → `RemoteServerManager::forward_client_event` → `RemoteServerManagerEvent` → app-layer subscribers. `ServerModel` (`app/src/remote_server/server_model.rs`) is the daemon-side orchestrator that dispatches `ClientMessage`s and sends `ServerMessage` responses and pushes.

Today `DiffStateModel` only works with local git repositories. To support code review on remote environments (SSH sessions), we need to split the model into local/remote variants behind a wrapper, add proto messages for diff state exchange, build a server-side `GlobalDiffStateModel` that manages diff state per (repo, mode), and implement a client-side `RemoteDiffStateModel` that receives server pushes.

## Proposed changes

### 1. Split DiffStateModel into wrapper + local + remote

Refactor `app/src/code_review/diff_state.rs` into a module directory `app/src/code_review/diff_state/`:
- `mod.rs` — `DiffStateModel` wrapper holding a `DiffStateBackend` enum (`Local` / `Remote`), delegating every read/write method to the active sub-model.
- `local.rs` — `LocalDiffStateModel` (renamed from the current `DiffStateModel`), retaining all existing behavior.
- `remote.rs` — `RemoteDiffStateModel`, initially a no-op stub with defaults for every read method.

The wrapper subscribes to the active sub-model and re-emits events via `forward_event`. `CodeReviewView` subscribes to the wrapper and renders diffs identically regardless of which backend is active. `DiffStateModelEvent` keeps its name — it's shared between local and remote.

**Additional mechanical changes in the split:**
- Wrap `NewDiffsComputed` payload in `Arc`: `Option<GitDiffWithBaseContent>` → `Option<Arc<GitDiffWithBaseContent>>` for cheap cloning during event forwarding.
- Simplify `DiffState::Loaded` to a unit variant (no inner `GitDiffData`). The diff payload is accessed via `DiffStateModel::get()` or arrives in the event itself.
- Update `WorkingDirectoriesModel`: cache key changes from `HashMap<PathBuf, ...>` to `HashMap<BufferLocation, ...>`, and `get_or_create_diff_state_model` takes `BufferLocation` instead of `PathBuf`. All call sites wrap paths in `BufferLocation::Local(...)`. `BufferLocation` (`app/src/code/buffer_location.rs`) already has `Local(PathBuf)` and `Remote(RemotePath)` variants with `Hash + Eq`.
- All callers in `code_review_view.rs`, `code_review_header/mod.rs`, `right_panel.rs` pass `ctx` to wrapper methods (the wrapper needs `AppContext` to dereference its inner `ModelHandle`).

### 2. Proto messages

Add to `crates/remote_server/proto/remote_server.proto`.

**Client → Server:**
- `GetDiffState { repo_path, mode }` — request/response. The server responds with a `GetDiffStateResponse` (snapshot or error), then pushes subsequent changes. Follows the `NavigatedToDirectory` → `NavigatedToDirectoryResponse` + `RepoMetadataSnapshot` pattern.
- `UnsubscribeDiffState { repo_path, mode }` — notification (fire-and-forget). Tells the server the client no longer needs updates for this (repo, mode).
- `DiscardFilesRequest { repo_path, files, should_stash, branch_name?, mode }` — request/response. Runs `git restore`/`git stash`/`git rm` on the remote filesystem for the specified files. `files` is a list of `FileStatusInfo { path, status }`. `should_stash` controls whether changes are stashed (recoverable) or discarded. `branch_name` specifies the branch to restore against (absent means HEAD). `mode` identifies which `(repo, mode)` diff state model the server should use — the server looks up the exact model via `DiffModelKey` rather than picking an arbitrary model for the repo.

**Server → Client:**
- `GetDiffStateResponse` — `oneof result { DiffStateSnapshot snapshot, DiffStateError error }`. Matches the `WriteFileResponse`/`RunCommandResponse` pattern.
- `DiffStateSnapshot` (push) — full state for a (repo, mode). Includes metadata + full `GitDiffData`. Pushed on structural changes (`NewDiffsComputed`).
- `DiffStateMetadataUpdate` (push) — metadata-only update for `MetadataRefreshed` events. Avoids re-serializing the entire diff payload on every 5-second throttled refresh.
- `DiffStateFileDelta` (push) — single-file diff update for `SingleFileUpdated` events. Carries one `FileDiff` + file path + updated metadata. Debounced at 2s on the server.
- `DiscardFilesResponse` — `oneof result { DiscardFilesSuccess, DiscardFilesError }`. Returned after processing a `DiscardFilesRequest`.

Wire into `ClientMessage.oneof` (field numbers 18–20) and `ServerMessage.oneof` (field numbers 18–22). Client: `get_diff_state = 18`, `unsubscribe_diff_state = 19`, `discard_files = 20`. Server: `get_diff_state_response = 18`, `diff_state_snapshot = 19`, `diff_state_metadata_update = 20`, `diff_state_file_delta = 21`, `discard_files_response = 22`. Current max field numbers: `ClientMessage` = 17 (`ResolveConflict`), `ServerMessage` = 17 (`ResolveConflictResponse`).

Sub-messages mirror the Rust domain types with two notable divergences:
- `DiffMode` and `GitFileStatus` use `oneof` (with per-variant wrapper messages) instead of the Rust `enum` + struct pattern, since `oneof` maps more faithfully to Rust tagged enums with per-variant data.
- `FileDiff` collapses `FileDiff` + `FileDiffAndContent` into a single message with an `optional string content_at_base` field. The Rust split exists for memory reasons (`!Clone` on the content-carrying variant); on the wire, every context that sends a `FileDiff` also requires base content for editor rendering (`set_base`), so there's no case where the field is absent by design — only absent for binary files or failed `git show`.

Conversion lives in a new `diff_state_proto.rs`, following the `repo_metadata_proto.rs` pattern.

**No `RefreshDiffMetadata` message** — the server pushes metadata changes automatically via its watcher, matching the `RepoMetadata` pattern.

**No `ChangeDiffMode` message** — mode changes are handled client-side: `RemoteDiffStateModel.set_diff_mode()` sends `UnsubscribeDiffState` for the old mode, resets internal state, then sends `GetDiffState` for the new mode (see §5). The server's per-model mode remains immutable (shared across connections), but the client model manages the transition internally.

**Rust wire types** (in a new shared module, e.g. `diff_state_wire.rs` — both client and server need these types):

```rust path=null start=null
/// Wire payload for a full diff state snapshot (after routing).
pub struct DiffStateSnapshotData {
    pub metadata: Option<DiffMetadata>,
    pub state: DiffState,
    pub diffs: Option<GitDiffData>,  // present when state is Loaded
}

/// Wire payload for a single-file diff delta (after routing).
pub struct DiffStateFileDeltaData {
    pub path: PathBuf,
    pub diff: Option<FileDiff>,
    pub metadata: Option<DiffMetadata>,
}
```

No new state enum — the existing `DiffState` has the right variants (`NotInRepository`, `Loading`, `Error(String)`, `Loaded`). After §1's changes, `Loaded` is a unit variant (no inner data); the diff payload is carried separately in `DiffStateSnapshotData.diffs`. The proto `oneof state` mirrors the variants directly.

**Wire ↔ event type bridging.** The wire `FileDiff` includes `content_at_base` (the file content at HEAD or merge-base), which the Rust side splits into `FileDiff` + `FileDiffAndContent`. On receipt, the `diff_state_proto.rs` conversion layer reconstructs `FileDiffAndContent { file_diff, content_at_head: proto.content_at_base }` from each wire `FileDiff`. The `RemoteDiffStateModel` wraps the full payload in `Arc` before emitting `NewDiffsComputed(Some(Arc::new(...)))`. This means remote code review editors receive base content eagerly — the same as local — and can call `set_base()` immediately without a separate RPC.

### 3. Server-side GlobalDiffStateModel

New file: `app/src/remote_server/diff_state_tracker.rs`.

```rust path=null start=null
#[derive(Hash, Eq, PartialEq, Clone)]
struct DiffModelKey {
    repo_path: StandardizedPath,
    mode: DiffMode,
}

pub struct GlobalDiffStateModel {
    states: HashMap<DiffModelKey, ModelHandle<LocalDiffStateModel>>,
    /// key → connections: used for push fan-out and orphan detection.
    key_to_connections: HashMap<DiffModelKey, HashSet<ConnectionId>>,
}
```

`DiffModelKey` uses `StandardizedPath` (not `RepositoryIdentifier`) because the server daemon only manages local-to-the-remote-host repositories — the `Remote` variant of `RepositoryIdentifier` is never used on the server side. This matches the existing `ServerModel` convention where per-connection state is keyed on `StandardizedPath` (e.g. `snapshot_sent_roots_by_connection`).

**Per-(repo, mode) models with immutable mode.** The server keys models on `(repo_path, mode)`.

**Lifecycle:**
1. `GetDiffState` arrives as a request. `GlobalDiffStateModel` looks up or creates a `LocalDiffStateModel` for the key. If already loaded, responds immediately. If loading, uses `ctx.spawn` to respond once `NewDiffsComputed` fires. Reuses `Repository` handles from `DetectedRepositories` (already detected by prior `NavigatedToDirectory`). If no repository has been detected yet (e.g. `GetDiffState` arrives before `NavigatedToDirectory`), responds with `DiffStateError` — the client retries after `NavigatedToDirectory` completes.
2. After responding, subsequent model events are pushed to subscribed connections only via `send_to_diff_state_subscribers(key, msg)`, which looks up `connections_by_key[key]`. Targeted sends avoid broadcasting large diff payloads (~500KB–2MB).
3. `UnsubscribeDiffState` calls `unsubscribe_connection` for the specific key. If no subscribers remain, the model is dropped.
4. `remove_connection(conn_id)` iterates `key_to_connections` to find all keys the connection belongs to and calls `unsubscribe_connection` for each, dropping orphaned models.

**Event → push mapping:**
- `NewDiffsComputed` → full `DiffStateSnapshot`
- `MetadataRefreshed` → `DiffStateMetadataUpdate` (metadata only, no diffs)
- `CurrentBranchChanged` → `DiffStateMetadataUpdate` (metadata only — diffs for the new branch haven't been computed yet; `NewDiffsComputed` follows with actual diffs)
- `SingleFileUpdated` → `DiffStateFileDelta` (debounced at 2s)

### 4. RemoteDiffStateModel implementation

Fill in the no-op stub in `diff_state/remote.rs` (created in §1) to:
- Hold `repo_id: RepositoryIdentifier` (always `Remote` variant), `mode: DiffMode` (mutable), `state: DiffState`, `metadata: Option<DiffMetadata>`.
- Apply incoming `DiffStateSnapshotData`, `DiffStateMetadataUpdate`, and `DiffStateFileDeltaData` from server pushes.
- Reconstruct `FileDiffAndContent { file_diff, content_at_head }` from wire `FileDiff` (extracting `content_at_base` → `content_at_head`) and wrap `GitDiffData` → `Arc<GitDiffWithBaseContent>` before emitting events.
- Emit `DiffStateModelEvent` variants matching the server push mapping (§3).
- Own the subscribe/unsubscribe lifecycle for mode changes via `set_diff_mode()` (see §5).

`DiffMode` is mutable on the client-side `RemoteDiffStateModel`, matching `LocalDiffStateModel`'s existing pattern where `set_diff_mode` mutates the mode field in place and triggers a reload. The server-side model remains immutable (keyed on `(repo_path, mode)` and shared across connections), but that constraint doesn't apply to the per-client remote model. This keeps the wrapper's delegation symmetric between `Local` and `Remote` backends, and avoids the need to destroy/recreate the model handle on mode changes (which would require re-wiring event subscriptions and introduces subscribe-before-request race conditions).

**Required read API surface** (defined in the wrapper's delegation interface):
- Core: `get()`, `diff_mode()`, `get_current_branch_name()`, `get_main_branch_name()`, `get_stats_for_current_mode()`, `get_uncommitted_stats()`, `has_head()`
- Git operations (stubs for v1 — `GitOperationsInCodeReview` won't be enabled for remote): `is_git_operation_blocked()`, `pr_info()`, `is_pr_info_refreshing()`, `is_on_main_branch()`, `unpushed_commits()`, `upstream_ref()`, `upstream_differs_from_main()`
- Mutations: `set_diff_mode()`, `load_diffs_for_current_repo()`, `set_code_review_metadata_refresh_enabled()`, `discard_files()`, `refresh_metadata_and_pr_info()`

For v1, mutation methods that are local-only (`load_diffs_for_current_repo`, `refresh_metadata_and_pr_info`, `set_code_review_metadata_refresh_enabled`) remain no-ops on `RemoteDiffStateModel` — the server drives all state.

### 5. Mode changes and unsubscribe

`RemoteDiffStateModel.set_diff_mode()` handles mode transitions internally, mirroring how `LocalDiffStateModel.set_diff_mode()` mutates mode and triggers a reload:
1. Sends `UnsubscribeDiffState { repo_path, mode: old_mode }` to the server.
2. Updates `self.mode` to the new mode.
3. Resets internal state: `self.state = DiffState::Loading`, clears `self.diffs` and `self.metadata`.
4. Emits `NewDiffsComputed(None)` so the view shows a loading spinner.
5. Sends `GetDiffState { repo_path, mode: new_mode }` to the server.
6. Server responds → model applies the snapshot, transitions to `Loaded`, emits `NewDiffsComputed(Some(...))`.

Since the `ModelHandle` never changes, the wrapper's event subscription (set up once at construction) remains valid across mode changes — no re-wiring needed.

**Unsubscribe cases:** code review pane close, mode change, repo change (cycling), connection drop, `drop_unused_diff_state_models` (tab close).

### 6. New ClientEvent / RemoteServerManagerEvent variants

`ClientEvent` carries raw proto-derived data (`StandardizedPath`, `DiffMode`). `forward_client_event` in `RemoteServerManager` attaches `host_id`, constructs `RepositoryIdentifier::Remote(...)`, and emits the corresponding manager event. This follows the `RepoMetadataSnapshotReceived` pattern.

Three new variants each for `ClientEvent` and `RemoteServerManagerEvent`:
- `DiffStateSnapshotReceived`
- `DiffStateMetadataUpdateReceived`
- `DiffStateFileDeltaReceived`

`push_message_to_event` in `RemoteServerClient` maps the new `ServerMessage` variants to `ClientEvent` variants.

### 7. WorkingDirectoriesModel integration

After §1's cache key migration, `get_or_create_diff_state_model` accepts `BufferLocation` and the map uses `HashMap<BufferLocation, ModelHandle<DiffStateModel>>`. When a `BufferLocation::Remote(...)` is passed, `DiffStateModel::new` (the wrapper constructor) creates a `RemoteDiffStateModel` in `Loading` state, subscribes to its events, and sends `GetDiffState` to the server — mirroring how the `Local` branch creates and subscribes to `LocalDiffStateModel` today.
