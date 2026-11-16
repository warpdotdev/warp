# Remote Server SSH: Binary Installation and Initialization

## Summary

Replace the current SSH wrapper's ControlMaster-based `RemoteCommandExecutor` with a persistent remote server binary (`warp-remote-server`) that runs on the remote machine. After the remote shell sends `InitShell`, Warp checks for the binary at `~/.warp/warp-remote-server`, installs it if missing, launches it, and performs a protobuf Initialize handshake over stdin/stdout. The session is fully bootstrapped only after both the remote server is initialized and the shell `Bootstrapped` hook has been received.

This is gated behind a new feature flag.

## Problem

The current SSH wrapper flow relies on SSH ControlMaster sockets to execute commands on the remote machine. This approach has limitations:
- ControlMaster connections can be unreliable and produce hard-to-debug errors.
- Every generator/completion command opens a new SSH channel through the control socket.
- There is no persistent process on the remote side to maintain state, cache results, or provide richer capabilities.

A persistent remote binary enables future capabilities (file watching, indexing, richer completions) and provides a more reliable command execution channel.

## Goals

- Install the `warp-remote-server` binary on the remote machine automatically when it is not present.
- Detect the remote OS (Linux or macOS) and architecture (x86_64 or aarch64) to download the correct binary.
- Show clear, stage-specific status messages in the Warp input during installation and initialization.
- Perform a protobuf-based Initialize handshake with the remote binary before marking the session as ready.
- Gate the entire flow behind a feature flag so the existing ControlMaster flow remains the default.
- Require both remote server initialization and shell bootstrap completion before the session accepts input.

## Non-goals

- Windows remote hosts (Linux and macOS only for now).
- Replacing all `RemoteCommandExecutor` functionality with the remote server in this iteration.
- Auto-updating the remote binary when a newer version is available.
- Handling SSH connections that require interactive password entry for the binary installation step (assumes ControlMaster socket is already established).
- Supporting remote hosts without `curl` or `wget`.

## Figma

Figma: none provided.

## User experience

### Feature flag

The new flow is gated behind a feature flag (e.g. `RemoteServerSSH`). When the flag is disabled, the existing ControlMaster-based flow is used unchanged. When enabled, the new flow described below applies to all SSH wrapper sessions.

### Status messages

Throughout the flow, the Warp input prompt area displays a status message (bold, in the same style as "Starting shell..."). The messages are:

1. **"Starting shell..."** — shown immediately after `InitShell` (same as today).
2. **"Installing Warp SSH tools... (X%)"** — shown while the binary is being downloaded and installed on the remote machine. Replaces "Starting shell..." once installation begins. The percentage reflects download progress reported by `curl`/`wget`. If progress cannot be determined, show **"Installing Warp SSH tools..."** without a percentage.
3. **"Initializing..."** — shown after the binary is launched and the Initialize handshake is in progress.
4. Once the Initialize handshake succeeds AND the `Bootstrapped` hook is received (in either order), the prompt transitions to the normal working directory display.

### Installation flow (after `InitShell`)

After `InitShell` is received and the pending session info is created:

1. **Check for existing binary.** Run a command over the existing SSH ControlMaster socket to check if `~/.warp/warp-remote-server` exists and is executable on the remote machine (e.g. `test -x ~/.warp/warp-remote-server && ~/.warp/warp-remote-server --version`).

2. **If the binary is not present or not functional:**
   a. Detect the remote OS and architecture by running `uname -sm` over SSH and parsing the output:
      - OS: `Darwin` → macOS, `Linux` → Linux
      - Arch: `x86_64` → x86_64, `arm64`/`aarch64`/`armv8l` → aarch64
   b. Download the Oz CLI tarball from the Warp server's `/download/cli` endpoint, using the detected OS and architecture. The endpoint accepts query parameters `os` (`macos` or `linux`), `arch` (`x86_64` or `aarch64`), `package` (`tar`), and `channel` (matching the current client channel). The endpoint returns a 302 redirect to the releases CDN (e.g. `https://releases.warp.dev/{channel}/{version}/cli/{os}/{arch}/warp-{channel}-{os}-{arch}.tar.gz`). The download is performed on the remote machine using `curl -fL` (preferred) or `wget` (fallback) via the SSH ControlMaster socket.
   c. Extract the Oz CLI binary from the tarball to `~/.warp/warp-remote-server` and set executable permissions (`chmod 755`).
   d. During this process, the input prompt shows **"Installing Warp SSH tools... (X%)"** with download progress when available, or **"Installing Warp SSH tools..."** without percentage if progress reporting is unavailable.

3. **If the binary is already present and functional**, skip installation.

### Launch and initialization flow

After the binary is confirmed present:

1. Launch `~/.warp/warp-remote-server` on the remote machine over the SSH ControlMaster socket. The process's stdin/stdout are used for communication.
2. Send a `ClientMessage` containing an `Initialize` message (protobuf, length-prefixed as defined in `remote_server.proto`).
3. Wait for a `ServerMessage` containing an `InitializeResponse`.
4. During this phase, the input prompt shows **"Initializing..."**.

### Session readiness

The session is considered fully bootstrapped and ready to accept user input only when **both** of the following conditions are met:
- The remote server has responded with `InitializeResponse`.
- The shell `Bootstrapped` DCS hook has been received.

These two events may arrive in either order. The session must wait for both before transitioning to the fully bootstrapped state.

### Error handling

- **Installation failure (download fails, extraction fails, unsupported platform):** The input prompt should show an error message (e.g. "Failed to install Warp SSH tools"). The session should fall back to the existing ControlMaster-based `RemoteCommandExecutor` so the user can still use the SSH session with reduced functionality. Log the error for diagnostics.
- **Binary launch failure:** Same fallback behavior. Show a brief error message, then proceed with ControlMaster-based execution.
- **Initialize handshake timeout:** If no `InitializeResponse` is received within 10 seconds, fall back to ControlMaster-based execution with a logged warning.
- **Unsupported OS/arch from `uname`:** Fall back to ControlMaster-based execution. Log the unrecognized platform string.

### Exiting SSH

When the SSH session ends (user types `exit` or the connection drops), the remote server process should be terminated. No special cleanup of `~/.warp/warp-remote-server` is needed — the binary remains installed for future sessions.

## Success criteria

1. When the feature flag is enabled and a user SSHs into a Linux or macOS remote host that does not have the binary installed, the binary is automatically downloaded and installed at `~/.warp/warp-remote-server` without user intervention.
2. The correct binary variant is downloaded based on the remote host's OS and architecture (linux-x86_64, linux-aarch64, darwin-x86_64, darwin-aarch64).
3. During installation, the input prompt displays "Installing Warp SSH tools..." instead of "Starting shell...".
4. After installation (or if the binary was already present), the remote server binary is launched, the Initialize handshake completes, and the input prompt shows "Initializing..." during this phase.
5. The session does not transition to the fully bootstrapped state until both the Initialize handshake and the `Bootstrapped` DCS hook have been received.
6. On subsequent SSH connections to the same host, the binary is already present and the installation step is skipped entirely.
7. If any step fails (download, launch, handshake), the session falls back to the existing ControlMaster-based `RemoteCommandExecutor` and the user can still use the session.
8. When the feature flag is disabled, the existing SSH flow is completely unchanged.

## Validation

- **Manual testing:** SSH into a fresh Linux VM and a fresh macOS remote. Verify the binary is downloaded, installed, launched, and the Initialize handshake completes. Verify the prompt messages transition correctly: "Starting shell..." → "Installing Warp SSH tools..." → "Initializing..." → working directory.
- **Subsequent connection test:** SSH into the same host again. Verify installation is skipped and the flow goes directly to launch + Initialize.
- **Architecture coverage:** Test on at least one x86_64 and one aarch64 remote host.
- **Error path testing:** Test with a remote host that has no `curl` or `wget`, or where the download URL is unreachable. Verify fallback to ControlMaster-based execution.
- **Feature flag off:** Verify the entire new flow is inactive and the existing SSH behavior is unchanged.
- **Race condition:** Verify correct behavior when `InitializeResponse` arrives before `Bootstrapped`, and vice versa.

## Open questions

None at this time.
