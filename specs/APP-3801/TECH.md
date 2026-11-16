# TECH.md — Remote Server Authentication (APP-3801)

Linear: [APP-3801](https://linear.app/warpdotdev/issue/APP-3801)

Behavior is specified in `specs/APP-3801/PRODUCT.md`. This document plans the implementation and documents the alternatives that were rejected because of how APP-4068 reshapes the server topology.

## 1. Context

### Terminology note

The originating Linear ticket is titled "remote server initialization with authentication runtime flags", which reads like CLI flags on `oz remote-server*`. The design does **not** introduce such flags. Once APP-4068's daemon topology is in scope, every startup-time credential transport (argv, env, fd inheritance, file handoff) fails for the same set of reasons — §4 walks through each. The "runtime" in this design means runtime protocol fields — an `auth_token` carried on `Initialize` for the initial credential, and a dedicated `Authenticate` message for mid-session rotation — exchanged over the already-encrypted client↔server byte stream, not CLI args parsed at process start. Reviewers who come in expecting `--auth-token` should start at §4.

### Remote server today

- `crates/remote_server/proto/remote_server.proto` — `ClientMessage`/`ServerMessage` envelopes, `Initialize`/`InitializeResponse`, shared `ErrorResponse` with `ErrorCode { UNSPECIFIED, INVALID_REQUEST, INTERNAL }`. No auth fields anywhere.
- `crates/remote_server/src/client.rs (117-199)` — `RemoteServerClient::initialize()`, per-request correlation via `pending_requests`, fire-and-forget helpers (`send_notification`, `notify_session_bootstrapped`).
- `crates/remote_server/src/manager.rs (168-283)` — `RemoteServerManager::connect_session` drives Setup → Launch → `Initialize` handshake → `Connected`.
- `app/src/remote_server/mod.rs:21` — `run()` configures stderr-only logging and boots the headless warpui app; reads no CLI args or env.
- `app/src/remote_server/server_model.rs (118-480)` — `ServerModel` state, `handle_message` dispatch, `handle_initialize` returns `server_version` + `host_id`.
- `crates/warp_cli/src/lib.rs:430-433` — `WorkerCommand::RemoteServer` is a unit variant; `app/src/lib.rs:548-551` dispatches to `remote_server::run()` with no args.

### Credential sources on the client

- `app/src/auth/credentials.rs` — `Credentials::{Firebase, ApiKey, SessionCookie, Test}`, `AuthToken::{Firebase(String), ApiKey(String), NoAuth}`, `AuthToken::bearer_token()`.
- `app/src/server/server_api/auth.rs (243-280)` — `ServerApi::get_or_refresh_access_token()` returns a fresh `AuthToken`, transparently refreshing Firebase tokens 5 minutes before expiry. Emits `ServerApiEvent::NeedsReauth` on refresh failure.

### APP-4068 daemon topology (the operative constraint)

APP-4068 splits the binary into `remote-server-proxy` (byte-bridging SSH stdio to a Unix socket via `std::io::copy`) and `remote-server-daemon` (long-lived, spawned by the first proxy via `setsid` with null stdio, serves multiple concurrent proxy connections, 10-minute grace after last disconnect). `ServerModel` already tracks connections in `HashMap<ConnectionId, Sender<ServerMessage>>` and exposes `register_connection(id, sender)` / `deregister_connection(id)`. Routing of responses is per-`ConnectionId` — there is no broadcast path except explicit fan-out over the map.

The critical invariants this imposes on auth design:

- Which tab spawned the daemon is an accident of timing — any credential baked into the daemon at spawn belongs statically to whichever proxy won the `flock` race, with no path to update it later.
- The proxy is deliberately protocol-agnostic; anything the proxy has to understand about auth is new coupling.
- The daemon outlives any single SSH session, so credential rotation has to work at any point during the daemon's life.

### Identity key and socket-path partitioning (APP-4068 foundational assumption)

APP-4068 partitions daemon sockets by user identity at the path level: `~/.warp[-channel]/remote-server/{identity_key}/server.sock`. The `{identity_key}` is:

- **Logged-in users**: the Warp canonical user UUID.
- **Anonymous users**: a per-install persistent UUID loaded from user preferences under key `"ExperimentId"`, generated and saved if absent. This ensures all tabs for the same anonymous install share a single daemon and socket, matching the logged-in behavior.

APP-3801 treats this partitioning as a given. Because the socket is identity-scoped, all connections to a given daemon belong to the same user by construction, and the daemon can store a single credential for its whole lifetime rather than one per connection. APP-3801 does not add any cross-user authentication logic. For anonymous users with no bearer token, `Initialize` carries an empty `auth_token` (no credential provided); handler behavior in the unauthenticated case is out of scope and ships with the first handler that actually needs an upstream credential (§7).

## 2. Proposed changes

### 2.1 Protocol (`crates/remote_server/proto/remote_server.proto`)

```protobuf
message ClientMessage {
  string request_id = 1;
  oneof message {
    Initialize initialize = 2;
    // ... existing variants ...
    Authenticate authenticate = 11;  // new, rotation only
  }
}

// Initialize gains an optional auth_token field that carries the daemon's
// initial credential as part of the handshake. Empty string means "no
// credential provided" (anonymous users); the daemon leaves its existing
// auth_token unchanged in that case rather than clearing it.
message Initialize {
  // ... existing fields ...
  string auth_token = N;  // new; empty = no credential provided
}

// Client → server: refresh the daemon's credential mid-session.
// Fire-and-forget. Sending replaces the previously stored credential
// (last-writer-wins; the daemon stores one token for its whole lifetime, §4.7).
message Authenticate {
  string auth_token = 1;
}
```

No other protocol additions. `ErrorCode` stays at the existing `{ UNSPECIFIED, INVALID_REQUEST, INTERNAL }`; `ErrorResponse` is unchanged. Auth-specific error codes (`MISSING_CREDENTIAL`, `CREDENTIAL_REJECTED`, `PERMISSION_DENIED`, and any sub-reason enum) are deferred to the follow-up PR that introduces the first handler actually needing them (§7) — defining them here with no caller would ship dead protocol surface.

No `ClearCredentials` message either. Mid-life clearing is not part of the protocol; the daemon holds the credential until process exit (§6).

Two paths exist for writing the daemon's singleton: `Initialize.auth_token` carries it during handshake (covering every new connection in a single round-trip), and `Authenticate` updates it mid-session (covering rotation). The split saves one fire-and-forget message per new connection over the SSH-bridged topology; see §4.5 for why the original unified `Authenticate`-only design was rejected in favor of this bundling.

### 2.2 Client (`crates/remote_server/src/client.rs`)

```rust
impl RemoteServerClient {
    /// Perform the Initialize handshake, optionally carrying the daemon's credential.
    /// If `auth_token` is `Some`, the server stores it as the daemon-wide singleton
    /// as part of handshake processing. If `None`, no credential is set or cleared.
    pub async fn initialize(
        &self,
        auth_token: Option<&str>,
    ) -> Result<InitializeResponse, ClientError> {
        // ... existing initialize logic, with auth_token wired into the Initialize message ...
    }

    /// Refresh the daemon's credential mid-session. Fire-and-forget.
    /// Used on token rotation only; initial auth rides on `initialize`.
    pub fn authenticate(&self, auth_token: &str) {
        let msg = ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::Authenticate(
                Authenticate { auth_token: auth_token.to_owned() },
            )),
        };
        self.send_notification(msg);
    }
}
```

No `ClientError` additions in this PR. Auth-specific error-code handling lands with the first handler that produces those codes (§7).

### 2.3 Manager (`crates/remote_server/src/manager.rs`)

**Pre-initialize auth step:**

Phase 4 of `connect_session` fetches the credential *before* `initialize` and passes it through as part of the handshake, rather than as a follow-up fire-and-forget message.

1. Before calling `client.initialize(...)`, hop to the main thread and obtain a fresh `AuthToken` via `ServerApi::get_or_refresh_access_token()` (reached through a new `AuthProvider` handle passed into `connect_session`, kept abstract so `remote_server` stays independent of `app/src/server`).
2. Call `client.initialize(auth_token.bearer_token())`. If the `Option` is `Some`, the daemon stores the token as part of handshake processing. If `None` (anonymous user, or an `AuthToken::NoAuth`), the daemon leaves its singleton untouched — in the anonymous case there is nothing to set; in the pathological case of a logged-in user arriving on an already-populated daemon, the existing credential is still valid.
3. Transition to `Connected`.

No separate follow-up `authenticate` call on first connection. See §3's mermaid for the single-round-trip flow and §4.5 for why the original split-message design was rejected.

**Token rotation (pick-one, no fan-out):**

Subscribe `RemoteServerManager` to the client-side auth state. On token rotation (the existing Firebase refresh path in `ServerApi`), pick one arbitrary `Connected` session and call `client.authenticate(new_token)` on it. The daemon stores a single credential shared across all its connections, so one send is sufficient to update every handler's view of the current token. This pattern matches how the manager already sends other server-bound notifications. If no sessions are `Connected` at the moment of rotation, the rotation is a no-op; the next new session's `initialize` carries the current token.

**Logout path:**

On logout, the client tears down its remote connections. No explicit server-side clear is sent — the daemon retains the stale token in memory until its grace period expires and the process exits. See §6 for the trade accepted.

No change to `start_remote_server` or SSH args. No credential enters argv or env.

### 2.4 Server (`app/src/remote_server/server_model.rs`)

```rust
pub struct ServerModel {
    // ... existing fields ...
    /// Daemon-wide credential, populated by handle_initialize (when the
    /// Initialize message carries a non-empty auth_token) or by
    /// handle_authenticate (mid-session rotation). Stored as a plain String;
    /// the server forwards tokens opaquely without categorising or validating
    /// them locally. Never cleared except by daemon process exit — see §4.7
    /// for why per-connection state was rejected given APP-4068's identity-
    /// scoped daemon.
    auth_token: Option<String>,
}

fn handle_initialize(&mut self, msg: Initialize) -> InitializeResponse {
    if !msg.auth_token.is_empty() {
        log::info!("Initialize carries credential (token=<redacted>)");
        self.auth_token = Some(msg.auth_token);
    }
    // If auth_token is empty: do nothing. Never clear an existing singleton
    // from Initialize — a reconnect with no token must not invalidate the
    // daemon's current credential.
    //
    // ... existing response construction (server_version, host_id, ...) ...
}

fn handle_authenticate(&mut self, msg: Authenticate) {
    log::info!("Handling Authenticate (token=<redacted>)");
    self.auth_token = Some(msg.auth_token);
}

pub fn auth_token(&self) -> Option<&str> {
    self.auth_token.as_deref()
}

// `deregister_connection` (added by APP-4068) is unchanged by this PR.
// `auth_token` is deliberately retained across connection teardown;
// the only cleanup event is daemon process exit (grace-period expiry,
// SIGTERM, panic). See §6.
```

**Why daemon-wide rather than per-connection?** APP-4068 already partitions daemon socket paths by user identity (§1), so every connection on a given daemon belongs to the same Warp user by construction and will present the same bearer token. A `HashMap<ConnectionId, String>` would hold N copies of the same string with no behavioral difference. §4.7 walks through the per-connection alternative in full; the short version is that `Option<String>` is simpler, smaller, and avoids cleanup machinery that has no work to do.

## 3. End-to-end flow

### Overview

Three flows — fresh connection, proactive refresh, and server-side teardown — route through two protocol touchpoints: the initial credential rides on the existing `Initialize` handshake (as a new `auth_token` field), and mid-session refreshes use the new fire-and-forget `Authenticate` message. The server is a passive recipient in both cases: it writes into its singleton `auth_token` slot when either carries a value, and does nothing else auth-related. The client-side `RemoteServerManager` owns every decision about *when* to send. Handler behavior when a credential is missing, rejected, or insufficient is out of scope here (§7).

### User identity scoping

User identity is established before any APP-3801 message flows. The client selects the daemon socket path by identity key — the Warp canonical user UUID for logged-in users, or a per-install persistent UUID for anonymous users (see §1 "Identity key and socket-path partitioning"). Because each daemon is bound to exactly one socket path, every connection it accepts already belongs to a single user by construction, and the daemon stores a single credential for all of them. APP-3801 carries no additional user-identifier field on the wire; the bearer token carried on `Initialize` (or `Authenticate` at refresh time) is the only user-identifying information the daemon sees.

**How identity flows in practice.** Using `abc123` as a placeholder for the user's Warp UUID:

1. **Client computes the path.** Warp (running on the local machine) already has the user's identity in memory. It builds the socket path: `~/.warp/remote-server/abc123/server.sock`. For anonymous users, `abc123` is replaced with the per-install `ExperimentId` UUID loaded from preferences.
2. **Client launches the proxy with that path.** When the user opens a remote session, the client invokes something like `oz remote-server-proxy --socket-path ~/.warp/remote-server/abc123/server.sock` on the remote host over SSH. The proxy receives the path as an opaque argv string — it does not parse, validate, or interpret the UUID segment; it just knows where the socket lives.
3. **Proxy finds or spawns the daemon.** The proxy attempts to `connect()` at the given path. If a daemon is already listening, it joins. If not, it spawns one (via `setsid`) whose first action is to `bind()` the socket at that path. APP-4068 owns this whole dance; APP-3801 rides on top of it.
4. **Daemon has no UUID awareness.** The daemon never sees a UUID in its own code. It does not parse an identifier out of any message, does not validate users, does not know what `abc123` means. It listens on the socket it was told to listen on and accepts whatever connections arrive there.

**Why this scopes identity without a protocol field:**

- A second tab from the same Warp user → same UUID → same path → same daemon (joins it).
- A tab from a different Warp user → different UUID → different path → different daemon (separate process).
- A process running as a different OS user on the remote host → cannot reach the socket at all, because the parent directory `abc123/` is `mode 0700` owned by the one OS user. The kernel's permission check enforces it.

The daemon never needs to trust a user identifier on the wire because the identifier isn't on the wire. It is in the file-system path, and the OS's permission check is what turns "a process requested connection to this path" into "that process is running as the owning user."

```mermaid
sequenceDiagram
    participant API as ServerApi (client)
    participant Mgr as RemoteServerManager
    participant C1 as Client (tab 1)
    participant C2 as Client (tab 2)
    participant Server as ServerModel (daemon)

    Note over Mgr,Server: 1. Initial authentication (first connection, single round-trip)
    Mgr->>API: get_or_refresh_access_token()
    API-->>Mgr: AuthToken
    Mgr->>C1: initialize(Some(token))
    C1->>Server: ClientMessage(Initialize{auth_token})
    Note right of Server: handle_initialize sets auth_token = Some(token)
    Server-->>C1: ServerMessage(InitializeResponse)

    Note over Mgr,Server: 2. Additional connection on same daemon (same single RTT)
    Mgr->>C2: initialize(Some(token))
    C2->>Server: ClientMessage(Initialize{auth_token})
    Note right of Server: auth_token overwritten with same value
    Server-->>C2: ServerMessage(InitializeResponse)

    Note over API,Server: 3. Proactive refresh (~5 min before expiry, pick-one)
    API-->>Mgr: token-rotated event
    Mgr->>C1: authenticate(new_token)
    C1->>Server: ClientMessage(Authenticate)
    Note right of Server: auth_token = Some(new_token)
    Note over C2: no message sent — singleton already updated

    Note over Server: 4. Server-side teardown (SSH drop / crash / logout)
    C1--xServer: connection closes
    Note right of Server: deregister_connection runs; auth_token retained
    C2--xServer: connection closes
    Note right of Server: grace timer starts; auth_token still retained
    Note right of Server: grace expiry → process exit → auth_token gone
```

### Fresh connection

1. Client calls `manager.connect_session(session_id, socket_path)`.
2. Manager runs Setup / Launch / creates `RemoteServerClient`.
3. Manager calls `get_or_refresh_access_token()`, then `client.initialize(auth_token.bearer_token())`. If `bearer_token()` is `Some`, the initial credential rides on the `Initialize` message; if `NoAuth`, `None` is passed and the daemon leaves its singleton untouched.
4. Server: `handle_initialize` runs the existing handshake logic and, if the carried `auth_token` is non-empty, sets `auth_token = Some(token)`. Responds with `InitializeResponse`. If this is not the first connection on this daemon, the value overwrites the existing singleton with the same token (same user, same `ServerApi`).
5. Session transitions to `Connected`.

### Token rotation

1. Client-side `ServerApi` refreshes its Firebase ID token (existing auto-refresh).
2. `RemoteServerManager` observes the rotation event and picks one arbitrary `Connected` session, calling `client.authenticate(new_token)` on it. This pattern matches how the manager already handles other server-bound notifications.
3. Server: `handle_authenticate` replaces the singleton.
4. In-flight requests using the previous token continue — warp-server accepts a refresh overlap window of both old and new tokens.
5. If no sessions are `Connected` at the moment of rotation, the rotation is a no-op; the next new session's `initialize` carries the current token.

### SSH drop / daemon grace-period expiry

1. Proxy sees SSH EOF → exits → daemon's accept loop observes connection close → `deregister_connection(connection_id)`.
2. `auth_token` is retained. There is no per-connection auth state to clean up.
3. Daemon continues serving other connections (if any) with the same singleton credential.
4. When the last connection leaves, APP-4068's grace timer starts. The daemon still holds `auth_token` during the grace window.
5. On grace-period expiry (or SIGTERM, panic) the daemon process exits. All in-memory state, including `auth_token`, dies with the process.

## 4. Alternatives considered

Each was considered and rejected. Each failure mode is stated with respect to APP-4068's daemon topology.

### 4.1 CLI flag on the server binary (`--auth-token`)

Shape: make `WorkerCommand::RemoteServer` a struct variant carrying `--auth-token`; the client includes the token in the SSH launch command.

Why rejected:

- **Leaks via `ps`.** On any shared-user remote host, argv is world-readable. Disqualifying on security grounds before even considering the daemon.
- **No refresh channel; stale at spawn.** Firebase ID tokens expire in ~1 hour. A startup-only credential cannot be updated once baked into argv: even if a later tab on the same daemon has a fresher token, there is no protocol path for it to land on the already-spawned daemon. The runtime `Authenticate` message this spec introduces is the only way to ship a rotation-capable credential.

### 4.2 Environment variable (`WARP_REMOTE_AUTH_TOKEN`)

Why rejected:

- Readable via `/proc/$pid/environ` on shared hosts.
- SSH strips env by default; `SendEnv`/`AcceptEnv` cooperation not guaranteed on arbitrary remote hosts.
- Same "no refresh channel, stale at spawn" problem as §4.1.

### 4.3 File-based handoff

Why rejected:

- Brief disk artifact; a crash between write and unlink leaves a credential on disk.
- Ambiguous ownership in daemon mode (multiple tabs drop distinct files; daemon must pick one).
- One-shot; doesn't model rotation.

### 4.4 File-descriptor handoff (`--auth-fd 3`)

Why rejected:

- No fd inheritance path to the daemon. APP-4068 spawns the daemon via `Command::pre_exec(setsid)` with null stdio; inherited fds are dropped.
- Even if the proxy held such an fd, the daemon-spawn step drops it.
- One-shot; doesn't model rotation.

### 4.5 Separate `Authenticate` for initial auth (in addition to refresh)

Shape: `Initialize` carries no credential. The client sends a fire-and-forget `Authenticate` message immediately after `Initialize` completes, and again on any later rotation. Both initial and refresh paths use the same message.

This is how the spec was structured before the bundling pivot — a unified "one message for setting credentials" surface.

Why rejected:

- **Extra message per new connection.** Every new connection on a daemon would send `Initialize` + one follow-up `Authenticate`, rather than a single `Initialize` carrying the token. On the SSH-bridged topology (Warp → SSH → remote proxy → Unix socket → daemon), each message adds bytes on the wire and a processing round on both ends. `Authenticate` is fire-and-forget so it does not block an RTT, but it adds one message per new connection for no semantic gain — the token was available to the client *before* `Initialize` was sent.
- **Transient unauthenticated state.** With split messages, the session briefly exists in an "Initialize complete, Authenticate in-flight" window. No one observes this state today (no handlers use `auth_token` yet, §7), but the moment the first such handler lands, either the client or the handler has to reason about what happens if a request arrives during that window. Bundling collapses this: once `InitializeResponse` is received, the daemon's credential is already set.
- **No unified-surface benefit.** The ostensible win of "one path to set the credential" is surface-level; `Authenticate` stays in place for refresh regardless, so both paths exist either way. The question is whether the split buys anything, and it doesn't.

### 4.6 Handshake-time upstream validation

Why rejected:

- Doubles handshake latency.
- Adds a failure mode orthogonal to the credential itself (transient network issue on the remote host blocks authentication even when the token is fine).
- Store-and-forward produces the same end-to-end behavior via the reactive-refresh path at lower cost.

### 4.7 Per-connection credential map

Shape: `ServerModel` holds `auth_tokens: HashMap<ConnectionId, String>` instead of `auth_token: Option<String>`. Each `Authenticate` is keyed by the connection that sent it; each `deregister_connection` clears that connection's entry; each handler reads the entry for the connection its request arrived on. A dedicated `ClearCredentials` message pairs with the map to give the client a proactive clear path on logout.

This is the closest alternative to the chosen design — it was the initial shape of APP-3801 before the design collapsed to a singleton.

Why rejected:

- **Storage doesn't reflect the data model.** APP-4068 partitions daemon socket paths by user identity (§1), so every connection on a given daemon belongs to the same Warp user and presents the same bearer token. A per-connection map holds N copies of the same string with no behavioral difference; the storage structure implies a flexibility the system does not provide.
- **Fan-out with no purpose.** Token rotation in the per-connection model requires the manager to iterate every `Connected` session and call `authenticate` on each — N messages to write the same value N times. The singleton collapses this to one `authenticate` on any arbitrary connection, matching the pick-one pattern already used for other manager-to-server notifications.
- **Cleanup machinery with nothing to clean.** Per-connection storage motivates a `ClearCredentials` message, a `handle_clear_credentials` branch, and an explicit `auth_tokens.remove` inside `deregister_connection` — each guarding narrow logout or teardown windows. With a singleton, none of these are needed: the daemon process itself is the trust boundary (Unix socket, user-partitioned path, OS-level file permissions owned by APP-4068), and the credential dies with the process on grace-period expiry. The per-connection design pays a fixed protocol-surface cost to close a window that, at realistic durations, is already bounded by the grace period the singleton accepts (§6).

**What the singleton gives up:** foreclosing per-connection policy if it is ever needed (different bearer tokens per tab, for example). No such use case exists today or is on the roadmap; adding per-connection state back later is a straight refactor if the need arises.

## 5. Testing and validation

- **Daemon-wide authentication:** unit tests on `ServerModel` using in-memory transports. Send `Initialize` with a non-empty `auth_token`; assert `auth_token()` returns the value. Send a second `Authenticate` (rotation); assert last-writer-wins. Send `Initialize` with an empty `auth_token` on a daemon whose singleton is already populated; assert the existing value is preserved (empty on Initialize never clears). Register a second connection and send on either one; assert the singleton reflects the last write regardless of source connection.
- **Lifecycle:** initialize with a token, deregister; assert `auth_token()` is still `Some` after `deregister_connection` (deliberate retention — no per-connection cleanup). Drop the `ServerModel`; assert a freshly-constructed model has no `auth_token`. Assert no file artifacts in `~/.warp*` contain the token at any point during the above.
- **Client and manager:** `RemoteServerClient` unit tests for `initialize(Some(token))` producing an `Initialize` message with `auth_token` set, and `initialize(None)` producing one with the field empty. Unit test for `authenticate(token)` producing the expected `Authenticate` `ClientMessage`. `RemoteServerManager` integration test: new-session flow fetches the token via `ServerApi` *before* calling `initialize` and passes it through. Rotation test: rotation event sends `authenticate` to exactly one `Connected` session (pick-one, no fan-out) and none of the `Disconnected` ones.
- **Security:** CI grep check that no `{:?}`/`Debug` formatting is applied to `ClientMessage`/`Initialize`/`Authenticate` inside server-side log sites. Unit test of `describe_client_message` on an `Initialize` input with a non-empty `auth_token` and on an `Authenticate` input asserts the token field is replaced with `<redacted>`. Audit test that `start_remote_server` argv and env contain no credential-shaped strings.

## 6. Risks and mitigations

- **Token-in-log regression.** A future contributor adds `log::info!("{:?}", msg)` in the server path. Mitigation: CI grep check on `{:?}`-formatting of `ClientMessage`/`Authenticate` inside `app/src/remote_server/` and `crates/remote_server/`, plus a redaction helper; all existing sites converted in this PR.
- **Credential persists during daemon grace period (accepted).** After the last connection disconnects, APP-4068's daemon keeps running for up to 10 minutes before exiting. During that window the daemon process still holds `auth_token` in memory with no active consumer. Mitigation: the daemon's Unix socket is owned by APP-4068 with user-scoped file-system permissions — the same security boundary that protects live credentials on disk. Reaching the socket requires OS-level access as the owning user, at which point far more sensitive material is already accessible. Treating the grace-period window as a real exposure would require either a second timer (artificial cleanup) or teardown-on-last-disconnect (collapsing the daemon model). Neither is justified by the actual threat.
- **Stale token after logout.** If the user logs out while one or more remote sessions are still alive, the daemon holds the (now-invalidated) token until its process exits at grace-period expiry. Mitigation: the client tears down the remote connections on logout, which starts the grace timer; warp-server treats the logged-out token as revoked on any upstream call, so the window is a latency issue, not an authorization-bypass issue.
- **Credential type heterogeneity.** `AuthToken` can be `Firebase(String)`, `ApiKey(String)`, or `NoAuth`. The server stores the opaque bearer string. `NoAuth` users skip `authenticate` and leave `auth_token` unset; handler behavior for that state ships with the first handler that needs upstream auth (§7). No server-side changes are needed when a new credential variant is added as long as it produces a bearer string.

## 7. Follow-ups

- **First handler that actually calls upstream.** This spec establishes only the plumbing (`ServerModel::auth_token`, `handle_initialize`'s auth-setting branch, `handle_authenticate`, `RemoteServerClient::initialize(auth_token)`, `RemoteServerClient::authenticate`, manager-side pre-initialize credential fetch and rotation wiring). No handler in this PR calls `auth_token()`. The follow-up PR introduces the first such handler and is where the `MISSING_CREDENTIAL`, `CREDENTIAL_REJECTED`, `PERMISSION_DENIED`, and any `ErrorSubReason` additions land — defined together with their first caller rather than as dead protocol surface now.
- **Client-side automatic reconnect.** If APP-4068's reconnect follow-up lands, the new session's post-initialize auth step re-authenticates naturally — no changes needed here.
- **Daemon-scoped handlers without a `ConnectionId` context.** If Warp ever needs a handler that runs outside any connection (periodic background sync, cross-tab broadcast), the singleton `auth_token` is directly usable. The current design already assumes every upstream call reads from daemon state rather than per-connection state.
