# TECH.md — CLI agent rich input visibility notifications

Issue: https://github.com/warpdotdev/warp/issues/15977  
Product spec: `specs/GH15977/product.md`  
Inspected commit: `82b52132c62705d8261921a532cb258925c774ec`

## Problem

Warp tracks when CLI agent rich input is open, but that state currently stays inside the Warp UI model. Third-party CLI agents receive host capability hints through environment variables and can emit agent-to-Warp status through OSC 777, but they do not have a Warp-to-agent signal for rich input visibility. As a result, an agent cannot hide its own terminal prompt bar when Warp rich input opens.

## Relevant code

- `crates/warp_core/src/cli_agent_protocol.rs` — current CLI-agent protocol constants, including `WARP_CLI_AGENT_PROTOCOL_VERSION_ENV`, `WARP_CLIENT_VERSION_ENV`, and the OSC 777 `CLIAgentNotification` schema.
- `app/src/terminal/cli_agent_sessions/mod.rs` — pane-scoped CLI agent sessions, `CLIAgentInputState`, `CLIAgentSessionsModel::open_input`, `close_input`, and `CLIAgentSessionsModelEvent::InputSessionChanged`.
- `app/src/terminal/view/use_agent_footer/mod.rs` — user-facing rich input open, close, submit, auto-dismiss, and auto-toggle entry points.
- `app/src/terminal/input/cli_agent.rs` and `app/src/terminal/input.rs` — rich input rendering and behavior once the session model reports the input as open.
- `app/src/terminal/local_tty/terminal_manager.rs` — creates the terminal model/surface before PTY spawn, builds `PtyOptions`, and passes pane-specific `env_vars` to the local TTY.
- `crates/warp_terminal/src/local_tty/unix.rs` — Unix PTY command environment setup; currently advertises CLI-agent protocol support when `FeatureFlag::HOANotifications` is enabled.
- `crates/warp_terminal/src/local_tty/windows/environment.rs` — Windows PTY environment setup and WSL allowlist handling for Warp environment variables.
- `crates/ipc/src/lib.rs` and `crates/ipc/src/native.rs` — existing native local IPC abstraction over Unix-domain sockets and Windows named pipes.
- `app/src/local_control/mod.rs` and `app/src/remote_server/unix/mod.rs` — examples of owner-only Unix socket binding, stale socket cleanup, nonblocking accept loops, and safe local-transport error handling.

## Current state

### Agent-to-Warp notifications

CLI agent status events use OSC 777 with the `warp://cli-agent` sentinel. The schema lives in `warp_core::cli_agent_protocol::CLIAgentNotification` and includes fields such as `event`, `session_id`, `query`, `response`, and `summary`. The listener in `app/src/terminal/cli_agent_sessions/listener/mod.rs` parses those notifications and updates `CLIAgentSessionsModel`.

This is the opposite direction from the requested feature: the agent writes OSC notifications to the PTY, and Warp reads them.

### Warp-to-agent host capabilities

Local PTY startup advertises Warp host capabilities through environment variables. On Unix and Windows, `WARP_CLIENT_VERSION` is always set when possible, and `WARP_CLI_AGENT_PROTOCOL_VERSION` is set when `FeatureFlag::HOANotifications` is enabled.

The issue's proposed capability fits this pattern: create a pane-scoped endpoint before the shell process starts, then advertise that endpoint in the environment inherited by agents launched from that pane.

### Rich input state

`CLIAgentSessionsModel` is the authoritative pane-scoped rich input state:

- `CLIAgentInputState::Closed`
- `CLIAgentInputState::Open { entrypoint, previous_input_config, previous_was_lock_set_with_empty_buffer }`

`open_input` and `close_input` update that state and emit `CLIAgentSessionsModelEvent::InputSessionChanged` with both previous and new state. Existing UI code already reacts to this event to update rendering, focus, input config, and footer state.

## Proposed changes

### 1. Add a Warp-to-agent control protocol type

File: `crates/warp_core/src/cli_agent_protocol.rs`

Add protocol constants and a serializable event type next to the existing OSC 777 definitions:

- `CLI_AGENT_CONTROL_PROTOCOL_VERSION: u32 = 1`
- `WARP_CLI_AGENT_CONTROL_SOCKET_ENV: &str = "WARP_CLI_AGENT_CONTROL_SOCKET"` initially, unless product review chooses a transport-neutral name.
- `CLIAgentControlEvent { v: u32, event: String, active: bool }`

For v1, the only event is:

- `event: "rich_input"`
- `active: true` when Warp rich input is open
- `active: false` when Warp rich input is closed

Keep this separate from `CLIAgentNotification` so agent-to-Warp OSC status and Warp-to-agent control events can evolve independently.

### 2. Add a pane-scoped control endpoint

New app module, recommended path: `app/src/terminal/cli_agent_sessions/control_endpoint.rs`

Responsibilities:

- Bind one local endpoint per terminal pane before the PTY is spawned.
- Track the current rich input state, initialized to closed.
- Accept multiple clients.
- On accept, write the current state as one JSON line.
- On state changes, broadcast the new JSON line to all connected clients.
- Drop clients whose write fails.
- Close the listener and clients when the terminal manager is dropped.

Unix implementation:

- Bind a Unix-domain socket under a per-user Warp runtime/cache directory, not a world-writable arbitrary path.
- Use a short filename derived from the terminal session UUID or generated session id, such as `ca-<short-id>.sock`, to stay below macOS's 104-byte socket path limit.
- Remove only the exact stale socket path generated for this pane before binding.
- Set permissions to `0600` after bind.

Windows implementation:

- Bind a named pipe/local socket address using the existing `ipc` crate's native transport when possible, or the same `interprocess` named-pipe primitive it wraps.
- Use a per-session, per-pane address such as `Warp<channel>_cli_agent_control_<short-id>`.
- The environment variable value is the address string the agent should connect to.

Failure behavior:

- If endpoint setup fails, report internally at warn/error level, do not set the environment variable, and continue spawning the terminal. The feature must fail open for the terminal UX and fail closed for the capability advertisement.

### 3. Inject the endpoint address into local PTY environment

File: `app/src/terminal/local_tty/terminal_manager.rs`

`TerminalManager::create_model_with_manager` creates the terminal model and surface before shell determination and PTY spawn. Add endpoint construction during that setup, before `on_shell_determined` consumes `env_vars`.

Recommended shape:

1. Create the control endpoint after the terminal model exists and before spawning the PTY.
2. If creation succeeds, insert the endpoint address into the pane's `env_vars` using `WARP_CLI_AGENT_CONTROL_SOCKET_ENV`.
3. Retain the endpoint handle on `TerminalManager<S>` so it lives for the pane lifetime.
4. Pass a lightweight publisher handle through `TerminalSurfaceInit` or the post-wire callback so `TerminalView` can publish rich input state changes without owning transport details.

Do not add the variable in `crates/warp_terminal/src/local_tty/unix.rs` or `windows/environment.rs` unconditionally, because the value must be pane-specific and only present after endpoint creation succeeds. Those files should only need to preserve existing behavior when `env_vars` are applied last.

### 4. Publish rich input state changes from the session model

Files:

- `app/src/terminal/cli_agent_sessions/mod.rs`
- `app/src/terminal/view.rs` or the terminal surface wiring that owns the control publisher

Subscribe to `CLIAgentSessionsModelEvent` for the current `terminal_view_id`. Publish to the control endpoint when:

- `InputSessionChanged` transitions to `CLIAgentInputState::Open { .. }` → send `active: true`.
- `InputSessionChanged` transitions to `CLIAgentInputState::Closed` → send `active: false`.
- `Ended` fires while the last published state is active → best-effort send `active: false`.

Keep `CLIAgentSessionsModel` as the source of truth. Do not duplicate rich input open/close sends in individual UI action handlers; otherwise manual close, auto-toggle, submit close, shared-session sync, and future close paths can diverge.

### 5. Lifecycle and cleanup

- Endpoint creation happens before PTY spawn so the shell and all descendant CLI agent processes inherit the environment variable.
- Endpoint teardown happens when the terminal pane/manager is dropped. Unix implementations remove the socket path they created.
- Pane close does not need to send a final JSON message if clients will observe EOF, but it should send `active: false` before teardown when practical and cheap.
- Session replacement should keep the same pane endpoint but reset current state to inactive unless the replacement immediately opens rich input.

### 6. Cross-platform behavior

Unix and macOS:

- Advertise the Unix-domain socket path.
- Enforce path-length discipline and owner-only permissions.

Windows:

- Advertise the named-pipe/local-socket address.
- Do not rely on Unix path semantics even if the environment variable retains the word `SOCKET`.

WSL:

- Do not add `WARP_CLI_AGENT_CONTROL_SOCKET_ENV` to `WSLENV` for v1. A WSL process generally cannot connect to a Windows named pipe using the same address, and advertising an unusable endpoint would increase agent-side error noise.

SSH and remote sessions:

- Do not forward the endpoint in v1. Local child processes in the pane can use it; remote processes keep existing behavior unless a later warpified remote protocol forwards or proxies control messages.

Docker sandbox:

- Treat as unsupported in v1 unless the endpoint can be safely mounted/proxied into the sandbox. If the host-side `sbx` process inherits the variable but the in-container agent cannot connect, agents should degrade by ignoring connection failure. Prefer omitting the variable for sandbox sessions if the implementation can distinguish them before env injection.

## Data flow

1. Terminal pane is created.
2. Warp creates a pane-scoped control endpoint and inserts its address into the pane's PTY environment.
3. User launches a CLI agent in that pane.
4. The CLI agent reads the environment variable and connects to the endpoint.
5. Warp writes the current state immediately, usually `{"v":1,"event":"rich_input","active":false}\n`.
6. User opens Warp rich input.
7. `CLIAgentSessionsModel::open_input` emits `InputSessionChanged`.
8. The control publisher broadcasts `{"v":1,"event":"rich_input","active":true}\n`.
9. User closes or submits from Warp rich input.
10. `CLIAgentSessionsModel::close_input` emits `InputSessionChanged`.
11. The control publisher broadcasts `{"v":1,"event":"rich_input","active":false}\n`.
12. Pane closes; endpoint closes and clients observe EOF.

## Risks and mitigations

- **Socket path too long on macOS**: bind under a short per-user runtime/cache directory and keep filenames short. Add a unit test for generated Unix address length.
- **Endpoint leaks or stale sockets**: retain ownership on `TerminalManager`, remove only the generated path on drop, and remove stale generated paths before bind.
- **Blocking UI on slow clients**: writes must happen off the UI thread or through nonblocking async tasks. Drop slow or failed clients instead of back-pressuring rich input.
- **Same-user local process can observe rich input state**: the channel carries only a boolean visibility bit and no prompt content. Use owner-only permissions on Unix and per-session named pipes on Windows.
- **Advertised but unusable transport in WSL/sandbox/remote**: omit the env var when known unsupported. Agents must treat connection failure as a no-op.
- **Divergent open/close paths**: publish from `CLIAgentSessionsModelEvent::InputSessionChanged`, not from individual UI handlers.
- **Version coupling with OSC 777**: use separate constants/types so a future breaking change in one direction does not require changing the other.

## Testing and validation

### Unit tests

- `CLIAgentControlEvent` serializes to a single JSON object with `v`, `event`, and `active`.
- Endpoint writes newline-terminated JSON Lines.
- New client receives the current state immediately on connect.
- Multiple clients receive the same state update.
- Failed/disconnected clients are dropped and remaining clients still receive updates.
- Generated Unix socket path stays under the macOS path-length limit.
- Unix socket permissions are owner-only where the platform exposes them.

### Model/surface tests

- `InputSessionChanged` closed-to-open publishes `active=true`.
- `InputSessionChanged` open-to-closed publishes `active=false`.
- Repeated open while already open does not duplicate messages.
- Repeated close while already closed does not duplicate messages.
- `Ended` after an active state publishes or records inactive before teardown.

### Environment tests

- Local Unix PTY startup includes `WARP_CLI_AGENT_CONTROL_SOCKET` only when endpoint creation succeeds.
- Windows PTY startup includes the named-pipe address only when endpoint creation succeeds.
- WSL environment allowlist does not include the control variable in v1.
- Endpoint setup failure still allows PTY spawn and rich input use.

### Regression tests

- Existing CLI agent rich input tests continue to pass for open, close, submit, auto-dismiss, auto-toggle, and image attachment flows.
- Existing `WARP_CLI_AGENT_PROTOCOL_VERSION` behavior remains unchanged.
- Existing OSC 777 listener behavior remains unchanged.

### Manual validation

- Start Warp locally, open a pane, and confirm the environment variable is present for local shell sessions where the endpoint is supported.
- Connect a minimal test client to the endpoint and verify initial false, open true, close false.
- Connect two clients and verify both receive broadcasts.
- Kill one client and verify the other continues receiving updates.
- Validate with a cooperating CLI agent build that hides its native prompt bar when `active=true` and restores it when `active=false`.

## Follow-ups

- Choose a transport-neutral environment variable name if product review decides `SOCKET` is too Unix-specific.
- Consider forwarding/proxying the visibility channel for warpified SSH sessions.
- Consider a WSL bridge if a Windows-hosted named pipe can be exposed through a WSL-friendly transport.
- Document the control channel for third-party CLI agent authors once the protocol is stable.
