# RepoMetadataModel Tech Spec
## Problem Statement
`RepositoryMetadataModel` is a singleton that tracks repositories and their file tree state. It currently only supports local file trees backed by a filesystem watcher. To support remote development (SSH), we need a model that can also hold file tree state sourced from a remote server.
The tech design ("Remote code model sync") proposes a generic wrapper `RepoMetadataModel` that dispatches to environment-specific sub-models. This spec details the implementation of that wrapper, the new `RemoteRepoMetadataModel` (client-side only, no syncing/indexing yet), and the consumer migration path.
## Current State
### Key types (all in `repo_metadata` crate)
* **`RepositoryMetadataModel`** (`model.rs`) — singleton, holds `HashMap<CanonicalizedPath, IndexedRepoState>` + an optional `BulkFilesystemWatcher`. Subscribes to `DetectedRepositories` for auto-indexing and the watcher for incremental updates.
* **`FileTreeState`** — holds a `FileTreeEntry` (the flattened map store), a `Vec<Gitignore>`, and an optional `ModelHandle<Repository>`.
* **`FileTreeEntry`** (`file_tree_store.rs`) — wraps `FileTreeMapStore` (parent→children + path→metadata hash maps) plus a `root_path: Arc<Path>`.
* **`CanonicalizedPath`** (`lib.rs`) — a `PathBuf` wrapper that `dunce::canonicalize`s on construction. Used as the HashMap key for repositories.
* **`SessionId`** (`app/src/terminal/model/session.rs`) — `u64` wrapper identifying a terminal session, already used to distinguish SSH sessions.
### Consumers in `app/`
* **`FileTreeView`** (`code/file_tree/view.rs`) — stores a `ModelHandle<RepositoryMetadataModel>`, subscribes to events, calls `get_repository`, `repository_state`, `is_lazy_loaded_path`, `load_directory`, `index_lazy_loaded_path`, `remove_lazy_loaded_path`.
* **`FileSearchModel`** (`search/files/model.rs`) — subscribes to `RepositoryMetadataEvent`, calls `has_repository`, `get_repo_contents`.
* **`SkillWatcher`** (`ai/skills/file_watchers/skill_watcher.rs`) — subscribes to `RepositoryMetadataEvent`, calls `RepositoryMetadataModel::as_ref(ctx)` for tree queries.
## Proposed Changes
### 1. New types
#### `RepositoryIdentifier`
A discriminated identifier for repositories across local and remote environments.
```rust
/// Identifies a repository across local and remote environments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RepositoryIdentifier {
    Local(CanonicalizedPath),
    Remote(RemoteRepositoryIdentifier),
}
```
#### `RemoteRepositoryIdentifier`
Pairs a session ID with the server-side path. Uses raw `PathBuf` because the path lives on the remote machine and cannot be canonicalized locally.
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteRepositoryIdentifier {
    pub session_id: SessionId,
    pub path: PathBuf,
}
```
`SessionId` will be moved from `app/src/terminal/model/session.rs` to `warp_core` so that `repo_metadata` can depend on it directly without circular crate dependencies.
### 2. `LocalRepoMetadataModel` (rename of existing model)
The existing `RepositoryMetadataModel` is renamed to `LocalRepoMetadataModel`. Its API is unchanged:
* `new(ctx)` — sets up watcher + `DetectedRepositories` subscription.
* `index_directory`, `index_lazy_loaded_path`, `load_directory`, `remove_lazy_loaded_path`, `remove_repository`.
* `get_repository`, `repository_state`, `has_repository`, `is_lazy_loaded_path`, `get_repo_contents`.
* Emits `RepositoryMetadataEvent` (unchanged).
The rename is mechanical: update the struct name, the `impl Entity`, `impl SingletonEntity`, and all import sites.
### 3. `RemoteRepoMetadataModel` (new, client-side only)
A model that holds file tree state for repositories on remote servers. In this initial phase it has **no syncing or indexing** — state is populated externally (e.g. by a future remote client model or via test helpers).
```rust
pub struct RemoteRepoMetadataModel {
    repositories: HashMap<RemoteRepositoryIdentifier, IndexedRepoState>,
}
```
#### Events
Re-uses the same event enum shape but scoped to remote identifiers:
```rust
#[derive(Debug)]
pub enum RemoteRepositoryMetadataEvent {
    RepositoryUpdated { id: RemoteRepositoryIdentifier },
    RepositoryRemoved { id: RemoteRepositoryIdentifier },
    FileTreeUpdated { ids: Vec<RemoteRepositoryIdentifier> },
    FileTreeEntryUpdated { id: RemoteRepositoryIdentifier },
}
```
#### Read-only query API
Matches the local model's query surface:
* `get_repository(&self, id: &RemoteRepositoryIdentifier) -> Option<&FileTreeState>`
* `has_repository(&self, id: &RemoteRepositoryIdentifier) -> bool`
* `repository_state(&self, id: &RemoteRepositoryIdentifier) -> Option<&IndexedRepoState>`
* `get_repo_contents(&self, id: &RemoteRepositoryIdentifier, args: GetContentsArgs) -> Option<Vec<RepoContent<'_>>>`
#### Write API (for future sync + test use)
* `insert_repository(&mut self, id: RemoteRepositoryIdentifier, state: FileTreeState, ctx: &mut ModelContext<Self>)` — inserts/replaces state, emits `RepositoryUpdated`.
* `remove_repository(&mut self, id: &RemoteRepositoryIdentifier, ctx: &mut ModelContext<Self>)` — removes state, emits `RepositoryRemoved`.
* `update_file_tree_entry(&mut self, id: &RemoteRepositoryIdentifier, entry: FileTreeEntry, ctx: &mut ModelContext<Self>)` — replaces the entry within an existing `FileTreeState`, emits `FileTreeEntryUpdated`.
These will be the integration points for the future remote sync layer.
### 4. `RepoMetadataModel` wrapper
A singleton that holds handles to both sub-models and provides a unified query API keyed by `RepositoryIdentifier`.
```rust
pub struct RepoMetadataModel {
    local: ModelHandle<LocalRepoMetadataModel>,
    remote: ModelHandle<RemoteRepoMetadataModel>,
}
```
#### Construction
```rust
impl RepoMetadataModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let local = ctx.add_model(|ctx| LocalRepoMetadataModel::new(ctx));
        let remote = ctx.add_model(|ctx| RemoteRepoMetadataModel::new(ctx));
        // Forward events from both sub-models to a unified event stream.
        ctx.subscribe_to_model(&local, Self::forward_local_event);
        ctx.subscribe_to_model(&remote, Self::forward_remote_event);
        Self { local, remote }
    }
}
```
#### Unified events
```rust
#[derive(Debug)]
pub enum RepoMetadataEvent {
    RepositoryUpdated { id: RepositoryIdentifier },
    RepositoryRemoved { id: RepositoryIdentifier },
    FileTreeUpdated { ids: Vec<RepositoryIdentifier> },
    FileTreeEntryUpdated { id: RepositoryIdentifier },
    UpdatingRepositoryFailed { id: RepositoryIdentifier },
}
```
The wrapper maps sub-model events into the unified enum.
#### Unified query API
Read operations are dispatched to the appropriate sub-model based on the `RepositoryIdentifier` variant:
* `get_repository(&self, id: &RepositoryIdentifier, ctx: &AppContext) -> Option<&FileTreeState>`
* `has_repository(&self, id: &RepositoryIdentifier, ctx: &AppContext) -> bool`
* `repository_state(&self, id: &RepositoryIdentifier, ctx: &AppContext) -> Option<&IndexedRepoState>`
* `get_repo_contents(&self, id: &RepositoryIdentifier, args: GetContentsArgs, ctx: &AppContext) -> Option<Vec<RepoContent<'_>>>`
Note: because the wrapper accesses sub-models through `ModelHandle`, the read APIs require an `AppContext` parameter to dereference the handle. Delegating via `as_ref(ctx)` is simpler than caching and avoids duplication.
#### Local-specific operations
Operations that are inherently local (watcher management, lazy loading, indexing) are exposed directly on the wrapper, which delegates to `LocalRepoMetadataModel` internally via `self.local.update(ctx, ...)`. The sub-model handles are **not** exposed to consumers.
* `index_directory(&self, repository: ModelHandle<Repository>, ctx: &mut ModelContext<Self>) -> Result<(), RepoMetadataError>`
* `index_lazy_loaded_path(&self, path: &Path, ctx: &mut ModelContext<Self>) -> Result<(), RepoMetadataError>`
* `load_directory(&self, repo_root: &Path, dir_path: &Path, ctx: &mut ModelContext<Self>) -> Result<(), RepoMetadataError>`
* `remove_lazy_loaded_path(&self, path: &Path, ctx: &mut ModelContext<Self>)`
* `remove_repository(&self, id: &RepositoryIdentifier, ctx: &mut ModelContext<Self>) -> Result<(), RepoMetadataError>` — dispatches to the correct sub-model based on variant.
* `is_lazy_loaded_path(&self, path: &Path, ctx: &AppContext) -> bool`
* `find_repository_for_path(&self, path: &Path, ctx: &AppContext) -> Option<CanonicalizedPath>`
As remote equivalents are needed (e.g. triggering a remote directory load via the sync layer), they can be added to the wrapper with `RepositoryIdentifier`-based signatures.
#### Encapsulation
The wrapper does **not** expose `.local()` or `.remote()` accessors. All consumers interact exclusively through `RepoMetadataModel`'s public API. This ensures:
1. Consumers are decoupled from the local/remote split — they don't know or care which sub-model handles their request.
2. Adding new environment variants (e.g. containers) doesn't require touching consumers.
3. The wrapper can evolve its internal delegation strategy (e.g. caching, batching) without breaking callers.
### 5. Crate structure
All new types live in the `repo_metadata` crate:
* `lib.rs` — re-exports, `CanonicalizedPath`, `RepositoryIdentifier`, `RemoteRepositoryIdentifier`.
* `model.rs` → renamed to `local_model.rs` (contains `LocalRepoMetadataModel`).
* `remote_model.rs` (new, contains `RemoteRepoMetadataModel`).
* `wrapper_model.rs` (new, contains `RepoMetadataModel`).
* `file_tree_store.rs` — unchanged, shared by both models.
### 6. Consumer migration plan
The migration can be done incrementally. The key invariant is that **existing local-only behavior is preserved** — the wrapper simply adds a remote dimension.
#### Phase 1: Introduce types + wrapper (this spec)
1. Add `RepositoryIdentifier`, `RemoteRepositoryIdentifier`, `RemoteRepoMetadataModel`, and `RepoMetadataModel` to `repo_metadata`.
2. Rename `RepositoryMetadataModel` → `LocalRepoMetadataModel`.
3. Make `RepoMetadataModel` the new singleton; it creates the `LocalRepoMetadataModel` and `RemoteRepoMetadataModel` internally.
4. Update `app/src/lib.rs` to instantiate `RepoMetadataModel` instead of the old singleton.
#### Phase 2: Migrate consumers to wrapper
Consumers construct `RepositoryIdentifier::Local(...)` for their path-based lookups and call all operations through the wrapper's public API. No sub-model handles are accessed directly.
* **`FileTreeView`** — change `ModelHandle<RepositoryMetadataModel>` → `ModelHandle<RepoMetadataModel>`. Subscribe to `RepoMetadataEvent`. For queries, construct `RepositoryIdentifier::Local(canonicalized_path)` and call `wrapper.get_repository(id, ctx)`, `wrapper.has_repository(id, ctx)`, etc. For local-only operations, call `wrapper.index_lazy_loaded_path(path, ctx)`, `wrapper.load_directory(root, dir, ctx)`, etc. directly on the wrapper.
* **`FileSearchModel`** — change `RepositoryMetadataModel::as_ref(app)` → `RepoMetadataModel::as_ref(app)`. Construct `RepositoryIdentifier::Local(...)` for query calls. Event subscription migrates to `RepoMetadataEvent`.
* **`SkillWatcher`** — change `RepositoryMetadataModel::as_ref(ctx)` → `RepoMetadataModel::as_ref(ctx)`. Construct `RepositoryIdentifier::Local(...)` for tree queries. Event subscription migrates.
This phase is purely mechanical and doesn't change behavior — all identifiers are `RepositoryIdentifier::Local(...)` during this phase. A convenience constructor like `RepositoryIdentifier::local(path: impl TryInto<CanonicalizedPath>)` reduces boilerplate at call sites.
#### Phase 3: Wire remote file tree (future, out of scope)
Connect the remote sync layer to `RemoteRepoMetadataModel::insert_repository`. Update `FileTreeView` to display remote repositories using `RepositoryIdentifier::Remote(...)`. This phase requires the remote client model and protobuf sync layer described in the parent tech design.
## Testing Strategy
* Unit tests for `RemoteRepoMetadataModel`: insert/remove/query/event emission.
* Unit tests for `RepoMetadataModel` wrapper: unified query dispatching, event forwarding.
* Existing `RepositoryMetadataModel` (now `LocalRepoMetadataModel`) tests remain unchanged.
* Integration tests in `app/` verify that consumer subscriptions and queries work through the wrapper.
## Decisions
1. **`SessionId` location** — Move `SessionId` to `warp_core` so `repo_metadata` can depend on it directly without circular dependencies.
2. **Event granularity** — The wrapper emits only unified `RepoMetadataEvent`. Consumers subscribe to the wrapper and filter by `RepositoryIdentifier` variant if they only care about local or remote events.
3. **Lifecycle of local-specific operations** — Local-only operations (e.g. `load_directory`) keep their current path-based signatures for now. Remote equivalents will be added to the wrapper once the remote client ↔ server sync layer is in place.
