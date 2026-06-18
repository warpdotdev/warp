# Remote Command Execution for SSH Completions via RemoteServerManager

Linear: [APP-3791](https://linear.app/warpdotdev/issue/APP-3791/code-feature-support-completions)

## 1. Problem

When a user SSHes into a remote host, Warp's completions pipeline (autosuggestions, syntax highlighting, tab completions) needs to run generator commands on the remote machine. Today this is done by opening a new SSH session per command, which is constrained by the host's `MaxSessions` limit and cannot run commands in parallel.

This spec covers building the completions path through the persistent `remote_server` process:

1. **Proto messages** — `RunCommandRequest` / `RunCommandResponse` for executing shell commands on the remote host.
2. **Server-side handling** — `ServerModel` dispatches `RunCommand` to `$SHELL -c`, returns output.
3. **Client-side API** — `RemoteServerClient.run_command()` sends the request and correlates the response.
4. **`RemoteServerCommandExecutor`** — a `CommandExecutor` that uses a `RemoteServerClient` to execute commands.
5. **Wiring** — how the executor gets its client from the manager via event subscription.

### Scope

Triggering `RemoteServerManager.connect_session()` is handled in a separate flow. The manager's connection flow sends an `Initialize` handshake (for version/host negotiation) followed by a `SessionBootstrapped` notification that carries `session_id`, `shell_type`, and `shell_path` so the server creates a per-session `LocalCommandExecutor` matching the bootstrapped shell. This spec assumes the server is already running and the manager is in `Connected` state. Error conditions are logged but not surfaced to the user.

## 2. Relevant Code

### Protocol
- `app/proto/remote_server.proto` — `ClientMessage`/`ServerMessage` envelopes, `RunCommandRequest` (field 7 on `ClientMessage`), `RunCommandResponse` (field 8 on `ServerMessage`)
- `app/src/remote_server/protocol.rs` — length-delimited protobuf read/write helpers, `RequestId` newtype

### Server-side
- `app/src/remote_server/server_model.rs` — `ServerModel` dispatches `handle_message` on the main thread; `RunCommand` arm delegates to `LocalCommandExecutor` via `ctx.spawn_abortable`, sends `RunCommandResponse` back through `response_tx`

### Client-side
- `app/src/remote_server/client.rs` — `RemoteServerClient` with `run_command()`, `initialize()`, background reader/writer tasks, `ClientError` enum, `ClientEvent::Disconnected`

### Remote server manager
- `app/src/remote_server/manager.rs` — `RemoteServerManager` singleton with per-session state (`RemoteSessionState` enum: `Connecting` → `Initializing` → `Connected` → `Disconnected`), `connect_session`, `client_for_session`, `deregister_session`, session-scoped events (`SessionConnected`, `SessionDisconnected`) and host-scoped events (`HostConnected`, `HostDisconnected`)
- `app/src/lib.rs:1202` — singleton registration at app startup

### Command executor framework
- `app/src/terminal/model/session/command_executor.rs` — `CommandExecutor` trait (`execute_command`, `supports_parallel_command_execution`), `new_command_executor_for_session` dispatch
- `app/src/terminal/model/session.rs (199-309)` — `Sessions::initialize_bootstrapped_session()` creates the command executor for each session

### Existing SSH executor (being replaced)
- `app/src/terminal/model/session/command_executor/remote_command_executor.rs` — `RemoteCommandExecutor` opens a one-off SSH session per command via `ControlMaster`/`ControlPath`. Limited by `MaxSessions`, does not support parallel execution.

## 3. Current State

SSH completions currently use `RemoteCommandExecutor`, which forks a new `ssh` process for every generator command (e.g. `compgen -c`, `ls`). Each invocation opens a new channel on the ControlMaster SSH connection. This has two problems:

- **MaxSessions limit**: Many SSH servers default `MaxSessions` to 10. When multiple generators fire in parallel, they can exceed this limit and get `channel: open failed` errors. To avoid this, `RemoteCommandExecutor` returns `false` from `supports_parallel_command_execution()`, serializing all generator commands and making completions slow.
- **Per-command overhead**: Each command requires SSH channel setup/teardown. Even with multiplexing, the per-command latency adds up across the dozens of generators that fire during a typical completion cycle.

The remote server architecture (proto, `ServerModel`, `RemoteServerClient`, `RemoteServerManager`) already exists. The `remote_server` binary runs on the remote host as a long-lived process, communicating with the client over a single SSH connection via length-delimited protobuf. The manager tracks per-session state (`Connecting` → `Initializing` → `Connected` → `Disconnected`) and emits lifecycle events. What's missing is the `RunCommand` flow (proto messages, server dispatch, client API) and the `CommandExecutor` implementation that plugs into the completions pipeline.

## 4. Proposed Changes

### 4.1. Proto changes

`RunCommandRequest` (field 7 on `ClientMessage.oneof`):
- `string command` — the shell command to execute.
- `optional string working_directory` — cwd for the command. If absent, uses the server's default.
- `map<string, string> environment_variables` — env vars applied natively via `cmd.envs(...)`, not baked into the command string.
- `uint64 session_id` — routes the command to the correct per-session executor.

`RunCommandResponse` (field 8 on `ServerMessage.oneof`):
- `bytes stdout` / `bytes stderr` — raw output. `bytes` to avoid UTF-8 validity assumptions.
- `optional int32 exit_code` — absent when the process is killed by a signal (Unix).

### 4.2. Server-side: per-session executors in `ServerModel`

`ServerModel` maintains a `HashMap<SessionId, Arc<LocalCommandExecutor>>` (`executors`) - every session must be registered via `SessionBootstrapped` before it can run commands.

**SessionBootstrapped handler**: When the client sends `SessionBootstrapped` with `session_id`, `shell_type`, and optionally `shell_path`, the server parses the shell type and creates a `LocalCommandExecutor` with the provided `shell_path` (or falls back to the bare shell name if absent). The executor is inserted into `executors` keyed by the session ID. If the shell type is unknown, the handler logs an error and returns early (this is a notification — no response is sent).

**Repeated SessionBootstrapped**: If `SessionBootstrapped` is sent again for the same `session_id`, the new executor overwrites the old one (last-writer-wins). In-flight commands on the old executor complete or fail naturally since they hold their own `Arc<LocalCommandExecutor>`. A warning is logged when this happens.

**RunCommand handler**: Looks up the executor by `session_id` from the request. If the session is unregistered, returns `ErrorResponse` with `INVALID_REQUEST` — this is a bug (every session goes through `SessionBootstrapped` before sending commands).
- **Future**: delegates to `LocalCommandExecutor::execute_local_command()`.
- **`on_resolve`** (main thread): removes the entry from `in_progress`. If `Ok(output)`, wraps in `RunCommandResponse` and sends via `response_tx`. If `Err(e)`, sends `ErrorResponse { code: INTERNAL, message }`.
- **`on_abort`** (main thread): removes the entry from `in_progress` and logs the cancellation. No response is sent — `Abort` is fire-and-forget.
- **Abort handling**: When the client sends `Abort`, `handle_message` removes the `SpawnedFutureHandle` from `in_progress` and calls `handle.abort()`. The framework aborts the background future (dropping it kills the child process via `kill_on_drop`) and invokes the `on_abort` callback.

This same `spawn_abortable` + `in_progress` pattern is used for all async request handlers (e.g. `NavigatedToDirectory`), so the `Abort` handler works generically for any in-progress request.

**Why `LocalCommandExecutor`:** Delegating to it gives us shell config flags (`--norc` for bash, `-f` for zsh, `--no-config` for fish) that suppress sourcing `.bashrc`/`.zshrc`, which is correct for generator commands. It also gives us process-group tracking and `kill_on_drop` via the `command` crate's `Command` wrapper.


### 4.3. Client-side: `RemoteServerClient.run_command()`

```rust path=null start=null
pub async fn run_command(
    &self,
    session_id: SessionId,
    command: String,
    working_directory: Option<String>,
    environment_variables: HashMap<String, String>,
) -> Result<RunCommandResponse, ClientError>
```

`run_command()` accepts `SessionId` to route to the correct per-session executor. It uses the same request/response correlation pattern as `initialize()`: generate a `RequestId`, construct a `ClientMessage`, call `send_request`, match the response variant. On `ErrorResponse`, returns `ClientError::ServerError`. On timeout, sends `Abort` and returns `ClientError::Timeout`. Session registration is handled separately by `notify_session_bootstrapped()`, which sends a fire-and-forget `SessionBootstrapped` notification (no response expected).

### 4.4. `RemoteServerCommandExecutor`

A `CommandExecutor` implementation in `app/src/terminal/model/session/command_executor/remote_server_executor.rs`.

**Structure:**
```rust path=null start=null
#[derive(Debug)]
pub struct RemoteServerCommandExecutor {
    session_id: SessionId,
    client: RwLock<Option<Arc<RemoteServerClient>>>,
}
```

The executor holds its `SessionId` and a `parking_lot::RwLock<Option<Arc<RemoteServerClient>>>` that starts as `None` and is updated from the main thread when the connection state changes.

**Why `RwLock<Option<>>`:** The manager stores each session's client as `Arc<RemoteServerClient>` inside `RemoteSessionState`. `client_for_session()` returns `Option<&Arc<RemoteServerClient>>`, so the main thread can clone the `Arc` out. Cloning gives a second handle to the same underlying channels (`outbound_tx`, `pending_requests`), fully functional from any thread. `RwLock<Option<>>` lets the main thread write the `Arc` on connect and clear it on disconnect, while background threads read it without needing `AppContext`.

Unlike `OnceLock`, `RwLock<Option<>>` supports clearing the client on disconnect and replacing it on reconnect. The read lock is uncontended in practice — the writer (main thread, on connect/disconnect) and readers (background threads, on `execute_command`) almost never overlap, and the read-side operation is just an `Arc::clone`.

**`set_client(client: Arc<RemoteServerClient>)`** — writes `Some(client)` into the `RwLock`. Called from the main thread on connect.

**`clear_client()`** — writes `None` into the `RwLock`. Called from the main thread on disconnect.

**`CommandExecutor` impl:**
- `execute_command(command, shell, cwd, env_vars, options)`: reads `self.client.read().clone()`. If `None` (server not connected yet or disconnected), returns empty `CommandOutput` with `Failure` status and logs a warning. Otherwise, calls `client.run_command(self.session_id, command, cwd, env_vars)`. Timeout and abort are handled by `send_request` internally using the shared `REQUEST_TIMEOUT`. Translates `RunCommandResponse` → `CommandOutput`.
- `supports_parallel_command_execution()` → `true`. The remote server multiplexes commands over a single SSH connection (unlike `RemoteCommandExecutor` which opens a new SSH session per command and is limited by `MaxSessions`).

### 4.5. Wiring the executor to `RemoteServerManager`

The executor does **not** spawn or manage the server. It gets its `RemoteServerClient` from the `RemoteServerManager` via two mechanisms set up during executor creation in `new_command_executor_for_local_tty_session`.

The manager is **per-session**: each SSH session gets its own `RemoteServerClient` and SSH connection. Events are session-scoped (`SessionConnected { session_id, host_id }`, `SessionDisconnected { session_id, host_id }`). The host-level tracking (`host_to_sessions`) exists only to deduplicate host-scoped models (e.g. `RepoMetadataModel`), not connections.

**A. Eager check at creation time**: If this session's server is already connected (the executor is created after the manager has already completed the handshake), set the client immediately:

```rust path=null start=null
let executor = Arc::new(RemoteServerCommandExecutor::new(session_id));
let remote_server_manager = RemoteServerManager::handle(ctx);
let executor_clone = executor.clone();
remote_server_manager.read(ctx, |manager, _ctx| {
    if let Some(client) = manager.client_for_session(session_id) {
        executor_clone.set_client(Arc::clone(client));
    }
});
```

**B. Event subscription for connection changes**: Subscribe to `RemoteServerManagerEvent` so the executor tracks the connection lifecycle. The subscription receives events for *all* sessions, so it filters on `session_id`:

```rust path=null start=null
let executor_clone = executor.clone();
let remote_server_manager_clone = remote_server_manager.clone();
ctx.subscribe_to_model(&remote_server_manager, move |_sessions, event, ctx| {
    match event {
        RemoteServerManagerEvent::SessionConnected { session_id: sid, .. } if *sid == session_id => {
            remote_server_manager_clone.read(ctx, |manager, _ctx| {
                if let Some(client) = manager.client_for_session(session_id) {
                    executor_clone.set_client(Arc::clone(client));
                }
            });
        }
        RemoteServerManagerEvent::SessionDisconnected { session_id: sid, .. } if *sid == session_id => {
            executor_clone.clear_client();
        }
        _ => {}
    }
});
```

Both paths are needed: (A) handles the case where the server is already connected for this session, (B) handles connection and disconnection events after creation. On disconnect, the client is cleared so `execute_command` returns clean "not connected" results. On reconnect, the subscription fires `SessionConnected` again and sets the new client.

### 4.6. Dispatch ordering in `new_command_executor_for_local_tty_session`

The `SshRemoteServer` branch is added as the **first** check in `new_command_executor_for_local_tty_session`, before all other SSH executor paths:

```text path=null start=null
1. SshRemoteServer + IsLegacySSHSession::Yes     → RemoteServerCommandExecutor  [NEW]
2. SSHTmuxWrapper + tmux_control_mode             → TmuxCommandExecutor
3. SessionType::Local (various)                   → LocalCommandExecutor / MSYS2 / WSL
4. WarpifiedRemote + legacy SSH + !InBandForSSH   → RemoteCommandExecutor
5. default                                        → InBandCommandExecutor / NoOp
```

**Why first:** When the remote server is available, it is strictly better than every other SSH command execution method:
- **vs `RemoteCommandExecutor`** (branch 4): opens a new SSH session per command, limited by the host's `MaxSessions` sshd setting. The remote server multiplexes all commands over a single persistent connection.
- **vs `InBandCommandExecutor`** (branch 5): injects commands into the user's visible terminal session. Slow, fragile, and pollutes terminal output.
- **vs `TmuxCommandExecutor`** (branch 2): wraps the session in tmux for generator access. The remote server provides the same capability without the tmux dependency.

By checking `SshRemoteServer` first, we ensure that when the flag is on, the persistent multiplexed connection is always preferred. When the flag is off, the existing executor dispatch is unchanged.

The full executor creation (including the eager check and subscription from §4.5) happens in this branch.

## 5. End-to-End Flow

### Precondition

`RemoteServerManager.connect_session()` was called by a separate flow (out of scope for this spec). The manager has completed: server startup → `Arc<RemoteServerClient>` creation (state: `Initializing`) → initialize handshake → `mark_session_connected` (state: `Connected`). The session's `RemoteSessionState` is `Connected { client: Arc<RemoteServerClient>, host_id }`.

### Executor creation

1. SSH session bootstraps. `Sessions::initialize_bootstrapped_session()` creates a `RemoteServerCommandExecutor` with `session_id`.
2. Eager check: reads `manager.client_for_session(session_id)` — if this session is `Connected`, clones the `Arc<RemoteServerClient>` and calls `executor.set_client(client)`.
3. Subscribes to `RemoteServerManagerEvent` for future `SessionConnected`/`SessionDisconnected` events, filtering on matching `session_id`.

### RunCommand flow

Detailed steps:
1. Generator calls `executor.execute_command("compgen -c", shell, cwd, env_vars, opts)`.
2. `RemoteServerCommandExecutor` reads `self.client.read().clone()`. If `Some`, uses the `RemoteServerClient`.
3. Calls `client.run_command(self.session_id, command, cwd, env_vars)`.
4. `run_command` calls `send_request(request_id, msg)`, which registers a oneshot in `pending_requests`, sends the `ClientMessage` via `outbound_tx`, and awaits the response with the standard `REQUEST_TIMEOUT`. If the timeout fires, `send_request` removes the `pending_requests` entry and sends `Abort` to the server.
5. **Client writer task** pulls message from channel, calls `write_client_message` → `[4-byte LE length][protobuf bytes]` over SSH stdin.
6. **Server stdin reader task** decodes `ClientMessage`, dispatches to `ServerModel::handle_message` via `ModelSpawner`.
7. `handle_message` matches `RunCommand`: delegates to `LocalCommandExecutor::execute_local_command()` via `ctx.spawn_abortable`. The returned `SpawnedFutureHandle` is stored in `in_progress` so the client can cancel it via `Abort`.
8. `on_resolve` callback receives `Output`, removes the entry from `in_progress`, constructs `RunCommandResponse { stdout, stderr, exit_code }`, sends via `response_tx.try_send(response)`.
9. **Server stdout writer task** encodes `ServerMessage` → `[4-byte LE length][protobuf bytes]` over SSH stdout.
10. **Client reader task** decodes `ServerMessage`, looks up `request_id` in `pending_requests`, resolves the oneshot.
11. `run_command()` receives `RunCommandResponse`, returns to executor.
12. Executor translates: `exit_code == Some(0)` → `CommandExitStatus::Success`, else `Failure`. Wraps in `CommandOutput { stdout, stderr, status, exit_code }`.

### Server connects after executor creation

1. Executor is created, but `self.client.read()` returns `None` because the manager hasn't connected this session yet.
2. Completions calls to `execute_command` return empty `Failure` results (logged).
3. Manager completes connection for this session, emits `SessionConnected { session_id, host_id }`.
4. Subscription handler fires on main thread (matches on `session_id`) → clones `Arc<RemoteServerClient>` from manager → `executor.set_client(client)`.
5. Subsequent `execute_command` calls read the client from the `RwLock`.

### Disconnection

1. SSH dies or server crashes for this session → `RemoteServerClient` reader task hits EOF → clears `pending_requests` (in-flight calls get `ResponseChannelClosed`).
2. Manager's subscription on the client fires `ClientEvent::Disconnected` → `mark_session_disconnected(session_id)` → emits `SessionDisconnected { session_id, host_id }`.
3. Subscription handler fires on the executor (matches on `session_id`) → `executor.clear_client()` → writes `None` into the `RwLock`.
4. Subsequent `execute_command` calls read `None` → return clean empty `Failure` result (logged).

### Reconnection

1. Manager reconnects this session (trigger out of scope for this spec) — creates a new `RemoteServerClient` for this session.
2. Manager emits `SessionConnected { session_id, .. }` → subscription handler fires → clones new `Arc<RemoteServerClient>` from manager → `executor.set_client(new_client)`.
3. Subsequent `execute_command` calls read the new client from the `RwLock`. Completions resume.
