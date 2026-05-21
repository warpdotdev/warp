# APP-3792 codebase indexing persistence PR tech spec
## Problem statement
This PR makes remote codebase indexing survive daemon restarts and reconnects by restoring daemon-owned codebase index metadata and snapshots at startup, keeping the client synchronized with daemon status snapshots and updates, and exposing remote indexed codebases to the agent context in the same broad shape as local indexed codebases.
The changes are intentionally scoped to the persistence/bootstrap and protocol plumbing needed for APP-3792. They do not redesign the local indexing product flow, move retrieval fully into the daemon, or remove the current remote `ResyncCodebase` protocol path.
## Current state
Local codebase indexing is owned in-process by `CodebaseIndexManager` in `crates/ai/src/index/full_source_code_embedding/manager.rs`. The normal app path constructs it with app-scoped persisted metadata, app-default snapshot storage, a `BulkFilesystemWatcher`, and a client-side `StoreClient`. Local settings can trigger a manual resync through `CodeSettingsPageAction::ManualResync`, which calls `CodebaseIndexManager::try_manual_resync_codebase` directly because the settings UI and index manager live in the same process.
Remote codebase indexing splits those responsibilities across the Warp client and the remote-server daemon. The daemon owns remote filesystem walking, snapshot files, indexing work, and backend sync. The client owns session context, settings/speedbump decisions, agent tool advertisement, and retrieval orchestration. The client and daemon communicate through `crates/remote_server/proto/remote_server.proto`, so operations that are direct method calls locally become client-to-daemon messages remotely.
Before this PR, remote daemon indexing state was too transient: a reconnect or daemon restart did not have a narrow restore path for known remote codebase index metadata and daemon-scoped snapshots. The client also needed a bootstrap status snapshot from the daemon so the active remote repo and agent context could reflect already-indexed remote codebases without waiting for a new indexing run.
## Goals
Restore remote codebase index metadata for the remote-server daemon while keeping a clear startup boundary between full persistence reads and the subset of restored data the daemon is allowed to consume.
Give the daemon an identity-scoped persistence root and snapshot directory so long-lived remote indexing data does not mix with normal app `warp.sqlite` state.
Reuse the existing `CodebaseIndexManager` implementation for both app and daemon paths by injecting snapshot storage rather than forking indexing logic.
Push a full remote codebase index status snapshot after daemon initialize/reconnect, then keep the client current with incremental status updates.
Avoid automatic reindex requests when navigating to a repo that is already ready, stale with a last ready index, queued, or indexing.
Expose ready remote codebases to agent context as stable `(name, path)` entries.
Keep the current `ResyncCodebase` protocol in this PR while documenting why it exists and how it compares with local resync.
## Non-goals
Do not remove or fold `ResyncCodebase` into `IndexCodebase` in this PR.
Do not change local codebase indexing behavior, local persistence schema semantics beyond the shared metadata reuse, or local settings UI behavior.
Do not build a daemon-only indexing implementation separate from `CodebaseIndexManager`.
Do not make the daemon consume or initialize app-only state such as panes, cloud objects, command history, user profiles, MCP servers, or projects.
Do not introduce a client-initiated per-repo status fetch path; daemon-pushed snapshots and deltas remain the synchronization mechanism.
## Proposed design
### Daemon-scoped persistence restore
`persistence::initialize` accepts a `PersistenceScope` so startup can choose between the normal app scope and the remote-server daemon scope. `PersistenceScope::App` uses the normal app database path. `PersistenceScope::RemoteServerDaemon { identity_key }` uses a daemon-specific database path derived from the remote-server identity. Both scopes read the same `PersistedData` shape through the existing SQLite restore helper and both scopes receive writer handles for subsequent updates.
The boundary between app and daemon restore lives at startup initialization rather than in the SQLite reader. `initialize_app` maps the full restored `PersistedData` payload directly into the startup variables used by singleton registration, matching normal app startup. Immediately after that mapping, `initialize_app` applies the launch-mode boundary. Normal app startup consumes the full app restore payload. `LaunchMode::RemoteServerDaemon` preserves `persisted_workspaces` from `codebase_indices` for indexing restore and defaults the app-only startup fields. This keeps a single persistence read path and makes daemon consumption explicit and auditable at initialization.
This split is intentional. The daemon needs the same Diesel/SQLite open, migrate, writer, and full-schema read path as the app so the persistence layer does not fork into app-shaped and daemon-shaped restore contracts. But the daemon should not retain unrelated app-scoped state after initialization. Keeping the direct `PersistedData` startup mapping and filtering in `initialize_app` makes the daemon boundary explicit where startup models are registered while still reusing the existing persistence infrastructure.
The full app restore payload includes state that is meaningful only inside the interactive app process: window/session restoration, cloud object caches, command history, user profiles, workspace language-server settings, MCP server installations, project rules, ignored suggestions, and other UI or app-lifecycle state. Synchronizing all of that into the remote daemon would create two problems. First, the daemon would spend startup time reading and allocating data it will never use. Second, the daemon would become another consumer of app-owned invariants and migrations, so future app persistence changes could accidentally affect a headless remote process.
The daemon only consumes enough persisted data to reconstruct codebase-indexing state:
- repo metadata used to seed `CodebaseIndexManager`,
- the daemon snapshot root used to validate or discard serialized snapshots,
- identity-scoped status/enablement decisions represented by restored codebase metadata,
- writer handles for subsequent codebase-index metadata updates.
That narrow startup consumption is still “syncing app data” in the sense that it reuses the same `codebase_indices` model and full persisted data shape that local app startup uses, but it is not letting the daemon initialize the entire app object graph. This is the intended boundary: share the data model and persistence infrastructure for codebase indexing, then select launch-mode-appropriate fields before registering startup models.
The daemon database and snapshot directories should remain owner-only, matching the remote-server socket/cache privacy model. The database is identity-scoped because enablement/status/backend authorization decisions are Warp-user-specific. Snapshot files are injected separately so the daemon can use its remote-server data root instead of the app default.
### Shared index manager with injected snapshot storage
`CodebaseIndexManager::new` remains the default app constructor. In `local_fs` builds it migrates old app snapshots if needed and passes `SnapshotStorage::app_default()` into `new_with_snapshot_storage`.
`CodebaseIndexManager::new_with_snapshot_storage` is the daemon-compatible seam. It accepts the same persisted `WorkspaceMetadata` and indexing configuration as the app constructor, but lets startup choose the snapshot root. The remote-server daemon passes daemon-scoped storage; the app passes app-default storage.
This keeps indexing behavior shared:
- snapshot validity checks use the same code,
- invalid metadata emits the same `RemoveExpiredIndexMetadata` event,
- valid metadata feeds the same persisted build queue,
- rebuild/resync/drop paths continue to use `CodebaseIndexManager` and `CodebaseIndex`.
The only difference is where metadata and serialized snapshots are restored from and written to.
### Startup and status synchronization
On app startup, `initialize_app` selects a persistence scope from `LaunchMode`. The normal app, CLI, proxy, and tests use `PersistenceScope::App`; `LaunchMode::RemoteServerDaemon { identity_key }` uses `PersistenceScope::RemoteServerDaemon`.
`initialize_app` normalizes the full restored `PersistedData` payload into the startup fields consumed later by singleton registration. It then clears app-only restored fields for daemon launch while preserving `persisted_workspaces` from `codebase_indices`. That lets the existing `CodebaseIndexManager` constructor receive daemon-restored index metadata without pretending the daemon has a full app session restore.
When a client initializes with the daemon, `RemoteServerModel` pushes `CodebaseIndexStatusesSnapshot`. The client-side `RemoteCodebaseIndexModel` applies that snapshot by replacing statuses for the connected host, then applies subsequent `CodebaseIndexStatusUpdated` deltas. This makes reconnect a full-resync boundary and keeps steady-state updates lightweight.
### Navigation and automatic indexing
`RemoteCodebaseIndexModel` records the active repo for a host when it receives `NavigatedToDirectory`. If the navigated directory is a git repo and remote auto-indexing is enabled, it calls `should_request_auto_index_for_navigated_git_repo` before sending an indexing request.
That guard mirrors the local product expectation: navigating into an already-known repo should not immediately trigger another indexing run. It returns false when the current status is ready, stale with a usable last root, queued, or indexing. It returns true when the repo is missing from the status map or has an unusable state such as failed/unavailable/missing root hash.
This preserves automatic indexing for first discovery and recovery while avoiding repeated reindex requests on every `cd` into a repo.
### Agent context and search availability
`RemoteCodebaseIndexModel::codebases_for_agent_context` projects ready searchable remote repos into stable entries with a display name and path. The model only includes statuses that resolve to `RemoteCodebaseSearchAvailability::Ready`, so unindexed, indexing, failed, or otherwise unavailable repos do not appear as searchable codebase context.
For active-session search, `active_repo_availability` resolves an explicit repo path first when it matches known status, otherwise falls back to the active repo for the host or current working directory. Ready availability carries the remote path, root hash, and embedding config needed by downstream search plumbing.
## Protocol shape and local resync contrast
Local resync has two distinct product shapes but no wire protocol. First-time indexing goes through the same manager that owns the local file watcher, persisted metadata, and snapshot storage. Manual resync is an in-process settings action: the settings page dispatches `ManualResync(PathBuf)` and directly calls `CodebaseIndexManager::try_manual_resync_codebase`. Drop/delete similarly calls the manager directly through settings UI actions. Local code can distinguish “index this repo for the first time,” “retry or manually resync this already-indexed repo,” and “drop this repo” by calling different Rust methods because the UI and index manager share memory.
The local pattern also keeps resync conservative. A manual resync only applies when a codebase is already known to the manager. Navigation or repo discovery does not imply a full resync if a ready or stale index already exists; local indexing keeps search available against the last ready root while watcher-driven or manual sync work catches up.
Remote resync crosses the client/daemon boundary. This PR currently models that distinction explicitly with two proto messages:
- `IndexCodebase { repo_path, auth_token }` requests indexing for a repo that may not have an index yet.
- `ResyncCodebase { repo_path, auth_token }` requests a manual full resync of a repo that is already indexed.
That explicit remote protocol mirrors the local product distinction between initial indexing and retry/resync affordances, while making the daemon-side behavior readable in logs, telemetry operation names, and request dispatch. It also lets the daemon return a clear unavailable status when asked to resync a repo it does not know about.
There is a reasonable simplification to consider later: make remote `IndexCodebase` mean “ensure indexed, and if already indexed, perform the manual full resync requested by the client.” That would remove `ResyncCodebase` from the proto and make remote indexing idempotent through one request type. This PR does not implement that simplification so the persistence/status bootstrap work remains isolated from protocol churn.
If a follow-up removes `ResyncCodebase`, it should update all of these seams together:
- proto oneof and message definition,
- `RemoteServerClient::resync_codebase`,
- `RemoteServerOperation::ResyncCodebase`,
- `RemoteCodebaseIndexMutation::Resync`,
- `RemoteServerModel::handle_resync_codebase`,
- client round-trip tests.
The daemon `handle_index_codebase` behavior would then need to explicitly call `try_manual_resync_codebase` for already-indexed repos.
## Mirroring local patterns in remote indexing
Remote indexing should feel like the local feature even though the implementation is split across processes. The strongest local patterns to preserve are:
- A single indexing manager owns per-repo index lifecycle, watcher integration, snapshot validation, sync state, retrieval state, and drop/resync behavior.
- Startup restores persisted codebase metadata first, then queues valid persisted indices through the same manager path as fresh indices.
- Snapshot corruption or incompatibility invalidates the snapshot and falls back to rebuild rather than leaving a repo permanently broken.
- Ready and stale states keep search available through the last ready root; queued and indexing states suppress duplicate indexing requests.
- Settings and speedbump UI decide when to index, retry, resync, or drop; the index manager executes those decisions.
- Manual retry/resync is separate from passive navigation. Navigating into a known repo should update active context, not force a rebuild.
This PR mirrors those patterns by reusing `CodebaseIndexManager`, injecting daemon snapshot storage instead of adding a remote-only manager, feeding daemon-restored metadata into the same persisted build queue, and using `RemoteCodebaseIndexModel` as the client-side analog of local availability state. The remote model records active repo context, applies daemon status snapshots, filters agent context to ready searchable repos, and avoids duplicate auto-index requests when a repo is already ready, stale, queued, or indexing.
The places where remote intentionally differs from local are the process boundary and persistence scope:
- Local can call manager methods directly; remote must encode user actions as proto messages.
- Local can consume full app state because it is the app; the daemon reads the same persisted shape but consumes only codebase-index data because it is a headless worker.
- Local snapshot storage uses the app default; the daemon injects a remote-server data root.
- Local UI state and daemon indexing state synchronize through explicit status snapshots and deltas rather than shared memory.
Future remote improvements should continue to ask “what is the closest local pattern?” before adding remote-specific behavior. Examples:
- If local treats retry as a manual resync of a known repo, remote should either keep a clearly named `ResyncCodebase` message or make `IndexCodebase` explicitly perform that same known-repo resync behavior.
- If local keeps stale search available, remote status pushes should include the last ready root hash for stale states so agent search can continue.
- If local snapshot validation removes invalid persisted metadata, daemon startup should remove invalid daemon metadata and push disabled/failed status deltas rather than silently dropping client state.
- If local settings are the source of truth for user intent, remote daemon handlers should execute explicit client decisions and avoid inventing new daemon-only enablement policy.
## Error handling and security
The daemon should report persistence startup failures through the existing SQLite error reporting and telemetry path, then degrade by starting without restored metadata rather than crashing the app path.
Remote indexing requests that can cause daemon-to-Warp-service calls carry `auth_token` in the protocol payload. The token must never be logged or persisted. The daemon uses it for request-scoped outbound auth and should reject missing or invalid credentials rather than treating the daemon's cached token as ambient authority for proxy-socket writers.
Status values sent to the client should avoid exposing implementation details beyond what the client needs: repo path, lifecycle state, progress, failure message, root hash when search is ready/stale, and embedding config when root hash is present.
## Testing strategy
Unit coverage should focus on the client model and protocol seams:
- snapshot application replaces host-scoped statuses and leaves other hosts untouched,
- incremental status updates update one repo,
- ready/stale statuses with usable root hashes are searchable,
- queued/indexing statuses suppress duplicate automatic indexing,
- failed/unavailable/missing-root statuses allow recovery indexing,
- agent context includes only ready searchable remote repos,
- remote client round-trip tests cover the current indexing/resync/drop request messages.
Persistence coverage should verify that the SQLite reader restores both app state and codebase index metadata through the full `PersistedData` payload. Startup-level coverage should verify that daemon launch consumes only codebase index metadata from that payload and passes daemon-scoped snapshot storage into `CodebaseIndexManager::new_with_snapshot_storage`.
Targeted validation for this PR should include the remote-server client tests, `RemoteCodebaseIndexModel` tests, and a compile check for the app/remote-server crates touched by the persistence and proto changes. Full PR validation should still use the repository presubmit expectations before pushing.
## Risks and mitigations
### Daemon accidentally consumes app state
Consuming full app data in the daemon would couple daemon startup to UI/session state and app-only singleton assumptions. Mitigate by reading the shared `PersistedData` shape, then making the `LaunchMode::RemoteServerDaemon` branch in `initialize_app` preserve only `persisted_workspaces` and default app-only restored fields before model initialization.
### Duplicate indexing on navigation
Remote navigation events can fire often. Without the status guard, entering a known repo could repeatedly enqueue indexing. Mitigate with `should_request_auto_index_for_navigated_git_repo` and tests for ready, stale, queued, indexing, failed, and missing states.
### Client and daemon status drift
If the client misses daemon state during reconnect, tool availability and settings rows can be stale. Mitigate by pushing `CodebaseIndexStatusesSnapshot` after initialize/reconnect and using incremental updates only after that bootstrap.
### Protocol complexity
Keeping both `IndexCodebase` and `ResyncCodebase` makes the remote protocol larger than the minimal idempotent shape. Mitigate by documenting the local-resync parity rationale now and treating removal/folding as a follow-up protocol cleanup rather than mixing it into this persistence PR.
## Definition of done
Remote daemon startup reads the shared persisted data shape but restores only known codebase index metadata into daemon startup state.
The daemon uses daemon-scoped snapshot storage through `CodebaseIndexManager::new_with_snapshot_storage`.
The client receives a bootstrap status snapshot and applies subsequent status deltas.
Remote auto-indexing does not re-request indexing for ready/stale/queued/indexing repos on navigation.
Ready remote repos can appear in agent codebase context.
The PR tech spec documents the persistence architecture and the `IndexCodebase` versus `ResyncCodebase` protocol tradeoff without removing `ResyncCodebase`.
