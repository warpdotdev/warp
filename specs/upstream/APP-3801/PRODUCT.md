# Remote Server Authentication

Linear: [APP-3801](https://linear.app/warpdotdev/issue/APP-3801)

## Summary

The remote server gains a daemon-wide authentication layer so that handlers running on a remote host can make authenticated upstream calls to Warp services on behalf of the Warp user driving the daemon. The initial credential rides on the existing `Initialize` handshake as a new `auth_token` field (no extra round-trip on connection setup); mid-session rotation uses a dedicated new `Authenticate` message. The credential lives in daemon memory for the daemon's lifetime and is cleared only on process exit — there is no explicit protocol message to clear it mid-life.

## Problem

Today the remote server has no notion of user identity. Any handler that needs to call Warp services (`app.warp.dev` APIs, upstream LLM routing, telemetry attribution, Drive-backed features) from the remote host has no credential to present. At the same time, APP-4068 makes the server a long-running daemon shared across multiple client connections from the same user's tabs, with a 10-minute grace period after the last disconnect. Any credential model for this system has to work within that architecture: the daemon is started by whichever proxy got there first, serves multiple concurrent connections, and may outlive any single SSH session.

## Goals

- Give the daemon a way to receive the current Warp credential for the user that owns its socket path.
- Let handlers running on the daemon use that credential for upstream calls.
- Support mid-session credential rotation so short-lived tokens (Firebase ID tokens) can refresh without tearing down the connection.
- Keep the credential in daemon memory only — never on disk, never in process arguments or environment.
- Minimize protocol surface: one new `auth_token` field on `Initialize` for the initial credential; one new `Authenticate` message for mid-session rotation; no new error codes in this PR, no explicit clear message.

## Non-goals

- Multi-user authentication on a shared daemon. All connections on a given daemon belong to the same Warp user by construction — socket-path partitioning by identity key in APP-4068 enforces this at the file-system level.
- Validating credentials locally on the remote host. Validity is determined by the upstream service that receives them.
- Persisting credentials across server restarts or across the daemon's grace-period expiry.
- Securing the server against adversarial co-located Unix users on the remote host. SSH is assumed to be the trust boundary.
- Authenticating the `Initialize`/`InitializeResponse` handshake itself. The handshake remains anonymous.
- Remote MCP spawn or server-side MCP credential handling. MCP is assumed client-side.

## Behavior

### Proxy and daemon topology

The remote server runs as two distinct process roles on the remote host:

**Daemon** (`remote-server-daemon`): a long-lived process scoped to a single user identity. The first proxy to connect for a given identity spawns the daemon; subsequent proxies join the already-running daemon. The daemon listens on a Unix socket whose path is partitioned by an identity key — the Warp canonical user UUID for logged-in users, or a per-install persistent UUID for anonymous users. All authentication state lives exclusively in the daemon, as a single credential shared by all of its connections.

**Proxy** (`remote-server-proxy`): a short-lived process scoped to a single SSH session. Each Warp tab that connects to a remote host spawns its own proxy. The proxy byte-bridges SSH stdin/stdout to the daemon's Unix socket and is auth-unaware — it does not inspect, hold, or forward credentials. One user can have multiple proxies connected to the same daemon simultaneously (one per open tab).

The lifecycle of auth state maps to process events as follows:

- **Proxy connects**: a new connection is registered with the daemon. The client calls `initialize(auth_token)` on that connection; the daemon stores (or overwrites) its singleton credential as part of handshake processing. No follow-up message is needed.
- **Proxy exits** (SSH drop, tab close, user logout): the daemon deregisters that connection. The singleton credential is retained. Handlers on any remaining connections continue to see it.
- **Last proxy exits**: the daemon enters its up-to-10-minute grace period. The credential is still held in memory but has no active consumer. A new proxy arriving during this window joins the existing daemon and sees the credential already populated; it nonetheless carries the current token on its own `Initialize` to keep the protocol path uniform.
- **Daemon exits** (SIGTERM, panic, or grace-period expiry): all in-memory state, including the credential, is lost. The next proxy to connect starts a fresh daemon with no credential.

The daemon never mutates its credential in response to connection events. The singleton is only written by `Initialize` (carrying `auth_token`) or `Authenticate`, and only cleared by process exit.

### Connection authentication

1. The client carries its current bearer token on the `Initialize` handshake itself (as the new `auth_token` field). The daemon stores it as its single credential as part of handshake processing. No follow-up message is required to complete initial authentication.
2. The credential is daemon-wide, not per-connection. All connections on a given daemon share one credential because the socket path guarantees they all belong to the same Warp user.
3. For mid-session rotation, the client sends a fire-and-forget `Authenticate` message. It carries only a new bearer token; no acknowledgement is sent. `Initialize` retains its existing request-response structure and is only used for new connections.
4. Any connection may carry a refresh `Authenticate`; the client picks one arbitrarily and does not fan out. Initial authentication and refresh are distinct on the wire (the former rides on `Initialize`), though both write to the same daemon singleton.

### Server-side credential usage

5. When a handler needs an upstream credential, it reads the daemon's single credential. The originating connection's identity is irrelevant.
6. If the daemon has no credential stored, handler behavior is defined by the PR that introduces the first such handler (see TECH.md §7). It is out of scope for this PR; no handler in this PR reads the credential.
7. Local-only handlers (filesystem operations, local command execution, repo metadata indexing) behave identically whether or not the daemon has a credential.

### Lifecycle

8. The credential exists only in daemon memory. It is never written to disk, environment variables, process arguments, or any on-disk artifact of any `oz remote-server*` binary.
9. Deregistering a connection does not clear the credential. The daemon deliberately retains it across connection teardown so that other connections (and any future reconnects to the same daemon) continue to work without re-auth machinery.
10. The credential is cleared only on daemon process exit — SIGTERM, panic, or APP-4068's grace-period expiry after the last proxy disconnects. There is no intermediate cleanup path.
11. A reconnected client (after SSH drop or explicit teardown) rejoins the existing daemon if it is still alive and sees the same credential. If it arrives after a daemon restart, it observes a fresh daemon with no credential and must authenticate.

### Client-side responsibilities

12. `RemoteServerClient` exposes two methods: `initialize(auth_token)` carries the initial credential during handshake, and `authenticate(token)` refreshes the credential mid-session. The client is responsible for calling `initialize` with the current bearer token when establishing a new connection, and `authenticate` only when the local token rotates.
13. On rotation, the manager picks one arbitrary `Connected` session and sends `authenticate` on it. No fan-out; the daemon's singleton propagates the new value to every handler.
14. On logout, the client tears down its remote connections. The daemon's credential is cleared only when the daemon process exits at grace-period expiry. Mid-life clearing is not part of the protocol.

### Security invariants

15. The credential is transmitted only over the already-encrypted client-to-server byte stream (SSH stdin/stdout for the per-SSH topology; the local Unix socket for the APP-4068 daemon topology, whose file-system permissions are owned by APP-4068 and scoped to the owning user). It never appears in process arguments, environment variables, or on-disk artifacts of any `oz remote-server*` binary.
16. The credential is never written to logs. Server-side log statements redact the credential field whenever `Initialize` or `Authenticate` messages are traced.
