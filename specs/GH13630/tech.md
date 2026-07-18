# Tech Spec: Start Warp for `warpctrl file open`

**Issue:** [warpdotdev/warp#13630](https://github.com/warpdotdev/warp/issues/13630)

## Context

`warpctrl` is an app-bundled wrapper around the channel-specific Warp executable. The wrapper injects `--warpctrl`; `app/src/lib.rs:695-710` recognizes that flag before the normal app/Oz parser and exits through the control CLI without initializing the GUI. The current product contract intentionally assumes an already-running app.

The relevant code paths are:

- `crates/warp_cli/src/local_control/commands.rs:682-698` builds the `FileOpenParams` request for `file open`.
- `crates/warp_cli/src/local_control/commands.rs:771-798` performs generic discovery, instance selection, request dispatch, and output rendering. Empty discovery reaches `select_instance` and returns `no_instance` immediately.
- `crates/warp_cli/src/local_control/mod.rs:555-559,720-734` defines the `file open` command and its path, line, column, new-tab, and target arguments.
- `crates/local_control/src/discovery.rs:315-362` reads owner-only same-channel discovery records and filters them by process liveness, protocol version, channel, and control authority.
- `app/src/local_control/handlers/app_state.rs:748-789` handles the authenticated `file.open` action. It currently constructs `PathBuf` directly from the protocol string, so a relative path would otherwise be interpreted in the app process's working directory rather than the CLI caller's working directory.
- `app/src/lib.rs:2415-2423` initializes local control only for GUI/Test launch modes when the feature is enabled. The app-side Scripting gate decides whether an actionable endpoint is published.
- `script/macos/create_warpctrl_wrapper:14-31` and `script/linux/bundle:278-288` generate wrappers that already resolve the matching channel executable.
- `app/src/uri/mod.rs:937-950,970-994` parses and handles app startup URIs. A private no-op startup intent can reuse this routing without carrying the requested file outside local control.

The security contract in `specs/warp-control-cli/SECURITY.md` remains normative. In particular, startup must not enable Scripting, publish authority when Scripting is disabled, or replace the brokered exact-action credential used for `file.open`.

## Proposed changes

### 1. Resolve relative paths before discovery or startup

At the start of `run_file_command`, convert the supplied path to a `PathBuf`. Preserve absolute inputs. For a relative input, read the control process's current working directory and join the input to it before any discovery or launch operation. Store that resulting absolute path in `FileOpenParams` for both already-running and cold-start flows.

Do not call `canonicalize`, inspect file metadata, expand `~` or environment variables, or otherwise require the path to exist. This preserves the app handler's existing validation and editor-selection behavior, supports paths that may be created later, and avoids changing symlink semantics. If the current directory cannot be read, return a structured `internal` error before launching. If the resolved path cannot be represented by the protocol's UTF-8 string field, return `invalid_params` rather than using a lossy conversion.

Resolving before discovery is required on macOS because Launch Services does not preserve the invoking shell's current directory when it cold-starts an app. It also fixes the existing already-running path so the same CLI invocation has one caller-relative meaning regardless of app state.

### 2. Give the bundled wrapper an explicit app-launch contract

Update the packaged macOS and Linux `warpctrl` wrappers to export a private `WARPCTRL_APP_EXECUTABLE` value containing the absolute channel-specific GUI executable path before they `exec ... --warpctrl`. The macOS wrapper also exports `WARPCTRL_APP_BUNDLE`, derived from the resolved wrapper location, containing the absolute `.app` bundle path whose `Contents/MacOS` executable matches `WARPCTRL_APP_EXECUTABLE`. This keeps launch routing tied to the exact app bundle/package that supplied the CLI instead of searching `$PATH`, guessing an application name, or selecting another installed copy of the same channel.

These values are implementation details rather than supported user-facing environment variables. The CLI validates that the executable is an absolute path to an executable file. On macOS it also validates that the bundle is an absolute `.app` path and that the executable belongs to that bundle. Launch commands receive arguments directly without a shell. A caller can override its own environment and cause its own user account to launch a different program, but this grants no privilege that the caller does not already have.

The standalone Linux `warpctrl` validation artifact is not a product app package and cannot launch a GUI. Its wrapper should omit the launch value so its cold-start path keeps returning a structured `no_instance` error. Normal Linux app-package integration can set the value when the wrapper is installed alongside the GUI executable.

### 3. Add a private, file-free startup intent

Add a private URI action such as `<channel-scheme>://action/warpctrl_startup` to `app/src/uri/mod.rs`. Parsing this action succeeds, but handling it performs no file operation and no local-control mutation.

The launcher uses this intent only to ask the OS or channel executable to start/reuse the matching app:

- On macOS, invoke `/usr/bin/open -a "$WARPCTRL_APP_BUNDLE" <channel-scheme>://action/warpctrl_startup`. Supplying the exact bundle path prevents Launch Services from selecting another Stable, Preview, Dev, or Local installation that registered the same URL scheme. Do not fall back to opening the bare URL when the bundle contract is unavailable or invalid.
- On Linux/FreeBSD package builds, spawn `WARPCTRL_APP_EXECUTABLE` with the startup URI as its only app argument. Existing startup forwarding treats the non-empty URI list as an app-open event rather than synthesizing a new-window request.
- Windows remains unsupported until Warp Control's authenticated discovery and broker requirements are implemented.

The startup intent matters in the race where Warp is already running but Scripting is disabled or a record has not yet been published. It can be forwarded to that process without opening an extra window. A genuinely new app still performs normal restoration and opens a default window if restoration and URI handling leave it with none.

The requested file path is deliberately absent from this URI. Once discovery succeeds, the CLI sends the normal authenticated `file.open` request, preserving target selection, exact-action authorization, acknowledgements, and any later `--wait` lifecycle.

### 4. Add a coordinated cold-start discovery helper

Add a small startup helper used only by `run_file_command`, for example `crates/warp_cli/src/local_control/startup.rs`:

```text
snapshot the last atomically completed startup-attempt generation
discover reachable same-channel instances
if any exist: return them unchanged
if --instance or --pid was explicit: return the empty set unchanged
acquire the per-channel startup lock
discover again after acquiring the lock
if records now exist: release the lock and return them
if the completed generation advanced since the snapshot:
    release the lock and reuse that attempt's recorded failure, timeout, or ready-then-exited result without launching
otherwise start the next generation with a shared 10-second deadline
request matching app startup once and poll authenticated discovery every 100 ms
record the generation's ready, launch-failed, or timed-out outcome
release the lock and return the discovered records or recorded error
```

The generation snapshot comes before initial discovery so an attempt that completes between discovery and lock acquisition is still recognized as overlapping. The second discovery under the lock closes the common race in which another command publishes an instance between the first scan and launch. Put the lock and a separate small attempt-state record in the existing owner-only local-control discovery directory and use an OS-released advisory lock (`flock` on the currently supported POSIX platforms), following the existing remote-server startup-lock pattern. The state stores only a monotonically increasing completed generation, its outcome, and the shared deadline; it never stores file paths, selectors, credentials, or other request parameters. Write the completed state to a temporary owner-only file and atomically rename it into place before releasing the lock, so commands can take a consistent pre-lock snapshot without reading a partial update.

Each command snapshots the completed generation before it can block on the lock. The leader records the next completed generation while still holding the lock. A command that overlapped that attempt therefore observes an advanced generation after acquiring the lock and reuses its outcome. If the instance remains reachable, the second discovery already returned it; otherwise the waiter returns the same structured launch failure or `no_instance` timeout. A leader that recorded readiness whose instance then disappeared is also reported as `no_instance`, without launching or waiting for another 10-second window. A later, non-overlapping invocation snapshots the already-completed generation and may start a new generation, so the state does not create a retry cooldown.

Dropping the file handle releases the advisory lock after success, launch failure, timeout, or process exit. If a leader crashes before recording completion, the next holder re-runs discovery and may start a replacement attempt; an OS-released lock cannot leave a permanent in-progress marker. The lock serializes only startup, not the subsequent file-open requests.

Keep `local_control::discovery::list_instances` as the readiness authority. A process is ready only after its record passes the existing liveness, protocol, channel, authority, and authenticated health checks.

### 5. Specialize `run_file_command` without changing generic commands

Refactor the generic request function just enough to allow `run_file_command` to obtain records from the cold-start helper before calling the existing selector/request/output path. Do not put auto-start inside `run_action_with_params`, because that would silently change all 84 actions.

The `file open` sequence becomes:

1. Resolve the supplied path from the CLI caller's current working directory as described above.
2. Parse target selectors and construct `FileOpenParams` with the resolved path and the existing line, column, and new-tab values.
3. Discover or cold-start only when there is no explicit `--instance`/`--pid`.
4. Call the existing `select_instance` with the resulting records and original selector.
5. Build the same `RequestEnvelope`, request an exact `file.open` credential, send it to the selected app, and render the existing response.

If multiple records appear during startup, the existing selector returns `ambiguous_instance`. If no record becomes reachable within 10 seconds, return `ErrorCode::NoInstance` with details that the matching Warp app was requested to start and that Settings > Scripting may need to be enabled. Since disabled apps intentionally publish no actionable record, the CLI must not claim it can distinguish a disabled setting from every other readiness failure.

### 6. Keep the security and output contracts unchanged

The startup URI is not a local-control request and performs no `file.open` action. It must not include path, line, column, target, or authorization data. The eventual request continues through discovery, the owner-authenticated credential broker, loopback HTTP, exact-action validation, and the app-side handler.

Pretty, text, JSON, and NDJSON success output remains the existing `file.open` acknowledgement. Launch failures and timeouts use the existing structured error envelope and a non-zero exit code. No new successful response shape is introduced.

## Testing and validation

Each product invariant maps to the following automated or manual coverage.

### Unit tests

- Add injectable discovery, launcher, clock/sleep, and request seams around the startup helper so tests do not start a real GUI.
- `relative_path_is_resolved_from_cli_cwd_before_discovery` — a relative input is joined to the fake caller cwd without filesystem canonicalization, and the resolved absolute path reaches both already-running and cold-start requests (invariants 1, 6).
- `absolute_path_is_preserved` — an absolute input is sent unchanged (invariant 1).
- `cwd_resolution_failure_does_not_launch` — failure to read the caller cwd returns a structured error before discovery or startup (invariants 1, 11).
- `file_open_launches_once_when_discovery_is_empty` — empty discovery followed by one reachable record launches once and sends one `file.open` request (invariants 3, 6, 9).
- `file_open_reuses_existing_instance` — a reachable record skips launch and preserves the existing request/output path (invariant 2).
- `explicit_instance_does_not_launch` and `explicit_pid_does_not_launch` — empty discovery reaches the existing `no_instance` result (invariant 4).
- `non_file_commands_do_not_use_startup_helper` — representative generic commands retain their current behavior (invariant 5).
- `cold_start_preserves_file_open_params_and_targets` — path, line, column, new-tab, and target selectors reach the same request envelope after startup (invariant 6).
- `multiple_instances_after_startup_remain_ambiguous` — no instance is silently selected (invariant 7).
- `concurrent_cold_starts_launch_once` — two helpers sharing a temporary discovery directory serialize on one successful startup generation while both complete their own request (invariant 8).
- `concurrent_cold_start_timeout_is_shared` and `concurrent_launch_failure_is_shared` — when the leader records a timeout or launch failure, a waiter reuses that outcome without a second launch or a second 10-second wait (invariants 8, 10, 11).
- `completed_ready_instance_that_exits_is_not_relaunched_by_waiter` — if the leader records readiness but the instance disappears before a waiter re-discovers it, the waiter returns `no_instance` without a second launch (invariants 8, 11).
- `later_command_can_retry_after_shared_failure` — a non-overlapping invocation snapshots the completed failed generation and can lead a new startup attempt, proving the generation state is not a retry cooldown (invariants 3, 8, 11).
- `cold_start_timeout_returns_actionable_no_instance` — the fake clock reaches 10 seconds and the structured error mentions startup and Scripting without claiming a definitive disabled state (invariants 10, 11).
- Add URI parser/handler coverage proving the private startup intent is accepted and produces no file, window, or local-control action when delivered to an existing app (invariant 12).
- Extend wrapper tests to assert the packaged wrapper preserves argument forwarding, exports the matching executable and macOS bundle paths, rejects a mismatched bundle/executable pair, and does not make the standalone validation artifact launch-capable (invariants 3, 11, 13).

### Manual validation

- Enable Settings > Scripting, quit Stable Warp, change into a directory containing a known file, run `warpctrl file open <relative-file>`, and confirm the exact Stable installation that supplied the wrapper starts, opens that file once, focuses the resolved target, and returns the normal acknowledgement.
- Repeat with an absolute path and confirm it is preserved.
- Repeat with Preview and `warpctrl-preview`; confirm Stable is not launched or targeted.
- With two copies of the same channel installed, invoke the wrapper from one copy and confirm that exact `.app` bundle starts rather than the other registered URL handler.
- With Warp already controllable, run the command and confirm no app startup or extra window occurs.
- With Warp running but Scripting disabled, run the command and confirm no extra window appears, the setting remains disabled, and the CLI times out with the actionable `no_instance` error.
- Start two file-open commands concurrently from a stopped app; confirm one app startup, no duplicate window, and both files open through separate authenticated requests.
- Supply stale `--instance` and `--pid` values and confirm Warp is not launched.
- Run `warpctrl instance list` and a representative non-file mutation with no app running; confirm their current behavior is unchanged.

## Risks and mitigations

### Risk: startup makes the CLI feel like it bypasses Scripting

Starting the desktop app and controlling it are separate operations. The private startup intent performs no control action, and readiness still requires an authenticated discovery record. The CLI never changes protected settings and times out if the app remains undiscoverable.

### Risk: concurrent commands create duplicate app processes or windows

The per-channel advisory lock serializes the empty-discovery transition, while the completed-generation record makes overlapping waiters share both successful and failed outcomes instead of launching serially. The file-free startup intent is safe to forward if a leader crashes before it can record completion and a successor must retry. Existing OS/app single-instance routing remains a second layer rather than the only coordination mechanism.

### Risk: a wrapper path points at the wrong executable

Wrappers derive absolute executable and, on macOS, bundle paths from their own installed bundle/package. The CLI validates that the macOS executable belongs to that bundle, passes the exact bundle to Launch Services, invokes commands without shell evaluation, and never searches `$PATH` or falls back to a bare URL, another installation, or another channel.

### Risk: caller-relative resolution changes existing running-app behavior

The running-app path currently depends on the app process's working directory, which is not a stable or caller-visible contract. Resolve only relative inputs, do it before both warm and cold paths, and avoid canonicalization or existence checks so the change is limited to making the documented shell-relative behavior deterministic.

### Risk: slow app startup exceeds the timeout

Ten seconds bounds scripting latency and is long enough to cover normal desktop cold starts. The error explains that Warp was requested to start and can be retried; it does not terminate the launched app.

### Risk: concurrent `file open --wait` work changes request lifetime

The proposed cold-start step finishes before the normal `file.open` request is sent. If #8741 lands, its view-close waiting begins after the same acknowledgement point and does not change startup coordination.

## Follow-ups

- Revisit the top-level `warp <path>` launcher after the CLI reorganization resolves command ownership.
- Specify directory-opening behavior separately rather than inferring it from this file-focused startup path.
- Consider whether other safe, user-initiated `warpctrl` mutations should opt into the same startup helper after this narrow behavior ships and gathers feedback.
