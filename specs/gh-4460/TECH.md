# Dev Container file tools and code review

Tracker: [#4460](https://github.com/warpdotdev/warp/issues/4460). This spec extends
[the Dev Container prototype](https://github.com/warpdotdev/warp/pull/15516) and is stacked on
[the build-pane implementation](https://github.com/warpdotdev/warp/pull/15655).

## Context

A Dev Container shell is attached through `docker exec`, but
`SessionInfo::determine_session_type` infers locality from the shell hostname
(`app/src/terminal/model/session.rs:723`). It therefore produces
`WarpifiedRemote { host_id: None }`. Remote file tools, path resolution, working-directory
models, and code review require a connected `HostId`; they must not use a host path because a
workspace can exist only in the container or on a named volume.

## Proposed changes

### Architecture and lifecycle

Add `DevContainerTransport` in `app/src/remote_server/`. It implements the existing
transport-neutral `RemoteTransport` contract
(`crates/remote_server/src/transport.rs:209`) with non-TTY Docker subprocesses:

- Store the resolved Docker path, container ID, remote user, and remote workspace from
  `ShellLaunchData::DevContainer` (`crates/warp_terminal/src/shell/mod.rs:826`).
- Run detection, checks, installation, and the proxy with the same effective user (`docker exec
  -u`, when configured) and workspace (`docker exec -w`) as the interactive shell.
- Connect with `docker exec -i ... <warp> remote-server-proxy`; do not allocate a TTY. Return the
  owning, `kill_on_drop` child in `Connection`.
- Reuse the remote-server install scripts and client tarball cache. Try the in-container download
  first. If the container has no usable downloader or network path, download on the client, copy
  the archive with `docker cp`, and run the same installer in the container. Install in the
  container user's channel-specific Warp directory, remove copied temporary files, and require a
  version-valid initialize handshake. Extract transport-neutral install/cache helpers from the SSH
  implementation instead of duplicating them.

Extend the existing Dev Container staging flow
(`app/src/terminal/view/dev_container/mod.rs:650`) in this order:

1. `devcontainer up` returns the container identity and Warp generates the existing `SessionId`.
2. Preflight and bootstrap-script staging succeed.
3. Drive `RemoteServerManager` through platform detection, preinstall check, binary
   check/install, connect, and initialize handshake using `DevContainerTransport` and that same
   `SessionId`.
4. Replace the build pane only after `SessionConnected { session_id, host_id }`. Keep the
   connection registered under that `SessionId`.
5. Bootstrap the terminal with the same `SessionId`, assign its already-connected `HostId`, and
   send the existing shell-bootstrap notification before downstream command requests.
6. On pane/session exit, call `deregister_session`. Dropping manager state terminates the Docker
   proxy child. Warp does not stop or remove the Dev Container.

Remote-server setup remains part of the current `Staging` phase; it adds no UI phase. Any detect,
check, install, connect, or handshake failure deregisters the attempted connection, leaves the
build pane failed with its logs and existing Retry/Close actions, and never opens a terminal with
host-local fallback. Retry starts the full existing attempt lifecycle with a new `SessionId`.

### Session identity and routing

Treat Dev Container launch data as an explicit remote-server-backed session origin before hostname
comparison:

- `SessionInfo::create_pending` classifies `ShellLaunchData::DevContainer` as
  `BootstrapSessionType::WarpifiedRemote`, even when container and host names match. It never
  classifies a Dev Container as `Local`.
- Generalize the remote-server event subscription, initial `HostId` lookup, reconnect executor
  swap, and `RemoteServerCommandExecutor` selection in
  `app/src/terminal/model/session.rs:175` and
  `app/src/terminal/model/session/command_executor.rs:144` from “SSH wrapper under
  `SshRemoteServer`” to “session origin has an enabled remote-server backend.” Preserve the SSH
  behavior.
- A connected Dev Container session has
  `SessionType::WarpifiedRemote { host_id: Some(container_daemon_host_id) }`. On disconnect,
  immediately clear the `HostId`; all remote operations fail closed until reconnection.
- Reconnect only through the captured container ID. A replaced or stopped container is not
  equivalent. Its failed connection must not fall back to host files. A new container handshake
  can return a new `HostId`; do not reuse models keyed by the old ID.
- Connections remain per `SessionId`. Sessions in one running container receive the daemon's same
  `HostId`, so host-scoped models deduplicate correctly.

Keep `FeatureFlag::LocalDevContainer` as the Dev Container gate. Do not require
`FeatureFlag::SshRemoteServer` and do not add a flag unless implementation needs an independently
reversible bootstrap gate. Retain the current Unix, local-TTY, Docker CLI, and Dev Containers CLI
requirements. Windows, remote-TTY, and Docker sandbox support are out of scope. Keep the backend
selection extensible so Docker sandbox can opt into the Docker transport later.

### Consumers

Do not add container-specific file or git APIs. The connected `HostId` must activate existing
remote paths:

- `ActiveSession::location_for_path` returns `LocalOrRemotePath::Remote`
  (`app/src/terminal/model/session/active_session.rs:105`).
- `read_files` and request-file-edits/apply-diff use the existing host request handle and operate
  on container paths; accepted edits create, update, and delete only container files.
- `WorkingDirectoriesModel` creates the existing remote `DiffStateModel`
  (`app/src/pane_group/working_directories.rs:385`). Repository detection, diffs, git status, and
  code-review operations run against container git and filesystem state. The current code-review
  and tool-call UI is unchanged.
- `grep` and `file_glob` continue through the session command executor. `search_codebase` uses the
  existing remote-index path when available. Do not change index eligibility, auto-indexing, or
  indexing UI.

### Security and compatibility

The selected container is a trust boundary. The initialize protocol sends the bearer token, user
identity, and crash-reporting preference
(`app/src/remote_server/auth_context.rs:14`) into it. Do not place credentials in Docker argv,
environment variables, logs, copied archives, or host path mappings. Send authentication only in
the existing protocol handshake. Run the proxy and installer as the attached container user and
retain owner-only daemon directories.

Supported-platform/install failures block attach. The client-copy fallback covers containers
without download tools or outbound access, not unsupported architectures or missing Docker
permissions. Container disconnects disable remote operations without exposing host data. Hostname
collisions never affect identity.

## Non-goals

- Host-to-container path translation or mirroring.
- Docker sandbox support, new indexing behavior, or UI redesign.
- Changes to the build-log/newline/JSON behavior specified in
  [#15648](https://github.com/warpdotdev/warp/pull/15648).
- Visual proof for this change; manual functional acceptance is required instead.

## Decisions and assumptions

- Use the in-container remote server, not host path mapping or the live PTY. This preserves
  container filesystem authority and reuses the file, git, indexing, reconnect, and `HostId`
  contracts already used by SSH.
- Block pane replacement until the handshake succeeds, rather than attach with reduced tools. This
  prevents a remote session from being mistaken for a usable local filesystem.
- Assumption: starting a Dev Container authorizes Warp to install its channel-matched daemon
  automatically as the configured container user. No additional install prompt is added.
- Assumption: the container remains user-managed. Warp owns only its proxy process and daemon
  files, not the container lifecycle.

## Testing and validation

Add focused tests with the repository's separate-test-file convention:

- Session tests prove Dev Container classification is remote for equal and unequal hostnames,
  preserves SSH/local classification, attaches the manager `HostId`, selects the remote executor,
  and clears identity on disconnect.
- Docker transport tests assert exact executable/argv/stdin mode, user/workdir propagation,
  shell-safe values, owner-only install paths, direct-install and client-copy fallback behavior,
  version mismatch removal/reinstall, reconnectability, cleanup, and absence of credentials in
  commands and logs.
- Dev Container view/operation tests prove the handshake precedes
  `ReplaceDevContainerBuildPane`; every setup failure retains the failed build pane and Retry/Close;
  cancellation/retry deregisters old state; late completions cannot replace a pane.
- Remote consumer tests use a fake host connection to prove `location_for_path`, `read_files`,
  create/update/delete edits, working-directory/repository detection, and remote `DiffStateModel`
  routing retain the same `SessionId` and `HostId`; disconnects must error without local fallback.
- Existing executor/index tests prove `grep`/`file_glob` are unchanged and `search_codebase` follows
  only existing remote-index availability.

Run:

- `cargo nextest run -p warp --features local_tty`
- `cargo nextest run -p remote_server`
- `./script/format`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`

Manual acceptance: open a Dev Container whose repository exists only in the container or a named
volume. Confirm `read_files`, create/update/delete edits, `grep`, `file_glob`, repository detection,
and code review use container contents and container git. Stop the container and confirm those
operations fail without reading or writing a host path.

## Parallelization

Implement transport/bootstrap/identity first because every consumer depends on the resulting
`HostId`; consumer tests can then be added in parallel without overlapping production ownership.
