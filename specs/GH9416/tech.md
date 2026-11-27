# Tech Spec: Keep local shells running across Warp relaunches
GitHub issue: https://github.com/warpdotdev/warp/issues/9416
Product spec: [`product.md`](product.md)

## Context
Quitting Warp kills every local shell:
- `on_will_terminate` (`app/src/lib.rs:2836`) calls `PtySpawner::prepare_for_app_termination` (`app/src/lib.rs:2868`), which drops the `TerminalServer` (`crates/warp_terminal/src/local_tty/spawner.rs:174`, `crates/warp_terminal/src/local_tty/server/mod.rs:219`). Dropping its `Children` sends SIGHUP to each shell (`crates/warp_terminal/src/local_tty/server/event_loop.rs:67-75`).
- The terminal server also exits when its parent pid changes (`server/event_loop.rs:223-226`) or its socketpair closes, and it hands the PTY leader fd to the app and closes its own copy (`server/event_loop.rs:276-301`). Nothing reads the PTY while no app is attached.
- Session restoration only restores layout and finished blocks, with a fresh shell.

Facts this design relies on:
- The app uses the leader fd beyond read/write: `TIOCSWINSZ` on resize, which panics on failure (`crates/warp_terminal/src/local_tty/unix.rs:717-723`), `TerminalAttributesPoller` (`tcgetattr`), and `tcgetpgrp` (`app/src/ai/agent_sdk/driver/harness/process_control.rs:59`, `app/src/ai/blocklist/action_model/execute/lrc_activity/sampler.rs:224`).
- Every DCS hook that requires a session is validated against the model's registered session ids (`crates/warp_terminal/src/model/ansi/mod.rs:558-572`, `app/src/terminal/model/terminal_model.rs:2667`); the app registers the id at shell spawn (`app/src/terminal/local_tty/terminal_manager.rs:1095`).
- Replaying `InitShell` makes `PtyController::initialize_shell` write the bootstrap script (`app/src/terminal/writeable_pty/pty_controller.rs:325`).
- Hook wire format: `ESC P $` + `d` + hex(JSON) + `ESC \` (`app/assets/bundled/bootstrap/zsh_body.sh:17-26`, `crates/warp_terminal/src/model/ansi/dcs_hooks.rs:17`).
- `terminal_panes.uuid` (`crates/persistence/src/schema.rs:413`) is stable across restore (`app/src/pane_group/mod.rs:1639` → `create_session`, `:6111`); new panes use `Uuid::new_v4()` (`:2180`). It does not reach `TerminalManager::create_model` today.
- The terminal-server framing does a single `recvmsg` per payload (`server/protocol.rs:153-168`) and carries one fd (`AuxData = [RawFd; 1]`, `:46`).
- `pre_exec` already resets signal dispositions in the child (`unix.rs:500-521`), and `Pty::kill` drops the PTY fd before killing to avoid a macOS 'E' state hang (`unix.rs:727-737`).
- The terminal server's client turns exits reported by the server into a `pid → status` map and raises SIGCHLD on itself (`server/mod.rs:71-111`), which `Pty::next_child_event` (`unix.rs:695-710`) consumes.

## Proposed changes

### Overview
1. A per-user, per-protocol-version terminal daemon (`warp terminal-server --daemon`) **owns the PTY**: it always reads it and keeps a 10 MiB ring buffer per session, so processes never block while no app is attached.
2. Per session the app receives two fds over SCM_RIGHTS: `data` (one end of a daemon-created socketpair, bytes in both directions) and `leader` (a dup of the PTY leader, used by the app **only for ioctls**: resize, termios, `tcgetpgrp`). No resize message is needed.
3. Session id = `terminal_panes.uuid`.
4. Reattach: the daemon sends a **preamble** (raw `InitShell` + `Bootstrapped` DCS of the login shell) followed by the **body** (bytes since the `CommandFinished` that immediately precedes the last `Precmd` seen before detach). The app registers the original hook session id, does not resend the bootstrap script, and parses the replay **before** the live loop, discarding terminal-query replies. This rebuilds `SessionInfo` and the running block through the normal model path.
5. Closing a pane sends `Kill`. Quitting the app detaches. An app crash is a detach.

Alternatives rejected: handing only the PTY fd to the app and reading in the daemon only while detached (incomplete ring; a crash loses the running block); replaying over the control channel (the framing cannot carry MBs, small macOS socket buffers); suppressing query replies inside `pty_read` by byte counting (chunks mix replay and live bytes).

### Modules
New, all `cfg(unix)`, under `crates/warp_terminal/src/local_tty/server/`:
- `api.rs`: the daemon contract next to the legacy `Message`.
- `protocol.rs`: generalized framing (any `Serialize` type, up to two fds, blocking and non-blocking readers).
- `daemon/`: `mod.rs` (event loop, requests, lifecycle), `listener.rs` (socket, lock, private directory, peer uid), `session.rs` (per-session PTY, queues), `ring.rs` (ring buffer and replay), `scanner.rs` (incremental DCS hook scanner), `process.rs` (daemonize, hardening, rlimits).
- `daemon_client.rs`: `DaemonClient`, `AttachedSession`, `DaemonPtyHandle`.

Changed: `local_tty/mod.rs` (`EventedPty::take_replay`), `local_tty/event_loop.rs`, `local_tty/unix.rs` (`Pty::from_attached`, `PtyOptions::base_env` / `child_rlimits`), `local_tty/spawner.rs` (daemon routing, `prepare_for_app_termination(detach_sessions)`), `crates/warp_cli/src/lib.rs` (`--daemon`), and in the app `lib.rs`, `root_view.rs`, `pane_group/`, `quit_warning/`, `session_management.rs`, `settings_view/features_page.rs`, `terminal/general_settings.rs`, `terminal/local_tty/terminal_manager.rs`, `terminal/writeable_pty/pty_controller.rs`.

### 1. Transport
- Directory: `warp_core::paths::state_dir().join("ts")` (`crates/warp_core/src/paths.rs:260`). Files: `v{DAEMON_PROTOCOL_VERSION}.sock`, `.lock`, `.log`. If the socket path exceeds 100 bytes, fall back to `$XDG_RUNTIME_DIR/warp-ts-<key>/` (Linux, when set and absolute), else `$TMPDIR/warp-ts-<uid>-<key>/`, else `/tmp/warp-ts-<uid>-<key>/`, where `<key>` is the low 48 bits of the FNV-1a hash of the state directory in hex, so different channels and homes never share a daemon.
- The directory is created with `DirBuilder::mode(0o700)` (never world-readable, not even briefly) and must be a real directory (not a symlink) owned by `geteuid()`. `connect_or_spawn` and the daemon tighten an existing one we own to 0700; `connect_existing` only validates it (missing → no daemon; symlink, other owner or group/other bits → error) and never creates it.
- `umask(0o077)` before `bind`, then `chmod 0600`. Both ends check the peer uid against `geteuid()` (`getpeereid` on macOS, `SO_PEERCRED` on Linux): the daemon on accept (close silently on mismatch) and the client right after `connect`, before sending `Hello` (error on mismatch).
- Framing: BE `usize` header + bincode + SCM_RIGHTS, generalized: `send_frame<T: Serialize>(fd, &T, fds: &[RawFd])`, `cmsg_space!([RawFd; 2])`, a blocking reader that loops until the payload is complete, and a non-blocking `FrameReader` on the daemon side that accumulates bytes and fds. The legacy `api::Message` path is unchanged.
- One full-duplex control connection; responses correlate by `request_id`; a client reader thread dispatches responses and events.
- The daemon never blocks on a client: every frame it sends goes through a per-connection outbound queue (`FrameWriter`, 8 MiB cap) written when the socket is writable; a client whose queue overflows is dropped. Fds attached to a frame stay open for 5 s after their first byte is written (macOS in-flight fd GC).
- Limits on the control socket: client frames ≤ 4 MiB (daemon frames ≤ 64 MiB); each read pass takes at most 256 KiB per connection before yielding; at most 16 concurrent connections; a connection must send `Hello` within 5 s.
- Received fds are close-on-exec immediately: `MSG_CMSG_CLOEXEC` on Linux, `FD_CLOEXEC` right after `recvmsg` on macOS. Daemon socketpairs use `SOCK_CLOEXEC` on Linux.
- Single client: a valid `Hello` evicts the previous client (a detach for it).

### 2. Daemon lifecycle
- CLI: `warp_cli::TerminalServerArgs` (`crates/warp_cli/src/lib.rs:734`) gains `#[clap(long)] pub daemon: bool`; `run_terminal_server` dispatches on it and `--parent-pid` is not required in daemon mode.
- Startup, before any thread: double fork, `setsid()`, `SIGHUP` ignored, `umask(0o077)`, `chdir("/")`, stdin → `/dev/null`, stdout/stderr → `v{N}.log` (append; truncated at start above 5 MB). Hardening: `RLIMIT_NOFILE` soft raised to the hard limit (capped at 10240), `RLIMIT_CORE` soft = 0, `PR_SET_DUMPABLE = 0` on Linux. Then `flock(v{N}.lock, LOCK_EX|LOCK_NB)` (on failure exit 0; held for life), unlink a stale socket, `bind`, `listen`, logger, `init_platform()` and feature flags as today.
- Log rotation: every tick, a log above 5 MB is renamed to `v{N}.log.old` and stdout/stderr point at a fresh file.
- Connect-or-spawn (client): `connect` + peer uid check + `Hello`. On `ENOENT`/`ECONNREFUSED`, or `EPIPE`/`ECONNRESET`/EOF during the handshake (a daemon that is idle-exiting), run `current_exe() terminal-server --daemon` with null stdio and fds ≥ 3 closed in `pre_exec` (same bound as `unix.rs` `close_fds`: soft `RLIMIT_NOFILE`, falling back to a positive `sysconf(_SC_OPEN_MAX)`, clamped to 3..65536), `wait()` the short-lived child, and retry `connect` every 50 ms up to 3 s. The daemon's flock serializes races.
- Shells are spawned with `local_tty::spawn` and `close_fds = true`; all daemon fds are CLOEXEC.
- Spawn context: the daemon never hands shells its own stale state (the environment of whichever app launched it, its `/` cwd, its hardened limits) and never mutates its own environment, cwd or limits to spawn. `Spawn` carries a `SpawnEnvironment { vars }` captured by the client (`std::env::vars_os()`, lossless). The daemon sets `PtyOptions::base_env = Some(vars)` and `PtyOptions::child_rlimits` to the soft `RLIMIT_NOFILE`/`RLIMIT_CORE` it inherited before hardening. With `base_env`, `unix.rs` builds the command with `env_clear()` + `base_env` (unsettable variables are skipped), resolves `HOME`/`USER`/`LOGNAME` from it, then applies the Warp overlay and `PtyOptions::env_vars` as in the legacy path; `base_env: None` (in-process and legacy terminal-server spawns) inherits the spawner's environment unchanged. In `pre_exec`, after closing fds, the child sets each `child_rlimits` soft limit (clamped to the hard limit, best effort, async-signal-safe calls only). The initial directory is `start_dir`, else `$HOME` from `base_env`. `PR_SET_DUMPABLE` is reset by `execve`.
- Idle exit: no sessions (alive or exited-pending) and no client for 60 s (overridable with `WARP_TERMINAL_DAEMON_IDLE_SECS` for tests).
- Control EOF from the app = detach all its sessions; nothing is killed.
- `Kill`: close the leader, SIGKILL the pid, one `waitpid(WNOHANG)`, reply `Done` right away; SIGCHLD reaps it. The event loop never sleeps or blocks on a single client or process.
- Shell exit: drain the leader until `EIO`/`EAGAIN`. If attached: `shutdown(SHUT_WR)` on data, emit `SessionExited`, drop the session. If detached: keep `Exited { status }` + ring until the next `Attach` (delivered, then dropped), `Prune`, or 24 h.

### 3. Ring buffer and replay (daemon)
- `RING_CAPACITY = 10 MiB` per session, lazily grown, absolute offsets (`total: u64`, `ring_start = total - len`).
- Incremental DCS scanner: `ESC P $` + `d|f` + payload + `ESC \` (or 0x9C; CAN/SUB abort), payload capped at 8 MiB (a warning is logged when a hook is dropped for size), tolerant of splits across reads. Sizing: the largest hook, `Bootstrapped`, carries aliases, functions, builtins, keywords, options and exported variable names; a measured zsh setup sends about 16 KiB of JSON (32 KiB hex), heavy plugin setups a few hundred KiB. For `d`, hex-decode and parse only `{hook, value.session_id, value.is_subshell}`. `CommandFinished`, `Precmd` and `Preexec` count only when `session_id` equals the session's hook session id; hooks from warpified subshells or SSH (other ids) or without an id never move the replay start:
  - `CommandFinished`: kept as pending; a `Preexec` clears a pending one.
  - `Precmd`: a prompt starts at the pending `CommandFinished` (consumed) or else at the `Precmd` itself. The start offset goes into `prompt_starts` (pruned on eviction); the raw `CommandFinished` + `Precmd` of the last prompt are kept.
  - `Preexec`: the last raw one is kept.
  - Login `InitShell` (`is_subshell == false`) followed by the first `Bootstrapped`: both raw DCS are stored as the preamble; a new login `InitShell` replaces the pair.
- On detach (explicit or EOF): `replay_from` = offset of the `CommandFinished` immediately before the last `Precmd`, or that `Precmd`'s offset if there is none (0 if no prompt yet). Starting at `CommandFinished` lets the restored model leave the hidden bootstrap block.
- Replay = preamble + body, where body is the first that applies:
  1. `ring[replay_from..]` if `replay_from >= ring_start`;
  2. `ring[oldest_prompt_start..]` if any prompt start remains;
  3. last `CommandFinished` (if paired with the last prompt) + last `Precmd` + last `Preexec` (if later) + `"\r\n[output truncated]\r\n"` + `ring[ring_start..]`.
- On `Attach` the daemon enqueues the replay on the data queue **before** resuming live forwarding, then replies with `replay_len`.
- Backpressure: per-session outbound queue capped at 4 MiB; above it the daemon stops reading the leader until the queue drops below 1 MiB. With no client attached it writes only to the ring and never blocks the process.

### 4. Contract (`crates/warp_terminal/src/local_tty/server/api.rs`)
```rust
pub const DAEMON_PROTOCOL_VERSION: u32 = 2;
pub const RING_CAPACITY: usize = 10 * 1024 * 1024;

pub struct PersistentSessionId(pub [u8; 16]);
pub struct SessionMetadata { pub hook_session_id: u64, pub shell_type: String }
pub struct SpawnEnvironment { pub vars: Vec<(OsString, OsString)> }

pub enum ClientMessage {
    Hello { protocol_version: u32, client_pid: u32, app_version: Option<String> },
    Request { request_id: u64, request: DaemonRequest },
}

pub enum DaemonRequest {
    List,
    Spawn { session_id: PersistentSessionId, metadata: SessionMetadata, options: Box<PtyOptions>, environment: Option<SpawnEnvironment> },
    Attach { session_id: PersistentSessionId },
    Kill { session_id: PersistentSessionId },
    Prune { keep: Vec<PersistentSessionId> },
    DetachAll,
}

pub enum DaemonMessage {
    HelloAck { protocol_version: u32, daemon_pid: u32 },
    HelloReject { protocol_version: u32, reason: String },
    Response { request_id: u64, response: DaemonResponse },
    Event(DaemonEvent),
}

pub enum DaemonResponse { Sessions(Vec<SessionSummary>), Attached(AttachInfo), Done, Error(String) }

pub struct SessionSummary {
    pub session_id: PersistentSessionId,
    pub pid: u32,
    pub metadata: SessionMetadata,
    pub exit_status: Option<ObservedExitStatus>,
    pub attached: bool,
}

pub struct AttachInfo {
    pub session_id: PersistentSessionId,
    pub pid: u32,
    pub metadata: SessionMetadata,
    pub replay_len: u64,
    pub has_bootstrap_preamble: bool,
    pub exit_status: Option<ObservedExitStatus>,
}

pub enum DaemonEvent {
    SessionExited { session_id: PersistentSessionId, pid: u32, status: ObservedExitStatus },
}
```
Rules:
- `Attached` always carries fds in order `[data, leader]`; `leader` is absent if the session already exited.
- `Spawn` replies `Attached` with `replay_len = 0`. `DaemonClient::spawn` always sends `environment: Some(SpawnEnvironment::capture())`; `None` keeps the daemon's environment (tests only).
- `Spawn` with an id the daemon already has replaces it: the old session is killed with `Kill` semantics, then the new shell is spawned under the same id. The app relies on this when `attach()` fails during restore.
- `Hello` must be the first frame; on version mismatch the daemon replies `HelloReject` and closes.

Client API (`server/daemon_client.rs`, re-exported from `server/mod.rs`):
```rust
impl DaemonClient {
    pub fn connect_or_spawn() -> anyhow::Result<Arc<Self>>;
    pub fn connect_existing() -> anyhow::Result<Option<Arc<Self>>>;
    pub fn list(&self) -> anyhow::Result<Vec<SessionSummary>>;
    pub fn spawn(&self, id: PersistentSessionId, metadata: SessionMetadata, options: PtyOptions) -> anyhow::Result<AttachedSession>;
    pub fn attach(&self, id: PersistentSessionId) -> anyhow::Result<AttachedSession>;
    pub fn prune(&self, keep: Vec<PersistentSessionId>) -> anyhow::Result<()>;
    pub fn detach_all(&self);
    pub fn kill_all_attached(&self);
    pub fn is_alive(&self) -> bool;
}
pub struct AttachedSession { pub info: AttachInfo, pub data: OwnedFd, pub leader: Option<OwnedFd>, pub handle: Box<dyn PtyHandle> }
```
- The reader thread turns `SessionExited` into a `pid → status` entry and raises SIGCHLD on itself (same pattern as `server/mod.rs:71-111`), so `Pty::next_child_event` works unchanged. The same applies when `attach()` returns `exit_status: Some`.
- Daemon EOF: every session becomes `Unavailable`, SIGCHLD is raised, `is_alive()` returns false.
- `DaemonPtyHandle::kill()` is a no-op after `detach_all()`; otherwise it sends `Kill` and waits for `Done`.

### 5. Terminal side (`local_tty`)
- `EventedPty` (`local_tty/mod.rs:66`) gains `fn take_replay(&mut self) -> Option<Vec<u8>> { None }`. In `local_tty/event_loop.rs`, right after `State::default()` and before registering fds, replay bytes go through `state.parser.parse_bytes` with a reply buffer that is **discarded**, then a wakeup is sent.
- `Pty` (`unix.rs:207`) gains `leader: Option<File>` and a pending replay length; `Pty::from_attached` builds it from an `AttachedSession`. `get_fd()` returns the leader if present; `on_resize` ioctls on it; `kill` drops the data fd and the leader **before** `pty_handle.kill()` (keeps the macOS 'E' state fix). The replay is read once, blocking, with a 5 s timeout.
- After attaching, `TIOCSWINSZ` with `(rows, cols - 1)` then `(rows, cols)` on the leader so TUIs redraw.
- `PtySpawner` stores a lazily connected `Option<Arc<DaemonClient>>`; a failed connection marks the daemon unavailable for the run; a dead daemon is reconnected at most once. `prepare_for_app_termination(detach_sessions: bool)` calls `detach_all()` or `kill_all_attached()` and then drops the legacy server as today.

### 6. App
- Setting in `app/src/terminal/general_settings.rs`, after `restore_session`: `persistent_sessions: PersistentSessions`, `bool`, default `true`, macOS + Linux, `toml_path: "general.persistent_sessions"`. `GeneralSettings::should_persist_sessions()` = `cfg!(unix) && persistent_sessions && restore_session && APPIMAGE unset`.
- Settings UI in `app/src/settings_view/features_page.rs`: `PersistentSessionsWidget` right after `SessionRestorationWidget`, `FeaturesPageAction::TogglePersistentSessions` handled with `toggle_and_save_value`, rendered disabled when `restore_session` is off. Copy as in product invariant 6.
- Readers of `should_persist_sessions()`: `TerminalManager::create_pty` per pane (daemon vs legacy), startup restore (attach vs prune), `on_will_terminate` (detach vs kill), and the quit warning.
- ON → OFF: new panes use the legacy path; daemon panes keep working until closed; on quit `kill_all_attached()`. OFF → ON: new panes only. No restart needed.
- Uuid plumbing: `create_session` → `create_model` / `create_model_with_manager` gain `persistent_session_id: Option<PersistentSessionId>` → `on_shell_determined` → `create_pty`. `create_tui_model` and `spawn_replacement_shell` pass `None`.
- New pane with persistence: `spawn(uuid, { hook_session_id, shell_type }, options)` and `Pty::from_attached`. On failure: report once, mark the daemon unavailable for the run, fall back to the legacy `spawn_pty`.
- Restore: `list()` once per run (cached). If the uuid exists with `has_bootstrap_preamble`: keep `set_login_shell_spawned` / `set_pending_shell_launch_data`, `register_session_id(SessionId::from(hook_session_id))` instead of the generated id, skip `enqueue_init_script`, call `PtyController::skip_bootstrap_write_for(session_id)` (consumed by the next `InitShell`, only for that id), and use the leader for `ShellProcessInfo` and `TerminalAttributesPoller`. Attaching runs off the main thread. If the attach fails or there is no preamble, `spawn` with the same uuid (replacing the session) and normal bootstrap.
- Prune (`root_view.rs`, `prune_persistent_sessions`), only after a snapshot was restored: if `connect_existing()` finds a daemon, `prune(terminal uuids in the snapshot plus sessions this run already owns)`; with persistence off, `prune(vec![])`. It never starts a daemon.
- Quit warning (`app/src/quit_warning/mod.rs`): when the scope is `QuitScope::App`, persistence is on and every long-running command's pane `persists_after_quit()` (its shell is daemon-backed and the daemon is alive), append " They will keep running in the background." to the process line.

### 7. Fallbacks
| Case | Behavior |
|---|---|
| Persistence off | Full legacy path; `Prune(vec![])` at startup only if a daemon exists. |
| Non-unix | Everything under `cfg(unix)`; Windows unchanged. |
| Cannot connect/spawn in 3 s | One `report_error`, legacy for the run, restored panes get fresh shells. |
| `HelloReject` | Same as above; the old daemon is left running with its sessions. |
| Daemon dies | Panes end as `Unavailable`; the next new pane retries `connect_or_spawn` once. |
| AppImage (`APPIMAGE` set) | Feature disabled (unmounting the image can SIGBUS the daemon). |

## Testing and validation
Each product invariant maps to automated tests (`crates/warp_terminal` unit tests unless noted) and to the manual checklist below.

| Invariant | Automated coverage |
|---|---|
| 1. Quit keeps processes alive | `spawner_tests::termination_detaches_sessions_when_persisting`; `daemon_tests::detach_all_makes_kill_a_no_op`; e2e `daemon_e2e_session_survives_client_exit_and_crash` |
| 2. Relaunch reattaches with output | `daemon_tests::detached_output_is_replayed_after_the_preamble_and_live_bytes_follow`; `ring_tests::replay_*`; `unix_tests::attached_pty_reads_replay_once`, `attached_pty_resizes_through_the_leader`; app `event_loop_tests::replay_is_processed_before_live_bytes_without_answering_queries`, `terminal_model_tests::reattached_session_replay_rebuilds_bootstrapped_running_block`, `daemon_shaped_replay_rebuilds_bootstrapped_running_block`, `pty_controller_lifecycle_tests::skip_bootstrap_write_*`; `spawner_tests::attach_*` |
| 3. Bounded output, never blocks | `ring_tests::ring_evicts_oldest_bytes_and_tracks_absolute_offsets`, `replay_case_three_*`; `daemon_tests::detached_session_keeps_running_through_heavy_output`, `backpressure_pauses_reading_without_losing_bytes`; e2e `daemon_e2e_heavy_output_while_detached_does_not_block` |
| 4. Exit while closed | `daemon_tests::exited_session_is_kept_until_attached_then_dropped`, `session_exit_while_attached_is_reported_and_drops_the_session`; `unix_tests::attached_pty_drains_output_until_eof_after_exit` |
| 5. Close kills | `daemon_tests::kill_terminates_the_process_and_removes_the_session`, `kill_does_not_wait_for_a_process_that_ignores_sigterm`; `unix_tests::attached_pty_kill_closes_fds_before_killing`; e2e `daemon_e2e_kill_prune_and_idle_exit` |
| 6–8. Setting and effective state | app `general_settings_tests::persistent_sessions_defaults_to_enabled`, `persistent_sessions_require_restore_session_and_no_appimage` |
| 9–10. Toggle at runtime | `spawner_tests::termination_kills_attached_sessions_when_not_persisting`, `termination_without_daemon_does_not_connect`, `backed_sessions_follow_daemon_liveness` |
| 11. Stale cleanup | `daemon_tests::prune_kills_detached_sessions_that_are_not_kept`; `spawner_tests::prune_never_starts_a_daemon`, `prune_uses_running_daemon`, `prune_keeps_sessions_owned_by_this_run`; app `root_view_tests::nothing_is_pruned_without_a_restored_snapshot`, `restored_terminal_panes_are_kept_when_persisting`, `every_session_is_pruned_when_not_persisting` |
| 12. Quit warning | app `quit_warning_tests::warning_mentions_background_only_for_persistent_sessions`, `sessions_without_running_commands_do_not_keep_running` |
| 13. Crash = detach | `daemon_tests::new_client_evicts_the_previous_one`; e2e `daemon_e2e_session_survives_client_exit_and_crash` |
| 14. Daemon unavailable | `spawner_tests::failed_connection_marks_daemon_unavailable_for_the_run`, `dead_daemon_is_reconnected_only_once`; `daemon_client_tests::daemon_exit_marks_attached_sessions_unavailable`, `connect_or_spawn_fails_if_the_spawn_fails`, `connect_or_spawn_replaces_a_daemon_that_closes_during_the_handshake` |
| 15. Version mismatch | `daemon_tests::hello_with_another_protocol_version_is_rejected`; e2e `daemon_e2e_rejects_another_protocol_version_and_stays_up` |
| 16. Isolation | `daemon_tests::socket_and_directory_are_private`, `peer_uid_mismatch_is_rejected`, `private_dir_*`, `symlinked_private_dir_is_rejected`, `connections_above_the_limit_are_refused`, `oversized_client_frame_closes_the_connection`, `connection_without_hello_is_closed_after_the_timeout`; e2e `daemon_e2e_daemonizes_and_is_private` |
| 17. Idle exit | `daemon_tests::idle_daemon_exits_and_removes_its_socket`, `daemon_with_a_session_does_not_exit_when_idle`, `second_daemon_does_not_start_while_the_lock_is_held` |
| 18. Shell environment | `unix_tests::base_env_replaces_the_inherited_environment`, `spawned_shell_gets_the_base_env_and_child_rlimits`; `daemon_tests::shell_starts_from_the_client_environment`; e2e `daemon_e2e_shell_gets_the_client_environment_and_limits` |

Protocol and scanner building blocks: `protocol_tests` (every message over a socket, two fds in order, payloads larger than the socket buffer, split frames, legacy messages), `api_tests` (bincode round trips, non-UTF-8 environment), `scanner_tests` (byte-by-byte splits, foreign DCS, CAN/SUB abort, payload cap).

The e2e tests run against the real binary, headless, with an isolated `HOME`, and are skipped unless `WARP_DAEMON_E2E_BIN` is set:
```
cargo build --bin warp-oss --features gui
WARP_DAEMON_E2E_BIN=$PWD/target/debug/warp-oss cargo test -p warp_terminal daemon_e2e -- --test-threads=1
```

### Manual verification
Build and run the app; the setting lives at Settings > Features, "Keep sessions running after quitting Warp". Use `pgrep -fl 'terminal-server --daemon'` and `ps -o pid,ppid,command -p <shell pid>` to observe processes.

Setting on (default, "Restore windows, tabs, and panes on startup" on):
1. Start `i=0; while true; do echo $i; i=$((i+1)); sleep 0.5; done` in a tab, quit and accept the quit warning, which says the processes keep running in the background. The shell and one `terminal-server --daemon` (ppid 1) stay alive. (Invariants 1, 12)
2. Relaunch: the tab is restored with the same shell pid, the counter continues without a gap, the prompt and block history are intact, and typing works. (2)
3. Run `vim` or `top`, quit and relaunch: the TUI redraws at the right size and follows window resizes. (2)
4. Force-quit the app (`kill -9`): relaunch and confirm the shells are still there. (13)
5. Close a tab or pane: its process ends and it is not restored. (5)
6. Open several windows, quit, relaunch: each pane reattaches to its own shell. (2)
7. Produce about 100 MB of output (`head -c 100000000 /dev/zero | tr '\0' y`) and quit mid-way: the command finishes in the background and the relaunch shows its tail without freezing. (3)
8. Quit with no shells running: the daemon exits after about 60 s. (17)

Toggle scenarios:
9. ON → OFF with a daemon tab open: the tab keeps working; a new tab uses an ordinary shell; quit kills every shell. (9)
10. OFF → ON: only new tabs persist; older tabs die on quit; no restart needed. (10)
11. Setting off at launch while a daemon from an earlier run holds sessions: they are pruned and panes get fresh shells. (11)
12. Session restoration off: the toggle renders disabled and behaves as off. (7)
13. Linux with `APPIMAGE` set: the feature is disabled and quitting kills shells. (8)
14. Delete the daemon socket while Warp runs and open a new tab: one error is reported and the tab falls back to an ordinary shell. (14)

## Risks and mitigations
1. **Rebuilt bootstrap.** The preamble moves the model from `WarpInput` straight to bootstrapped and the replayed hooks trigger follow-up events. Validated by the model tests above; a fallback would be `set_bootstrapped()` plus a synthetic session entry.
2. **Warpified subshells and SSH** use other session ids and are rejected on replay; their output is shown as plain text.
3. **Crash before persisting.** Blocks before `replay_from` that were not yet persisted are lost (never duplicated).
4. **macOS.** No app sandbox; the hardened runtime allows fork/exec/setsid. Bundle replacement on update uses an atomic rename, so a running daemon keeps its binary.
5. **Linux.** systemd `KillUserProcesses` kills the daemon on logout (accepted).
6. **Throughput.** Extra hop PTY → daemon → socket → app; measure with `yes | head -c 1G`.
7. **Memory.** Up to 10 MiB per busy session.
8. **Protocol bumps.** A new protocol version does not reattach sessions from the previous one; the old daemon keeps them until they exit.

## Threat model
- Trust boundary: the user account. Any process running as the same uid can connect to the socket, list, attach to, write into and kill every persistent session, and read its scrollback. This is accepted, as with tmux, screen and similar multiplexer servers: such a process can already `ptrace` the user's shells (where permitted) or read their files.
- Other users are kept out: the directory is 0700 and owned by the user (not a symlink), the socket is 0600, and both ends enforce the peer uid; the client never sends anything to a daemon owned by another uid.
- Scrollback lives only in daemon memory (10 MiB ring per session); nothing is written to disk. Core dumps are disabled (`RLIMIT_CORE = 0`) and on Linux the daemon is non-dumpable, so its memory is not exposed through core files or same-uid `ptrace` / `/proc/<pid>/mem`. The log records lifecycle events only, never terminal bytes.
- Resource abuse by a same-uid client is bounded (frame size, per-read budget, connection count, `Hello` timeout, outbound queue).

## Follow-ups
- Rebuild blocks for warpified subshells and SSH sessions on reattach.
- Decide whether a mismatched old daemon should be shut down on protocol bumps.
- Windows support.
