# TECH.md — Remote Server Manager, Host ID Subcommand, and Session Connection Flow

Linear: [APP-3787](https://linear.app/warpdotdev/issue/APP-3787)

## 1. Problem

The Warp client needs a centralized way to manage connections to `remote_server` processes running on remote hosts. Today, each downstream feature (file tree, code review, agent apply diff) would need to independently figure out how to reach the remote server for its session's host. Each SSH session needs its own dedicated connection to the remote server because SSH connections are tied to the parent session's lifecycle — if the parent session dies, all multiplexed connections through it die too. Deduplication to a single long-lived server process happens on the remote host, not on the client.

This spec covers three pieces:
1. A protocol change — the `InitializeResponse` returns a `HostId` so the client can deduplicate per-host models
2. A `RemoteServerManager` singleton model — the global registry that maps sessions to `RemoteServerClient` instances
3. The session connection flow — the manager's internal `connect_session` workflow (server startup, initialization, host identification via the protocol). The specifics of *when and where* `connect_session` is triggered are out of scope and will be addressed in a future spec alongside the binary installation flow.

## 2. Relevant Code

### Remote server crate
- `crates/remote_server/src/client.rs` — `RemoteServerClient` struct with background reader/writer tasks, `initialize()` handshake
- `crates/remote_server/src/server_model.rs` — `ServerModel` singleton on the server side, handles `ClientMessage` dispatch
- `crates/remote_server/src/protocol.rs` — length-delimited protobuf read/write helpers, `ProtocolError`, `RequestId`
- `crates/remote_server/proto/remote_server.proto` — `ClientMessage`/`ServerMessage` envelopes with `Initialize`/`InitializeResponse` (to be extended with `host_id`)

### CLI subcommand dispatch
- `crates/warp_cli/src/lib.rs (384-426)` — `WorkerCommand` enum with `RemoteServer` variant
- `app/src/lib.rs (542-544)` — `WorkerCommand::RemoteServer` dispatch calling `remote_server::run()`

### Session bootstrap flow
- `app/src/terminal/model/terminal_model.rs (2874-2919)` — `init_shell()` handler creates `SessionInfo::create_pending()` with `SessionType`
- `app/src/terminal/model/terminal_model.rs (2820-2858)` — `bootstrapped()` handler merges pending session info and emits `HandlerEvent::Bootstrapped`
- `app/src/terminal/model_events.rs (89-111)` — `ModelEventDispatcher` receives `Bootstrapped`, calls `sessions.initialize_bootstrapped_session()`
- `app/src/terminal/model/session.rs (199-309)` — `Sessions::initialize_bootstrapped_session()` creates `Session`, emits `SessionsEvent::SessionBootstrapped`
- `app/src/terminal/view.rs (11022-11199)` — `TerminalView::handle_session_bootstrapped()` reacts to the event

### Session and SSH types
- `app/src/terminal/model/session.rs (691-699)` — `SessionType::Local` / `SessionType::WarpifiedRemote`
- `app/src/terminal/model/session.rs (426-451)` — `SessionInfo` struct with `hostname`, `user`, `session_type`, `spawning_session_id`
- `app/src/terminal/model/terminal_model.rs (632-647)` — `SubshellInitializationInfo` with `ssh_connection_info: Option<InteractiveSshCommand>`
- `app/src/terminal/ssh/util.rs (86-89)` — `InteractiveSshCommand { host, port }`

### Remote command execution over SSH
- `app/src/terminal/model/session/command_executor/remote_command_executor.rs` — `RemoteCommandExecutor` uses SSH `ControlPath` to run one-off commands over an existing SSH connection
- `app/src/terminal/model/session.rs (388-391)` — `IsLegacySSHSession::Yes { socket_path }` stores the SSH control socket

### Existing remote host patterns
- `app/src/terminal/view.rs (6094-6105)` — `active_session_remote_host()` returns `Some("user@hostname")` for remote sessions
- `app/src/terminal/view.rs (9362-9436)` — `is_block_considered_remote()` checks `session.is_local()`
- `app/src/terminal/cli_agent_sessions/mod.rs (107-111)` — `CLIAgentSession` stores `remote_host: Option<String>` per session

### Singleton model precedents
- `app/src/terminal/cli_agent_sessions/mod.rs (234-462)` — `CLIAgentSessionsModel` singleton with `HashMap<EntityId, CLIAgentSession>`, event emission, session lifecycle
- `app/src/ai/mcp/templatable_manager.rs (41-78)` — `TemplatableMCPServerManager` singleton with per-server state tracking, spawn/abort handles

## 3. Current State

- The `remote_server` crate has a working `RemoteServerClient` and `ServerModel` with `Initialize`/`InitializeResponse` over length-delimited protobuf.
- The `warp remote-server` subcommand boots the headless app and runs the server over stdin/stdout.
- There is no client-side manager. No code exists to spawn the remote server over SSH, track which hosts have running servers, or route feature requests to the right client.
- Sessions know their `hostname` and `session_type` after bootstrap, and SSH sessions have `ssh_connection_info` from the parsed SSH command.
- The `RemoteCommandExecutor` demonstrates how to run commands over an existing SSH connection using the control socket.

## 4. Proposed Changes

### 4.1. Protocol: `HostId` in `InitializeResponse`

Update the protobuf schema so that `InitializeResponse` includes a `host_id` field. The server generates a stable identifier for the host and returns it during the initialize handshake, eliminating the need for a separate host-id probe step.

```protobuf
message InitializeResponse {
  string server_version = 1;
  string host_id = 2;
}
```

The server generates the `host_id` once when the long-lived server process starts (a v4 UUID). Since the remote host infrastructure deduplicates connections to a single long-lived server process, all clients connecting to the same host receive the same `host_id`.

**Server-side implementation**: The `ServerModel` generates a UUID at construction time and includes it in every `InitializeResponse`. Because the remote host routes multiple incoming connections to the same long-lived `ServerModel` process, all clients receive the same ID.

### 4.2. `HostId` newtype

Add to `crates/remote_server/src/host_id.rs` and re-export from `lib.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HostId(String);

impl HostId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
```

### 4.3. `RemoteServerManager` singleton model

Create `app/src/remote_server_manager.rs` (or `app/src/remote_server_manager/mod.rs` if it grows).

```rust
/// Per-session connection state. Encodes which data is available at each
/// lifecycle stage so the compiler prevents invalid combinations.
#[derive(Clone, Debug)]
pub enum RemoteSessionState {
    /// `connect_session` has been called; background task is starting the
    /// server process over SSH.
    Connecting,
    /// Server process spawned, client exists, initialize handshake in progress.
    Initializing { client: ModelHandle<RemoteServerClient> },
    /// Initialize handshake succeeded. Client is ready for requests.
    Connected { client: ModelHandle<RemoteServerClient>, host_id: HostId },
    /// Connection dropped (EOF/error from the reader task).
    Disconnected,
}

pub struct RemoteServerManager {
    /// Per-session connection state. Each SSH session gets its own dedicated
    /// connection to the remote server.
    sessions: HashMap<SessionId, RemoteSessionState>,
    /// Reverse index: host → sessions for O(1) lookup by `HostId`.
    host_to_sessions: HashMap<HostId, HashSet<SessionId>>,
    /// Spawner for running closures back on the main thread.
    spawner: ModelSpawner<Self>,
}
```

Each SSH session gets its own `RemoteSessionState`. The manager maps sessions → state directly, with no host-level connection sharing. The `HostId` lives inside the `Connected` variant — it's only available after the initialize handshake succeeds. The `host_to_sessions` reverse index gives downstream features O(1) lookup of all sessions on a given host, which they need for model deduplication and cleanup. If `Connected` grows beyond 2–3 fields in the future, we'll consider extracting into a `ConnectedSession` struct.

**Why per-session connections**: SSH control socket multiplexing ties all multiplexed connections to the parent session's lifecycle. If session A starts SSH and session B piggybacks via the control socket, session B's remote server connection dies when session A's SSH exits. Per-session connections ensure each session's remote server survives independently. The remote host infrastructure handles routing multiple connections to the same long-lived server process — dedup is the server's job, not the client's.

**`RemoteServerClient` as an Entity**: The `RemoteServerClient` is a warpui model (implements `Entity`) that can emit events and be subscribed to. It also derives `Clone`, producing a second handle to the same underlying channels — this is used to call async methods (e.g. `initialize`) from a background thread while the original lives inside a `ModelHandle`. The manager holds a `ModelHandle<RemoteServerClient>` in `server_clients`, and downstream features can subscribe directly to a specific client for server-pushed notifications (e.g. file change events, progress updates) rather than routing everything through the manager's event system.

**Entity and events**:

```rust
impl Entity for RemoteServerManager {
    type Event = RemoteServerManagerEvent;
}

impl SingletonEntity for RemoteServerManager {}

#[derive(Clone, Debug)]
pub enum RemoteServerManagerEvent {
    // --- Session-scoped events ---

    /// A connection flow has started for this session.
    SessionConnecting { session_id: SessionId },
    /// This session's server is connected and ready. Includes the HostId
    /// received from the initialize handshake, for model deduplication.
    SessionConnected { session_id: SessionId, host_id: HostId },
    /// This session's connection dropped.
    SessionDisconnected { session_id: SessionId, host_id: HostId },
    /// A session was deregistered (torn down).
    SessionDeregistered { session_id: SessionId },

    // --- Host-scoped events ---

    /// The first session for this host reached `Connected`. Downstream
    /// features should create per-host models (e.g. RepoMetadataModel).
    HostConnected { host_id: HostId },
    /// The last session for this host was disconnected or deregistered.
    /// Downstream features should tear down per-host models.
    HostDisconnected { host_id: HostId },
}
```

Events are emitted at two granularities. Session-scoped events fire for every session lifecycle change. Host-scoped events fire at the boundaries — `HostConnected` when the *first* session for a host reaches `Connected` (checked via `host_to_sessions`), and `HostDisconnected` when the *last* session for a host is disconnected or deregistered. This way downstream features that key on `HostId` can subscribe to host events for model lifecycle without reimplementing first/last tracking themselves. `SessionDisconnected` also carries `host_id` so consumers don't need to look it up from an already-transitioned state.

**Public API**:

```rust
impl RemoteServerManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Store a ModelSpawner for use by background tasks.
        ...
    }

    // --- Public API (called by the trigger layer and downstream features) ---

    /// Entry point called when an SSH session needs a remote server connection.
    /// Spawns a dedicated connection for this session.
    ///
    /// Immediately sets status to `Connecting` and emits `SessionConnecting`.
    /// Then spawns a background task that:
    /// 1. Runs `warp remote-server run` over SSH, creates the client
    /// 2. Calls `initialize()`, receives HostId, marks Connected
    pub fn connect_session(
        &mut self,
        session_id: SessionId,
        socket_path: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) { ... }

    /// Removes a session and tears down its connection.
    pub fn deregister_session(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) { ... }

    /// Returns the client handle for this session, if connected.
    pub fn client_for_session(
        &self,
        session_id: SessionId,
    ) -> Option<&ModelHandle<RemoteServerClient>> {
        match self.sessions.get(&session_id)? {
            RemoteSessionState::Connected { client, .. } => Some(client),
            _ => None,
        }
    }

    /// Returns the connection state for this session.
    pub fn session(
        &self,
        session_id: SessionId,
    ) -> Option<&RemoteSessionState> {
        self.sessions.get(&session_id)
    }

    /// Returns the HostId for this session, if the initialize handshake
    /// has completed. Used by downstream features for model dedup.
    pub fn host_id_for_session(
        &self,
        session_id: SessionId,
    ) -> Option<&HostId> {
        match self.sessions.get(&session_id)? {
            RemoteSessionState::Connected { host_id, .. } => Some(host_id),
            _ => None,
        }
    }

    /// Returns all session IDs connected to a given host. O(1) via the
    /// reverse index. Used by downstream features for model dedup and
    /// cleanup (e.g. tear down RepoMetadataModel when the last session
    /// for a host is deregistered).
    pub fn sessions_for_host(
        &self,
        host_id: &HostId,
    ) -> Option<&HashSet<SessionId>> {
        self.host_to_sessions.get(host_id)
    }

    // --- Private ---

    /// Transitions a session from `Initializing` to `Connected`.
    /// Moves the client out of the old variant and into the new one.
    fn mark_connected(
        &mut self,
        session_id: SessionId,
        host_id: HostId,
        ctx: &mut ModelContext<Self>,
    ) { ... }

    /// Transitions a session to `Disconnected`.
    fn mark_disconnected(
        &mut self,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) { ... }
}
```

**Registration**: The manager is registered as a singleton during app initialization in `app/src/lib.rs`, alongside other global models. It stores a `ModelSpawner<Self>` at construction time (same pattern as `TemplatableMCPServerManager`) for use by background tasks.

### 4.4. `connect_session` internal workflow

When `connect_session` is called, the manager owns the entire flow from that point. The specifics of *when and where* `connect_session` is triggered (e.g. from the terminal view after SSH bootstrap, or as part of a binary installation flow) are out of scope for this spec and will be addressed in a future iteration.

**Inside `connect_session`**: The manager sets the session status to `Connecting`, emits `SessionConnecting`, and spawns a background task with two phases.

**Phase 1 — Server startup and client creation**:

1. Run `warp remote-server run` over SSH as a long-running child process. This is a standalone `Command::new("ssh").spawn()` call — not routed through the session's `CommandExecutor`. Same SSH args pattern as `RemoteCommandExecutor` — ControlPath multiplexing, password auth disabled, X11 disabled.

```rust
let mut args = ssh_args(&socket_path);
args.extend(["warp", "remote-server", "run"].map(String::from));
let mut child = tokio::process::Command::new("ssh")
    .args(&args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
```

We use `.spawn()` (not `.output()`) because the remote server is long-running — `.output()` would block until the process exits. The spawn happens on the background thread because SSH connection establishment involves network I/O that shouldn't block the main thread.

2. Take ownership of the child's stdin/stdout/stderr.
3. Hop to the main thread via `spawner.spawn()` and:
   a. Create `RemoteServerClient::from_child_streams(child_stdin, child_stdout, child_stderr, runtime)` — this internally spawns the stderr forwarder and protocol reader/writer tasks.
   b. Subscribe to `RemoteServerClientEvent::Disconnected` on the client handle so the manager is notified when the connection drops.
   c. Transition the session to `Initializing { client }` — the client handle is stored early so the disconnect subscription works even if the handshake fails.
   d. Clone the `RemoteServerClient` (it derives `Clone`, producing a second handle to the same channels) for use in the async Phase 2.
4. If the server process fails to start: log the error and transition the session to `Disconnected`.

**Phase 2 — Initialize handshake**:

Back on the background thread, call `client.initialize()` on the cloned client. The `InitializeResponse` now includes `host_id`.
- On success: hop to main thread, call `mark_connected(session_id, host_id)` → transitions from `Initializing { client }` to `Connected { client, host_id }` and emits `SessionConnected { session_id, host_id }`.
- On failure: log the error, hop to main thread, call `mark_disconnected(session_id)` → transitions to `Disconnected` and emits `SessionDisconnected`. The session continues without remote server features.

Each session always gets a fresh connection. The remote server infrastructure on the host handles routing multiple connections to the same long-lived server process.

**Disconnection detection**: The `RemoteServerClient`'s reader task detects EOF (server crash or SSH death), clears pending requests, and emits `RemoteServerClientEvent::Disconnected`. During Phase 1, the manager subscribes to this event on each client handle. When the event fires, the subscription callback calls `mark_disconnected(session_id)`, which transitions to `Disconnected` and emits `SessionDisconnected`. Other sessions to the same host are completely unaffected — each has its own independent connection.

**Session teardown**: When `deregister_session(session_id)` is called, the manager removes the session from `sessions`. Dropping the `RemoteSessionState` (and its `ModelHandle<RemoteServerClient>` if present) closes the outbound channel, causing the writer task to exit, which closes stdin on the child process, which triggers EOF on the server side. The server infrastructure handles the lifecycle of the long-lived server process independently.

### 4.5. Client-side model deduplication via `HostId`

After the initialize handshake, the `HostId` is stored inside the `Connected` variant. Downstream features use this to share per-host models:

- When a feature needs a per-host model (e.g. `RepoMetadataModel`), it calls `manager.host_id_for_session(session_id)` and uses the `HostId` as the key into its own model registry.
- If two sessions return the same `HostId`, the feature reuses the existing model rather than creating a duplicate.
- When all sessions for a given `HostId` are deregistered, the feature tears down the per-host model. Use `manager.sessions_for_host(host_id)` to check if any sessions remain.

This keeps the `RemoteServerManager` focused on connection lifecycle while letting each downstream feature manage its own dedup policy.

## 5. End-to-End Flow

### New SSH session connecting to a host with no existing sessions

1. The caller (trigger TBD — see follow-ups) calls `manager.connect_session(session_id, socket_path)`. Manager sets status to `Connecting`, emits `SessionConnecting`.
2. Background task runs `ssh -o ControlPath=<socket> placeholder@placeholder warp remote-server run` → gets child stdin/stdout/stderr.
3. Hops to main thread: creates `RemoteServerClient::from_child_streams(...)`, subscribes to disconnect events, stores client handle. Clones client for async use.
4. Back on background thread: calls `client.initialize()` → receives `InitializeResponse { server_version: "1.2.3", host_id: "abc123" }`.
5. Hops to main thread: transitions session to `Connected { client, host_id: HostId("abc123") }`, emits `SessionConnected { session_id, host_id }`.
6. File tree subscribes to `SessionConnected`, calls `manager.host_id_for_session(session_id)` → `"abc123"`, creates a `RepoMetadataModel` keyed by `"abc123"`.

### Second SSH session to the same host

1. `manager.connect_session(session_id_2, socket_path_2)` → same flow, **new independent connection**.
2. Background task starts its own `warp remote-server run` over SSH. The remote host routes this connection to the same long-lived server process.
3. `client.initialize()` → `InitializeResponse { host_id: "abc123" }` (same host ID because it's the same server process).
4. File tree sees `host_id = "abc123"`, finds an existing `RepoMetadataModel` for that host — reuses it instead of creating a duplicate.
5. Session 2's `RemoteServerClient` is fully independent of session 1's. If session 1's SSH dies, session 2 is unaffected.

### Session disconnection

1. Session 1's `RemoteServerClient` reader task hits EOF (SSH session died), emits `RemoteServerClientEvent::Disconnected`.
2. Manager's subscription callback marks session 1 as `Disconnected`, emits `SessionDisconnected { session_id }`.
3. Session 2's connection is completely unaffected — it has its own SSH connection and `RemoteServerClient`.
4. Downstream features see session 1 disconnected but session 2 is still connected with the same `HostId`. Per-host models stay alive.

### Last session to a host deregistered

1. Both sessions are deregistered via `deregister_session`.
2. Manager removes each session's `RemoteSessionState`.
3. Downstream features call `manager.sessions_for_host(host_id)`, see no remaining sessions for `HostId("abc123")`, and tear down the per-host `RepoMetadataModel`.
