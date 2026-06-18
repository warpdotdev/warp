# Remote Server SSH: Technical Spec

## Problem

The current SSH wrapper flow creates a `RemoteCommandExecutor` that runs every generator/completion command by opening a new SSH channel through a ControlMaster socket. This is unreliable and stateless. We want to replace it with a persistent remote server binary (`~/.warp/remote-server/oz`) on the remote machine that communicates over stdin/stdout using length-prefixed protobuf messages. The binary is the Oz CLI, installed from the `/download/cli` endpoint if not already present.

The challenge is that this introduces two independent async conditions that must both complete before the session is ready: (1) the shell `Bootstrapped` DCS hook, and (2) the remote server `InitializeResponse`. Today the bootstrap path is fully synchronous — `Bootstrapped` DCS immediately triggers `initialize_bootstrapped_session()`. We need to gate that call on both conditions without breaking non-SSH or flag-off flows.

## Relevant code

- `warp_core/src/features.rs` — `FeatureFlag` enum and `DOGFOOD_FLAGS` array
- `app/src/terminal/model/terminal_model.rs:2808-2846` — `bootstrapped()` handler that takes `pending_session_info` and emits `HandlerEvent::Bootstrapped`
- `app/src/terminal/model/terminal_model.rs:2848-2860` — `pre_interactive_ssh_session()` and `ssh()` handlers
- `app/src/terminal/model/terminal_model.rs:2862-2908` — `init_shell()` handler
- `app/src/terminal/model_events.rs:89-110` — `ModelEventDispatcher` handler for `HandlerEvent::Bootstrapped` that synchronously calls `sessions.initialize_bootstrapped_session()`
- `app/src/terminal/model/session.rs:220-332` — `Sessions::initialize_bootstrapped_session()` which creates the command executor and emits `SessionBootstrapped`
- `app/src/terminal/model/session.rs:410-414` — `IsLegacySSHSession` enum
- `app/src/terminal/model/session.rs:448-542` — `SessionInfo` struct and `create_pending()`
- `app/src/terminal/model/session/command_executor.rs:190-296` — executor selection logic in `new_command_executor_for_local_tty_session()`
- `app/src/terminal/model/session/command_executor/remote_command_executor.rs` — current `RemoteCommandExecutor` using ControlMaster
- `app/src/terminal/prompt_render_helper.rs:231-254` — `prompt_working_dir()` and `bootstrapping_shell_message()` that drive the status text
- `app/src/terminal/view.rs:10255-10267` — `ModelEvent::PreInteractiveSSHSession` (no-op) and `ModelEvent::SSH` handlers
- `app/src/terminal/view.rs:10860-11024` — `handle_session_bootstrapped()`
- `remote_server/proto/remote_server.proto` — protobuf definitions for `Initialize`/`InitializeResponse`
- `remote_server/src/lib.rs` — generated proto bindings
- `app/assets/bundled/bootstrap/bash_body.sh` and `zsh_body.sh` — shell-side SSH wrapper functions

## Current state

### DCS hook sequence for SSH wrapper flow

1. **`PreInteractiveSSHSession`** — no-op marker
2. **`SSH`** — carries `socket_path` (ControlMaster) and `remote_shell`. Stored in `pending_legacy_ssh_session`.
3. **`InitShell`** — creates `SessionInfo::create_pending()`, consuming `pending_legacy_ssh_session` to populate `IsLegacySSHSession::Yes { socket_path }`. Calls `reinit_shell()` to reset the block list. Warp input becomes visible with "Starting shell...".
4. **`Bootstrapped`** — `terminal_model.bootstrapped()` merges pending session info, emits `HandlerEvent::Bootstrapped`. The `ModelEventDispatcher` synchronously calls `sessions.initialize_bootstrapped_session()`, which creates the `RemoteCommandExecutor` from the socket path, stores the session, and emits `SessionBootstrapped`. The view shows the normal prompt.

### Key constraint

`initialize_bootstrapped_session()` is called synchronously inside the `ModelEventDispatcher` event handler for `HandlerEvent::Bootstrapped` (model_events.rs:98-106). This is the only place session initialization happens. All downstream logic (view, history, telemetry) flows from the `SessionBootstrapped` event it emits.

## Proposed changes

### 1. New feature flag

Add `RemoteServerSSH` to the `FeatureFlag` enum in `warp_core/src/features.rs`. Add it to `DOGFOOD_FLAGS` for initial rollout.

### 2. Remote server setup state machine

Create a new module `app/src/terminal/ssh/remote_server_setup.rs` (or similar location alongside the existing `app/src/terminal/ssh/` module) that encapsulates the install → launch → initialize flow.

```
enum RemoteServerSetupState {
    /// Checking if the binary exists on remote.
    Checking,
    /// Downloading and installing the binary.
    Installing { progress_percent: Option<u8> },
    /// Binary is launched, waiting for InitializeResponse.
    Initializing,
    /// Handshake complete. Ready.
    Ready,
    /// Something failed. Fall back to ControlMaster.
    Failed { error: String },
}
```

The setup runs as an async task, using the existing SSH ControlMaster socket (from `IsLegacySSHSession::Yes { socket_path }`) to execute remote commands. The steps are:

1. **Check**: `ssh -o ControlPath={socket} placeholder@placeholder 'test -x ~/.warp/remote-server/oz && ~/.warp/remote-server/oz --version'`
2. **Install** (if check fails): pipe the following script into `bash -s` over the control socket SSH connection (mirroring how `RemoteCommandExecutor` runs commands today):

   ```sh
   set -e
   arch=$(uname -m)
   case "$arch" in
     x86_64)  pkg=oz-linux-x86_64.tar.gz ;;
     aarch64|arm64) pkg=oz-linux-aarch64.tar.gz ;;
     *) echo "unsupported arch: $arch" >&2; exit 2 ;;
   esac
   mkdir -p "$HOME/.warp/remote-server"
   curl -fSL "$WARP_GET_URL?package=$pkg" -o "$HOME/.warp/remote-server/oz.tar.gz"
   tar -xzf "$HOME/.warp/remote-server/oz.tar.gz" -C "$HOME/.warp/remote-server"
   chmod +x "$HOME/.warp/remote-server/oz"
   ```

   `$WARP_GET_URL` is substituted at runtime from the configured server root URL (`SERVER_ROOT_URL` env var or its compiled-in default), pointing at `/download/cli`. Exit code 2 is mapped to `ErrorReason::UnsupportedPlatform`. The script is shipped as a constant `&str` in the install module. Parse `curl` stderr for download progress if available.
3. **Launch**: `ssh -o ControlPath={socket} placeholder@placeholder '~/.warp/remote-server/oz'` — keep the SSH channel open, forwarding stdin/stdout.
4. **Initialize**: Send a `ClientMessage { request_id, initialize: Initialize {} }` (length-prefixed protobuf) to the process's stdin. Read a `ServerMessage { initialize_response }` from stdout. Timeout after 10 seconds.

The async task communicates state changes back to the UI via an event channel.

### 3. New event: `RemoteServerReady`

Add a new `Event` variant:

```
Event::RemoteServerReady {
    session_id: SessionId,
    result: Result<(), RemoteServerSetupError>,
}
```

This is sent by the async setup task when it completes (success or failure). The `ModelEventDispatcher` receives this alongside the existing `HandlerEvent::Bootstrapped`.

### 4. Gate session initialization on both conditions

This is the core change. In `ModelEventDispatcher` (model_events.rs), when the feature flag is enabled and the session is a legacy SSH session:

- When `HandlerEvent::Bootstrapped` arrives: store the `BootstrappedEvent` payload in a new field `pending_bootstrapped_event: Option<BootstrappedEvent>` on `ModelEventDispatcher`. Do NOT call `initialize_bootstrapped_session()` yet.
- When `Event::RemoteServerReady` arrives: store the result.
- After either event, check if both are present. If so, call `initialize_bootstrapped_session()` as normal.
- If the remote server setup failed, call `initialize_bootstrapped_session()` anyway (fallback to ControlMaster executor).

When the flag is disabled, or the session is not a legacy SSH session, the existing synchronous path is unchanged.

### 5. Dynamic bootstrapping message

Extend `bootstrapping_shell_message()` in `prompt_render_helper.rs` to show stage-specific messages. The `Sessions` model (or a new model) needs to expose the current `RemoteServerSetupState` for the pending session. The render helper checks this state:

- `RemoteServerSetupState::Checking` → "Starting shell..." (unchanged)
- `RemoteServerSetupState::Installing { progress_percent: Some(p) }` → "Installing Warp SSH tools... ({p}%)"
- `RemoteServerSetupState::Installing { progress_percent: None }` → "Installing Warp SSH tools..."
- `RemoteServerSetupState::Initializing` → "Initializing..."
- No remote server state (flag off, non-SSH) → existing behavior

The state is stored on `Sessions` keyed by `SessionId` and updated via events from the async setup task. The prompt re-renders on each state change via `ctx.notify()`.

### 6. Command executor selection

In `new_command_executor_for_local_tty_session()` (command_executor.rs:245-258), when the flag is enabled and the remote server is ready, create a new `RemoteServerCommandExecutor` instead of `RemoteCommandExecutor`. This new executor sends commands to the running remote server process via stdin/stdout protobuf messages instead of opening SSH channels.

For the initial iteration, the new executor can wrap the same interface but communicate through the persistent process. If the remote server setup failed, the existing `RemoteCommandExecutor` (ControlMaster) is used as fallback.

### 7. Cleanup on SSH exit

The remote server process is spawned as an SSH channel via ControlMaster. When the SSH session exits:
- The ControlMaster connection is torn down, which kills the remote process.
- The `Arc<Session>` holding the executor is dropped via Rust's RAII.
- No explicit cleanup is needed.

## End-to-end flow

1. User runs `ssh user@host`.
2. `PreInteractiveSSHSession` DCS → no-op.
3. `SSH` DCS → stores socket path in `pending_legacy_ssh_session`.
4. `InitShell` DCS → creates `SessionInfo::create_pending()` with `IsLegacySSHSession::Yes { socket_path }`. Warp input appears with "Starting shell...". **If `RemoteServerSSH` flag is enabled**: kicks off the async remote server setup task using the socket path from the pending session info.
5. Setup task transitions: Checking → Installing (if needed) → Initializing. Each state change updates the `Sessions` model and triggers a prompt re-render.
6. `Bootstrapped` DCS → `ModelEventDispatcher` stores the `BootstrappedEvent` instead of immediately initializing the session.
7. Setup task sends `Event::RemoteServerReady { session_id, Ok(()) }`.
8. `ModelEventDispatcher` sees both conditions met, calls `initialize_bootstrapped_session()` which creates the `RemoteServerCommandExecutor`.
9. `SessionBootstrapped` event fires. View transitions to normal prompt with working directory.

If `Bootstrapped` arrives after `RemoteServerReady`, step 6 triggers immediate initialization. The order doesn't matter.

## Risks and mitigations

**Risk: Setup task blocks on slow network.** The download could take a long time on a slow connection. Mitigation: the shell bootstrap runs in parallel, so only the delta matters. Show progress percentage to set expectations. Timeout the entire setup after a generous limit (e.g. 60 seconds) and fall back.

**Risk: ControlMaster socket gone before setup completes.** If the SSH session drops during setup, the task will fail. Mitigation: the task uses the existing error handling — any SSH command failure transitions to `Failed` state, and the session falls back.

**Risk: Remote host has no curl or wget.** Mitigation: try `curl` first, then `wget`. If neither exists, transition to `Failed` and fall back to ControlMaster.

**Risk: Race between `Bootstrapped` and `RemoteServerReady` events.** Mitigation: the `ModelEventDispatcher` stores whichever arrives first and processes initialization when both are present. This is a simple two-flag check, not a complex synchronization problem.

**Risk: Regression for non-SSH sessions.** Mitigation: the gating logic in `ModelEventDispatcher` only applies when the feature flag is enabled AND `is_legacy_ssh_session` is `Yes`. All other sessions use the existing synchronous path.

## Testing and validation

- **Unit tests**: Test `RemoteServerSetupState` transitions, `uname` output parsing, download URL construction.
- **Unit tests**: Test the two-condition gate in `ModelEventDispatcher` — verify `initialize_bootstrapped_session` is called only when both conditions are met, in both arrival orders.
- **Unit tests**: Test fallback behavior when setup fails — verify `RemoteCommandExecutor` is created instead of `RemoteServerCommandExecutor`.
- **Integration test**: SSH into a Docker container (using the existing SSH testing setup in the repo). Verify binary installation, launch, and Initialize handshake.
- **Manual testing**: SSH into fresh Linux x86_64 and aarch64 hosts. Verify prompt message transitions. SSH again to verify installation is skipped.
- **Feature flag off**: Verify no behavioral change with the flag disabled.

## Follow-ups

- **Version checking**: Compare installed binary version to client version and re-install on mismatch.
- **Auto-update**: Silently update the binary when a newer version is available.
- **Richer remote server capabilities**: File watching, codebase indexing, cached completions via the persistent process.
- **Replace ControlMaster entirely**: Once the remote server is stable, remove the ControlMaster-based `RemoteCommandExecutor` code path.
- **Progress reporting**: Improve download progress by parsing curl/wget output more reliably.
