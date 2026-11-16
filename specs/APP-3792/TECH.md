# APP-3792: Remote Codebase Indexing — TECH.md

Linear: [APP-3792](https://linear.app/warpdotdev/issue/APP-3792)

Behavior is specified in `specs/APP-3792/PRODUCT.md`. This document updates the branch spec against current `origin/master` and the latest design notes: the daemon owns embedding/sync/cache work using its authenticated token, the machine-local serialized Merkle/snapshot cache can be shared when it contains no user-specific data, and the client owns UI state and direct retrieval calls using the daemon-supplied root hash.

## 1. Context
### Current local indexing architecture on master
- `app/src/lib.rs:1825` registers the local `CodebaseIndexManager` singleton.
- `crates/ai/src/index/full_source_code_embedding/manager.rs:167` defines `CodebaseIndexManager`; `manager.rs:186` constructs it from persisted metadata, limits, a `StoreClient`, and a `BulkFilesystemWatcher`.
- `manager.rs:452` handles watcher events, `manager.rs:564` starts indexing a directory, and `manager.rs:850` retrieves relevant files.
- `crates/ai/src/index/full_source_code_embedding/codebase_index.rs:147` defines `CodebaseIndex`, the per-repo owner of the Merkle tree, sync state, snapshot, and retrieval state.
- `crates/ai/src/index/full_source_code_embedding/store_client.rs:15` defines the authenticated backend seam. Its methods are `update_intermediate_nodes`, `generate_embeddings`, `populate_merkle_tree_cache`, `sync_merkle_tree`, `rerank_fragments`, `get_relevant_fragments`, and `codebase_context_config` (`store_client.rs:17-62`).
- `app/src/server/server_api/ai.rs` implements that trait for the client-side `ServerApi`; current master includes the codebase calls around `generate_code_embeddings`, `sync_merkle_tree`, `populate_merkle_tree_cache`, `get_relevant_fragments`, `rerank_fragments`, and `codebase_context_config`.
- `crates/ai/src/index/full_source_code_embedding/snapshot.rs` owns serialized snapshot persistence. The daemon path should reuse the format while changing the base directory.
- `app/src/ai/blocklist/action_model/execute/search_codebase.rs:28` defines `SearchCodebaseExecutor`; the current hydration path uses local file reads after `GetRelevantFilesController`.
- `app/src/ai/agent/api/impl.rs:189-194` explicitly disables `SearchCodebase` for `WarpifiedRemote { host_id: Some(_) }`.
- The existing local UI strings and flows live in `app/src/ai/blocklist/codebase_index_speedbump_banner.rs:20-30` and `app/src/settings_view/code_page.rs:84-98`.

### Current remote-server architecture on master
- `crates/remote_server/proto/remote_server.proto` defines the client/server envelopes. Current messages include `Initialize`, `NavigatedToDirectory`, `ReadFileContext`, and `Authenticate`.
- `app/src/remote_server/server_model.rs:173` stores the daemon-wide `auth_token`; `server_model.rs:514` writes it from `Initialize`, `server_model.rs:532` writes it from `Authenticate`, and `server_model.rs:540` exposes `auth_token()`.
- `app/src/remote_server/server_model.rs:379` dispatches incoming remote-server messages. `server_model.rs:696` handles `NavigatedToDirectory`; `server_model.rs:995` handles `ReadFileContext`.
- `crates/remote_server/src/manager.rs` owns connection setup, initialize, and token rotation from the client side.

### Dependency assumptions
- APP-3801's per-user authenticated daemon model is assumed to land as designed in `specs/APP-3801`: the client sends the current bearer token on `Initialize`, refreshes with `Authenticate`, the daemon stores the credential in memory only, and daemon sockets are partitioned by Warp identity. Remote codebase indexing is the first feature that materially depends on daemon-side upstream calls.
- APP-3790's remote file read path is assumed available for hydrating full file context after retrieval.
- The v1 design assumes daemon-to-`app.warp.dev` egress is available. That was checked with the initial target enterprise environments. If this assumption fails later, the fallback is a client-proxied `StoreClient`, not part of v1.

Daemon responsibilities:
- Check its persisted cache when building the startup snapshot, learning about a repo through navigation, or handling index/drop requests.
- Build the Merkle tree and fragment metadata from the remote filesystem.
- Read remote file bytes for chunking and fragment hydration.
- Run full and incremental sync with the backend through a daemon-side `StoreClient` authenticated by the APP-3801 token.
- Fetch and respect server-backed codebase-indexing config such as embedding config, batch sizes, and sync cadence.
- Persist the serialized Merkle/snapshot cache on the remote host in a machine-local repo cache, while keeping user decisions/status metadata identity-scoped.
- Watch the remote filesystem and push status/root-hash updates to the client.

Client responsibilities:
- Decide whether to offer remote indexing, based on feature flags, user settings, active repo, and remote-server capability.
- Render speedbump/settings/status UI for local and remote repos.
- Cache the latest remote index status per `(remote_identity_key, host_id, repo_path)`, including the current ready root hash and embedding config.
- Expose `SearchCodebase` to the agent only when the active remote repo has a ready index.
- Call the app server directly for retrieval using the current root hash, then call the daemon only to map content hashes back to remote fragment metadata and use the remote file-read path for bytes.

Backend responsibilities:
- Store and retrieve Merkle-tree/index data and embeddings keyed by hashes.
- Authorize every root-hash retrieval against the authenticated Warp user and repo association that created or owns the remote index.
- Answer `get_relevant_fragments(root_hash, query, repo_metadata, embedding_config)`.
- Rerank candidate fragments.
- Provide codebase context config to both local client indexing and daemon-side remote indexing.

### Why the client needs the root hash
The client needs the current ready root hash so retrieval can be client → app server instead of client → daemon → app server. The root hash is the backend lookup key for the synced index; it is enough for retrieval, while avoiding a full tree sync to the client. The client should not need fragment bytes or the complete Merkle tree to decide search candidates.

Root hashes are not treated as standalone bearer capabilities. Backend retrieval must verify that the authenticated caller is allowed to use the root for the associated remote repo before returning candidate fragments.

### Rejected alternative: daemon keeps only tree/bytes, client StoreClient syncs
Alternative shape: the daemon builds or maintains the remote Merkle tree and fragment bytes, while the client's existing `StoreClient` talks to the backend. The daemon sends the tree/root state back to the client, and the client drives backend sync.

Why rejected for v1:
- New repos would require syncing the entire tree and enough fragment data over SSH before backend sync can complete. That adds heavy startup traffic on the least reliable leg of the system.
- APP-3801 exists specifically to let daemon handlers call Warp services with the user's token; not using it here loses the main benefit.
- The only strong argument is resilience when daemon → `app.warp.dev` egress is blocked. The initial customer check says that egress is acceptable, and if it is unavailable, the product should fail visibly rather than silently route a much heavier protocol through SSH.

### Rejected alternative: daemon handles all retrieval
Alternative shape: the daemon receives `SearchCodebase`, calls `get_relevant_fragments`, hydrates fragments, reranks, and returns final locations.

Why rejected for v1:
- Adds an SSH hop to every retrieval query even though the client already has a valid app-server auth path.
- Makes retrieval unavailable when the daemon's backend connection is flaky even if the client can reach the backend.
- Couples agent retrieval latency to the remote link more than necessary.

## 3. Proposed changes
### 3.1 Reuse daemon-compatible indexing code
For v1, wire the daemon path to the existing `crates/ai/src/index/full_source_code_embedding/` implementation instead of creating a new crate up front. The remote-server daemon lives in `app`, and `app` already depends on `ai`, so the simplest implementation can reuse `CodebaseIndexManager`, `CodebaseIndex`, `sync_client`, `store_client`, `snapshot`, `merkle_tree`, `chunker`, `fragment_metadata`, `changed_files`, and their existing tests directly.

The daemon wiring still needs daemon-specific adapters:
- daemon-local SQLite-backed metadata instead of ad-hoc JSON/file metadata,
- daemon-side snapshot base directory,
- remote-compatible filesystem/repo metadata dependencies,
- daemon-compatible `StoreClient` auth plumbing.

Keep daemon entrypoints narrow so the remote-server path depends only on indexing, syntax/chunking, remote filesystem/repo metadata, and backend GraphQL types. Avoid introducing daemon dependencies on unrelated `crates/ai` agent, MCP, terminal, or UI modules. Extracting the indexing implementation into a smaller crate such as `crates/codebase_index` remains a follow-up if v1 shows unacceptable daemon binary size or dependency coupling.

### 3.2 Add daemon-compatible `StoreClient`
`app/src/server/server_api/ai.rs` already implements `StoreClient` for the client-side `ServerApi`; reuse that codebase GraphQL operation and conversion logic. The preferred shape is to make the relevant `ServerApi` request path configurable for whether it is allowed to refresh auth tokens, instead of adding a separate wrapper solely to avoid refresh behavior.

Introduce a small token-refresh policy seam, for example a trait or provider with `allowed_to_refresh_token() -> bool`:
- The normal client `ServerApi` path returns `true`, preserving today's `get_or_refresh_access_token()` behavior and existing `ServerApiEvent::NeedsReauth`/`AccessTokenRefreshed` flow.
- The daemon remote-indexing path returns `false`, uses the request-scoped token from the proto message for request-triggered calls, and uses the in-memory APP-3801 daemon token cache for daemon-initiated background sync. If that token is missing, expired, or rejected, the call returns an unauthenticated/error status instead of trying to refresh through client `AuthState`.

Do not instantiate the full client `ServerApiProvider` inside the daemon unless the constructor can accept the daemon token source and refresh policy without registering client-only UI/auth lifecycle dependencies. `ServerApiProvider` setup currently assumes client app singletons and event handlers such as `AuthManager`, network logging, and auth-token rotation subscriptions. `run_daemon_app` currently registers only headless remote-server, repo metadata, filesystem, and telemetry no-op models, so pulling in the full provider unchanged would add client UI/auth lifecycle coupling to the daemon.

Once the token source and `allowed_to_refresh_token` policy are injectable, the daemon can reuse the same `ServerApi` implementation directly for codebase-indexing backend calls, with refresh disabled. Until then, share the GraphQL operation construction, result conversion, error mapping, and `http_client::Client` usage; do not fork the GraphQL operations.

Required behavior:
- Reads the request-scoped token supplied by the remote client/server proto message for operations triggered by that message. The daemon may keep `ServerModel::auth_token()` or an injected token provider as the initialized token cache for daemon-initiated background sync, but request-triggered auth-required outbound Warp service requests must not be authorized solely by the cached daemon token.
- Disables token refresh for daemon calls by using `allowed_to_refresh_token() == false`. The daemon path must surface missing/expired/revoked credentials to the client instead of invoking the client's token refresh path.
- Sends the same backend operations the local client sends today: config fetch, Merkle tree sync, embedding generation, intermediate-node update, cache population, relevant-fragment retrieval only if a future daemon-retrieval path needs it, and reranking only if a future daemon-retrieval path needs it.
- Classifies errors into at least unauthenticated, backend unreachable, backend rejected, and internal/unknown so status UI can distinguish actionable failures.
- Redacts tokens from logs and never persists them.

For v1 sync, daemon-side retrieval methods may still be implemented because the trait requires them, but the normal remote retrieval path should use the client's `ServerApi` for `get_relevant_fragments` and `rerank_fragments`.

### 3.3 Add daemon-side index cache and startup bootstrap
The daemon keeps two persistence layers under the remote-server cache root:

- Shared machine-local snapshot files, keyed by repo identity/path and content, containing serialized Merkle trees, fragment metadata, snapshots, and other data that is derived only from files readable by the OS user running the daemon. These snapshot files intentionally contain no Warp-user-specific choices, credentials, or authorization state and can be reused by multiple Warp identities that connect to the same OS account and repo.
- Daemon-local SQLite metadata, using the existing `persistence`/Diesel infrastructure from the app/oz binary rather than ad-hoc JSON. Add remote-indexing migrations for shared cache records and identity-scoped user state. The remote daemon should initialize the SQLite persistence subsystem in `run_daemon_app` or an equivalent daemon bootstrap path before constructing the indexing manager.

Example layout:
- `~/.warp/remote-server/codebase-indexes/shared/snapshots/{repo_key}/...`
- SQLite database under the daemon's state directory, with tables such as `remote_codebase_index_cache` and `remote_codebase_index_user_state`.

Sharing the serialized Merkle/snapshot cache is acceptable because it is just a representation of the local codebase for the remote OS account. Sharing user metadata is not acceptable: enablement/decline/drop choices, status, and backend root authorization remain scoped per Warp identity. Backend storage may also deduplicate content-addressed Merkle nodes, fragments, or embeddings internally, but retrieval authorization must bind usable roots to the authenticated Warp user and repo.

Shared cache metadata in SQLite should record at least repo path, repo identity key, snapshot/schema version, snapshot file key/path, root hash, embedding config, last indexed time, and enough timestamps to rebuild the local `WorkspaceMetadata` inputs that currently populate the local build queue. Identity-scoped SQLite metadata should record at least `identity_key`, repo path, enabled/disabled/declined state, current status, last user-visible error, last status update, backend association state, and the last ready root hash associated with that Warp identity.

Daemon SQLite wiring:
- Do not call the full `persistence::initialize(ctx)` path from `run_daemon_app` unchanged. That initializer is app/CLI-shaped: it reads full app state, expects `AuthStateProvider`, creates the general `PersistenceWriter`, and restores UI/session/cloud-object data the daemon does not need.
- Instead, factor the reusable SQLite pieces behind a daemon-scoped initializer, for example `persistence::initialize_remote_codebase_indexing(ctx)` or a lower-level `sqlite::initialize_with_scope(scope, path)`. It should reuse the existing Diesel migrations, schema generation, `establish_connection` pragmas, error reporting pattern, and writer-thread/event pattern, but only read/write remote-codebase-indexing tables.
- Store the daemon codebase-indexing database under the remote-server cache root, separate from the normal app/Oz `warp.sqlite`, for example `~/.warp/remote-server/codebase-indexes/index.sqlite`. Keeping it remote-server-scoped avoids mixing long-lived daemon cache rows with a user's normal app/CLI session-restore database while still reusing the same SQLite infrastructure.
- Create the parent directory, shared snapshot files, and SQLite file with owner-only access, matching the remote-server socket/cache privacy model. The shared snapshot files and shared metadata tables may be machine-local for the remote OS account; identity decisions still remain keyed by `identity_key`.
- Add migrations under `crates/persistence/migrations/` for remote indexing tables and regenerate `persistence::schema`/`persistence::model` in the normal way. Tables should live in the shared schema so app/CLI and daemon code can use the same typed Diesel models, but daemon reads should be limited to the remote-indexing tables.
- Add daemon-specific `ModelEvent` variants or a separate daemon persistence event enum for `UpsertRemoteCodebaseIndexCache`, `UpsertRemoteCodebaseIndexUserState`, `DeleteRemoteCodebaseIndexUserState`, and `DeleteRemoteCodebaseIndexCache`. Prefer a separate enum if adding these events to the app-wide `ModelEvent` would make the general writer handle daemon-only concepts.
- Register a daemon persistence writer singleton in `run_daemon_app` before constructing the remote indexing manager. Pass its sender/handle into the daemon indexing manager so manager events can persist status/root changes without blocking the remote-server message handler.
- On startup, the daemon initializer should synchronously read only the remote-indexing rows needed to build initial shared cache metadata and identity-scoped user state. Those values feed the daemon indexing manager before it accepts `IndexCodebase` or status requests.
- On shutdown, rely on the `PersistenceWriter`-style drop/terminate behavior so the SQLite writer thread drains or terminates cleanly when the daemon exits after its grace period.

Suggested implementation sequence:
1. Extract SQLite open/migrate/start-writer helpers so they can accept an explicit database path and a narrowed read function.
2. Add remote-indexing Diesel models and writer events.
3. Add `remote_server::run_daemon_app` bootstrap that initializes the daemon-scoped SQLite database and registers the writer singleton.
4. Construct the daemon indexing manager from the synchronously read remote-indexing rows plus the writer sender.
5. Wire indexing manager status/cache events to the writer and verify reconnect/status responses read from the in-memory state populated from SQLite.

Startup/reconnect behavior:
1. Load shared cache metadata/snapshots and identity-scoped user metadata before accepting indexing requests.
2. Build an identity-scoped status snapshot containing every repo the daemon knows about for that identity.
3. For repos enabled by the connected identity with a valid shared ready snapshot, include `Ready` status with that identity's authorized root hash.
4. For repos with a valid shared snapshot but no enablement record for the connected identity, include `Not enabled` so the user still controls whether that repo is searchable for them.
5. For known enabled repos without a valid shared snapshot, include `Failed` or queue rebuild depending on whether recovery can start immediately.
6. Push the full status snapshot to connected clients after daemon initialization and after reconnect, before relying on incremental status updates.
7. After the snapshot is applied, keep the client and daemon synchronized with `CodebaseIndexStatusUpdated` deltas for every status/root change and every newly known repo. When the daemon learns about a git repo through navigation or repo detection and that repo is not already in the synchronized set, it should immediately push an explicit status such as `Not enabled`, `Ready`, `Failed`, or `Unavailable`.

Snapshot parsing should follow local behavior: if a snapshot is incompatible or corrupt, delete it and rebuild from scratch rather than leaving the repo permanently failed.

Cache invalidation behavior:
1. Snapshot schema/version mismatch, corrupt snapshot data, or missing snapshot files invalidate the shared local snapshot and trigger a rebuild the next time any identity indexes the repo.
2. If the repo path no longer exists or is no longer a git repo, return a failed or not-enabled status with a user-readable reason rather than reusing stale root hashes indefinitely.
3. Filesystem watcher changes mark the repo stale for all identities that have enabled it when a last-ready root hash exists, keep search available against each identity's last authorized root, and run incremental sync toward a new ready root.
4. Backend config or embedding-config changes mark affected shared snapshots stale and re-run the necessary embedding/sync work with the new config.
5. Auth identity changes clear the client-side `RemoteCodebaseIndexModel` cache and reconnect through the identity-scoped daemon path, but they do not delete the shared machine-local index cache.
6. If the backend rejects, cannot find, or no longer authorizes a previously ready root hash for a specific identity, mark that identity's repo status failed with an actionable reason and require `IndexCodebase` to rebuild, resync, or re-associate the shared cache for that identity.

### 3.4 Add remote-server protocol messages
Extend `crates/remote_server/proto/remote_server.proto` with request/response and push messages for remote indexing. Names can be adjusted during implementation, but the protocol needs these concepts:

- `IndexCodebase { repo_path }`
- `DropCodebaseIndex { repo_path }`
- `GetFragmentMetadataFromHash { repo_path, content_hashes }`
- `CodebaseIndexStatusesSnapshot { statuses }`
- `CodebaseIndexStatusUpdated { repo_path, status }`

`IndexCodebase` is the only client-triggered indexing command in v1. The client owns all product decisions about feature flags, speedbump acceptance, automatic indexing settings, and retry affordances before it sends this message. The daemon treats the message as an explicit request to index, retry, or rebuild the repo path.

`CodebaseIndexStatusesSnapshot` is the status bootstrap and full-resync path. After daemon initialization and after a client reconnects, the daemon must push the complete set of identity-scoped repo statuses it loaded from SQLite. The client uses this snapshot to populate settings and initial tool-advertisement state without asking for every repo one-by-one.

There is intentionally no per-repo status fetch or client-initiated bulk status fetch in v1. The daemon and client should always converge through the pushed `CodebaseIndexStatusesSnapshot` after initialize/reconnect plus `CodebaseIndexStatusUpdated` deltas. When a user navigates to a repo that is not already in the synchronized status set, the daemon should push `CodebaseIndexStatusUpdated` for that repo as soon as it recognizes the repo, usually `Not enabled` for a first-seen repo. The client should not ask the daemon for just that repo.

`GetFragmentMetadataFromHash` is used after client-side backend retrieval. The backend returns content hashes for candidate fragments, but only the daemon has the remote snapshot metadata needed to map those hashes back to remote file paths, ranges, symbols, and other fragment metadata. The daemon must verify every requested content hash belongs to the enabled repo's current or last-ready snapshot before returning metadata. Content bytes should be read through the APP-3790 remote `ReadFileContext` path rather than this RPC.

All new remote-indexing RPCs are scoped to the identity-partitioned remote-server daemon socket. Authorization requirements by message:
- `DropCodebaseIndex` mutates only identity-scoped user metadata for the connected identity and must carry a request-scoped bearer credential, either in the proto payload or authenticated request envelope, before it calls the backend to revoke or delete that user's repo/root association.
- `GetFragmentMetadataFromHash` requires that the connected identity has enabled the repo and that every requested content hash belongs to that repo's current or last-ready snapshot. It must not read cross-repo metadata from the shared cache.
- `IndexCodebase` must carry a request-scoped bearer credential, either in the proto payload or authenticated request envelope, because it can trigger config fetches, embedding generation, and index sync.

Any request message that can lead to auth-required outbound Warp service calls must carry the current client auth token or an equivalent request-scoped bearer credential. Handlers must reject missing or invalid request-scoped tokens instead of falling back to the daemon's stored `auth_token`; the stored token is only a cache/initialization aid and must not make the proxy socket an ambient-authority boundary. If future versions let `GetFragmentMetadataFromHash` or daemon-side retrieval call Warp services, those messages must also carry the token before those outbound calls are added.

`IndexStatus` should include:
- `state`: not enabled, queued, indexing, ready, stale, failed, disabled, unavailable.
- `progress`: optional current phase and counts.
- `failure_reason`: optional user-readable string plus machine-readable category.
- `root_hash`: present for ready and stale states when a last-ready index exists.
- `embedding_config`: present whenever `root_hash` is present.
- `last_updated_at`: useful for settings and debugging.

The client should receive root hashes only through status responses/pushes. It should never receive the whole Merkle tree.

### 3.5 Add daemon indexing manager wiring
In `app/src/remote_server/mod.rs`, register the indexing manager as a daemon singleton with:
- SQLite-backed shared cache metadata,
- SQLite-backed identity-scoped user metadata,
- daemon-side shared snapshot base directory,
- daemon-side `StoreClient`,
- `BulkFilesystemWatcher`,
- remote-compatible repo metadata / detected-repo dependencies already used by the daemon.

In `app/src/remote_server/server_model.rs`, add handler arms for the new RPCs:
- `IndexCodebase`: check cache first; if miss, failed, stale, or invalid, enqueue/build index and immediately push queued/indexing status. Retrying a failed repo is the same message after the client chooses retry.
- `DropCodebaseIndex`: remove or update the connected identity's user metadata for that repo, stop watcher registration if no enabled identities still need it, push disabled/not-enabled status, and call the backend to revoke or delete that user/repo/root association for synced remote index data. The shared machine-local Merkle/snapshot cache may remain for other identities or future reuse, and content-addressed backend blobs may remain subject to backend retention or deduplication policy, but dropped roots must become inaccessible for retrieval by that user/repo.
- `GetFragmentMetadataFromHash`: verify each content hash belongs to the enabled repo's current or last-ready snapshot, map hashes to fragment metadata, and return remote file paths/ranges plus metadata needed by retrieval. Do not read file bytes or make backend calls in this handler.
Update the existing `NavigatedToDirectory` handling so that when the daemon recognizes a git repo that is not in the current identity-scoped status set, it computes the repo's cached status and pushes `CodebaseIndexStatusUpdated` immediately. First-seen repos should become explicit `Not enabled` entries rather than remaining absent from client state.
Subscribe once to indexing manager events and fan out `CodebaseIndexStatusUpdated` deltas to connected clients after the initial snapshot. On disconnect/reconnect, push `CodebaseIndexStatusesSnapshot` again; push messages are the primary steady-state path, and reconnect is the full-resync boundary.

### 3.6 Fetch and respect server-backed config on the daemon
The daemon should call `codebase_context_config` through its `StoreClient` before sync work and at the cadence expected by the local implementation. Server-backed values such as embedding config, embedding cadence, generation batch size, and sync batching should be owned by the backend and respected on the remote host.

The client should evaluate user/client-controlled gates before sending `IndexCodebase`, such as whether the remote-indexing feature flag is enabled, whether the user accepted indexing, and whether persistence is allowed. Do not send client-owned feature or preference values for the daemon to reinterpret, and do not use client-sent values for server-owned tuning knobs when the daemon can fetch them directly.

### 3.7 Client-side state and UI model
Add a client singleton such as `RemoteCodebaseIndexModel` that subscribes to `RemoteServerManager` events and tracks:
- `(remote_identity_key, host_id, repo_path) -> RemoteIndexState`
- remote server capability per `host_id`, including unsupported old-daemon builds and disconnected/unavailable hosts
- the last known active repo per remote session/host so speedbump and agent-tool code can ask about the current remote repo without re-deriving it

`RemoteIndexState` should carry:
- `lifecycle`: not enabled, queued, indexing, ready, stale, failed, disabled, unavailable, or unsupported.
- `progress`: optional phase/counts for queued/indexing/stale.
- `failure_reason`: optional user-readable string plus machine-readable category for failed/unavailable/unsupported states.
- `root_hash`: present only for ready/stale states with a last usable index.
- `embedding_config`: present whenever `root_hash` is present.
- `last_updated_at`: daemon-supplied or client-observed timestamp for settings/debugging.
- `source`: whether the value came from daemon startup bootstrap, direct status response, push update, or local disconnect/capability handling.

Public APIs should cover the upstream callers explicitly:
- `state_for_repo(remote_identity_key, host_id, repo_path) -> Option<RemoteIndexState>` for settings rows and low-level callers.
- `state_for_active_remote_repo(session_id) -> Option<RemoteIndexState>` for speedbump and agent-tool advertisement.
- `entries_for_settings() -> Vec<RemoteIndexSettingsEntry>` returning stable display rows with host label, repo path, lifecycle, progress/failure, and supported actions.
- `can_search(session_id, repo_path) -> RemoteSearchAvailability`, returning ready/stale plus root hash and embedding config, or a typed unavailable reason for agent/tool plumbing.
- `request_index(session_id, repo_path, auth_token)` to send `IndexCodebase` after client-side feature/preference/speedbump decisions.
- `drop_index(session_id, repo_path, auth_token)` to send `DropCodebaseIndex` and optimistically move the entry to disabled/not-enabled only after daemon acknowledgement.
- `apply_status_snapshot(host_id, statuses)` to replace/reconcile the initial daemon-provided status set for settings and tool-advertisement bootstrap.

Event handling:
- On `RemoteServerManagerEvent::SessionConnected`/`SessionReconnected`, record host capability, enter an awaiting-snapshot state, and clear any local unavailable marker only after the daemon's `CodebaseIndexStatusesSnapshot` arrives. If the snapshot does not arrive within the expected protocol window, treat the session as out of sync and reconnect or mark the host unavailable/unsupported rather than issuing a separate status request.
- On `CodebaseIndexStatusesSnapshot`, replace or reconcile all identity-scoped entries for that host and notify settings/speedbump/tool subscribers.
- On `NavigatedToDirectory`, update the session's active repo and wait for/apply the daemon-pushed `CodebaseIndexStatusUpdated` if this is a newly known repo. The speedbump or auto-indexing flow should act on the explicit status, such as `Not enabled`, rather than inferring a missing state locally. Do not issue a per-repo status request on navigation.
- On `CodebaseIndexStatusUpdated`, upsert the keyed `RemoteIndexState`, notify settings/speedbump/tool subscribers, and preserve a ready root hash when the daemon reports stale with a last-ready root.
- On `SessionDisconnected` or `HostDisconnected`, mark affected entries unavailable without deleting their last ready/stale root hash. Search should not be advertised while unavailable, but settings should still show the last known status and host disconnect reason.
- On identity changes/logout, clear the client cache and rely on the identity-scoped daemon socket/status bootstrap after reconnect; do not reuse root hashes across identities.
- On unsupported old daemon/protocol errors, store `unsupported` per host so UI does not keep offering the speedbump for that session.

Model invariants:
- Never persist auth tokens, request-scoped credentials, or fragment bytes in the client model.
- Do not expose `SearchCodebase` unless `can_search` returns ready/stale with a root hash, embedding config, connected host, and matching active repo.
- Keep local and remote indexing state separate. Local `CodebaseIndexManager` remains the source of truth for local repos; `RemoteCodebaseIndexModel` only owns remote host/repo state.
- Avoid wildcard host-only keys: every cached remote repo entry must include `remote_identity_key`, `host_id`, and repo path so same-host or same-path collisions do not leak status across users or identities.

Use this model from:
- `app/src/settings_view/code_page.rs` to render remote entries alongside local entries with a remote tag/host label and the states from PRODUCT §8-14.
- `app/src/ai/blocklist/codebase_index_speedbump_banner.rs` to show the remote-aware speedbump and dispatch `IndexCodebase`.
- agent/tool plumbing to decide whether `SearchCodebase` is advertised for remote sessions.

Settings should distinguish local auto-indexing from remote auto-indexing. If implementation chooses to reuse one preference, the product spec must be updated before shipping; the current product expectation is independent control.

### 3.8 Remote retrieval path
When `SearchCodebaseExecutor` runs in `SessionType::WarpifiedRemote { host_id: Some(_) }`:
1. Resolve the active remote repo path.
2. Read `RemoteCodebaseIndexModel` for `(remote_identity_key, host_id, repo_path)`.
3. If the state is not ready/stale with a root hash, return a typed `SearchCodebaseResult::Failed` reason for indexing-in-progress, failed, disabled, unavailable, or not indexed.
4. Use the client's `ServerApi` to call `get_relevant_fragments(root_hash, query, repo_metadata, embedding_config)`.
5. Call `GetFragmentMetadataFromHash` on the daemon with the returned content hashes.
6. Use the APP-3790 remote `ReadFileContext` path to read the fragment ranges from the returned metadata.
7. Use the client's `ServerApi` to call `rerank_fragments(query, hydrated_fragments)`.
8. Convert reranked fragments into `CodeContextLocation`s and hydrate any remaining full file context through the APP-3790 remote `ReadFileContext` path.
9. Return the normal `SearchCodebaseResult::Success { files }`.

The local path remains unchanged.

### 3.9 Feature flag and rollout
Add `FeatureFlag::RemoteCodebaseIndexing` and gate only client-visible behavior:
- speedbump offer,
- settings controls,
- remote tool advertisement,
- remote dispatch branch in `SearchCodebaseExecutor`.

The daemon should not independently check the feature flag. If it receives a valid `IndexCodebase` request from an authenticated client build, it should perform the requested work. This avoids requiring daemon/client flag state to be perfectly synchronized.

## 4. End-to-end flows
### New repo
1. Client observes remote navigation into repo, and the daemon receives the existing navigation signal.
2. Daemon recognizes the git repo, checks identity-scoped metadata/shared cache, and pushes `CodebaseIndexStatusUpdated { repo_path, status: Not enabled }` if this is a first-seen repo for the connected identity.
3. Client applies the explicit `Not enabled` state in `RemoteCodebaseIndexModel`.
4. Client offers speedbump or auto-enables based on settings.
5. Client sends `IndexCodebase`.
6. Daemon builds the tree on the remote host.
7. Daemon fetches backend config and syncs missing tree nodes/fragments/embeddings using daemon auth.
8. Daemon saves metadata/snapshot and pushes `Ready { root_hash, embedding_config }`.
9. Client caches status and enables `SearchCodebase`.

### Previously seen repo
1. Daemon loads metadata and snapshots during startup.
2. Daemon pushes `CodebaseIndexStatusesSnapshot` containing the ready cached repo.
3. Client caches `Ready { root_hash, embedding_config }`.
4. Client enables retrieval without full rebuild when the user navigates to that repo.

### Startup with known repos
1. Daemon loads metadata and snapshots during startup.
2. Daemon builds the full identity-scoped status set for known remote repos.
3. Daemon pushes `CodebaseIndexStatusesSnapshot { statuses }` to connected clients.
4. Client populates remote settings entries from the snapshot.
5. Watcher registration resumes for enabled repos.

### Incremental changes
1. Daemon filesystem watcher fires for a watched repo.
2. Daemon marks the repo stale if a previous root hash exists.
3. Daemon refreshes backend config if due, computes the incremental tree diff, asks backend what is missing, and syncs only missing nodes/fragments/embeddings.
4. Daemon authorizes background sync with the in-memory APP-3801 token cache for the same connected identity that enabled the repo. If the token is missing, expired, revoked, or the identity has no authenticated client connection allowed to refresh it, the daemon pauses sync, keeps the last ready root usable as stale, and pushes a failed or unavailable status that asks the client to reauthenticate/retry.
5. Daemon saves the new snapshot/root hash and pushes ready status to the client.
6. Client replaces its cached root hash; new retrievals use the new index.

### Retrieval
1. Client already knows the current ready root hash.
2. Client calls app server for candidate fragment hashes.
3. Client asks daemon to map those hashes into fragment metadata and remote file ranges.
4. Client reads the fragment ranges from the remote host and calls app server for reranking.
5. Client hydrates full file context from the remote host and returns the standard result shape to the agent.

## 5. Incremental PR plan
Break the implementation into small PRs that keep behavior behind `FeatureFlag::RemoteCodebaseIndexing` until the end-to-end path is ready.

### PR 1: Basic daemon/client handshake
Add the remote-server protocol capability and no-op status synchronization path first. The daemon should advertise remote-indexing support, push an empty or SQLite-backed `CodebaseIndexStatusesSnapshot` after initialization/reconnect, and push `CodebaseIndexStatusUpdated { status: Not enabled }` when navigation reveals a first-seen git repo. The client should add `RemoteCodebaseIndexModel` enough to apply snapshots and pushed repo-status updates, track unsupported/unavailable hosts, and prove settings/tool callers can observe the synchronized status set without exposing `SearchCodebase` yet.

### PR 2: ServerApi token-refresh policy
Make `ServerApi` configurable with an injectable token source and `allowed_to_refresh_token` policy. Keep the existing client path on refresh-enabled behavior, add refresh-disabled daemon tests, and verify missing/expired/revoked daemon credentials return actionable auth errors instead of entering the client refresh flow.

### PR 3: Daemon SQLite persistence bootstrap
Add remote-indexing SQLite migrations/models/writer events and daemon-scoped persistence initialization under the remote-server cache root. Load shared cache metadata and identity-scoped user state before the daemon sends its snapshot, and persist status/root changes from daemon events.

### PR 4: IndexCodebase daemon indexing path
Wire `IndexCodebase` to the reused codebase-indexing manager, daemon snapshot directory, filesystem watcher, server-backed config fetch, embedding/sync calls, and pushed `CodebaseIndexStatusUpdated` transitions. Keep retrieval disabled until a ready root and embedding config are reliably synchronized to the client.

### PR 5: Remote retrieval path
Add `GetFragmentMetadataFromHash`, connect `SearchCodebaseExecutor` for remote sessions, call client-side `get_relevant_fragments`, map hashes through the daemon, read bytes via APP-3790 `ReadFileContext`, rerank with client `ServerApi`, and return the standard `SearchCodebaseResult` shape.

### PR 6: Settings, speedbump, and rollout polish
Expose remote entries in settings, add the remote-aware speedbump/auto-indexing controls, add manual validation for open-egress and blocked-egress hosts, and keep the feature flag off until local non-regression and remote end-to-end tests pass.

## 6. Testing and validation
- Keep existing codebase-index unit tests running against the reused indexing implementation. This covers PRODUCT §21-24 and local non-regression in §34-36.
- Add daemon-side `StoreClient` tests for token-present, missing-token, backend-unreachable, backend-rejected, and config-fetch behavior. This covers PRODUCT §25 and §31-33.
- Add remote-server protocol/handler tests for index, status, drop, fragment-metadata lookup, retry-via-index, pushed status transitions, and rejection of auth-required requests that omit the request-scoped token. This covers PRODUCT §8-14, §21-24, and §29.
- Add client `RemoteCodebaseIndexModel` tests for queued → indexing → ready, ready → stale → ready, failed → retry → ready, disabled, and unavailable transitions. This covers PRODUCT §10-14 and §17-19.
- Add `SearchCodebaseExecutor` tests for remote ready, indexing, failed, unavailable, not indexed, and local fallback paths. Verify the remote ready path calls client `get_relevant_fragments`, daemon `GetFragmentMetadataFromHash`, remote `ReadFileContext`, client rerank, then final remote `ReadFileContext` in order. This covers PRODUCT §15-20 and §29.
- Add settings/speedbump UI tests or snapshots for local entries, remote entries, remote tag/host labeling, retry, drop, and independent local/remote automatic indexing settings. This covers PRODUCT §4-14 and §34.
- Add per-user isolation tests proving shared machine-local snapshots do not share enablement/status/backend authorization, and that daemon auth token usage remains identity-scoped. This covers PRODUCT §26-28.
- Add manual verification on an open-egress remote host: enable indexing for a new repo, observe progress in settings, run `SearchCodebase`, edit a file, observe stale/ready transition, and verify subsequent retrieval uses the updated repo.
- Add manual verification on a blocked-egress remote host: enable indexing, verify failed status and retry behavior, and verify other remote tools remain usable.

## 7. Risks and mitigations
- **Daemon egress blocked.** Mitigation: product shows failed/unreachable with retry. Keep client-proxied `StoreClient` as a follow-up only if real deployments require it.
- **Binary size increase.** Reusing indexing code brings tree-sitter/chunking/GraphQL dependencies into the daemon. Mitigation: measure daemon binary size before landing, keep daemon entrypoints narrow, and extract a smaller indexing crate later only if needed.
- **Config drift between local and remote indexing.** Mitigation: daemon fetches server-backed config via `codebase_context_config`; client owns user/feature gate decisions before sending `IndexCodebase`.
- **Status push loss during disconnect.** Mitigation: identity-scoped status is cached on the daemon; reconnect pushes a fresh `CodebaseIndexStatusesSnapshot`, and clients treat reconnect as the full-resync boundary before trusting incremental deltas.
- **Snapshot corruption/version skew.** Mitigation: match local snapshot behavior by deleting bad snapshots and rebuilding.
- **Credential exposure.** Mitigation: use APP-3801 token provider, never persist tokens, redact protocol logs, and ensure agent context never includes auth material.
- **Proxy-socket auth bypass.** Mitigation: require request-scoped auth tokens on remote client/server proto messages before handlers make auth-required outbound Warp service requests; reject missing or invalid tokens instead of relying on daemon-stored credentials as ambient authority.
- **Root hash staleness.** Mitigation: stale state keeps last ready root hash usable until a new ready hash arrives; failed sync does not overwrite the last ready hash.

## 8. Follow-ups
- Client-proxied `StoreClient` fallback for hosts that cannot reach `app.warp.dev`.
- Garbage collection for shared machine-local snapshots when no identity metadata references them.
- Daemon-direct telemetry for indexing metrics instead of client-forwarded status-only events.
- Cross-repo remote context across multiple repos on one host.
- Retrieval caching for repeated queries within one remote session.
