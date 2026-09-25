# TECH.md — Support warpctrl (local control) on Windows

Issue: https://github.com/warpdotdev/warp/issues/16158
Product spec: `specs/GH16158/product.md`

## Context

Local control has three stages (see `specs/warp-control-cli/TECH.md`):
1. An owner-only filesystem discovery record points to a loopback HTTP endpoint and an instance-bound credential broker.
2. The broker authenticates the OS user and issues a short-lived credential scoped to one action.
3. The client presents that credential to the HTTP endpoint.

Only stages 1 and 2 are platform-specific. On Windows both currently fail closed (line numbers at `9fb32eddd`):

- `app/src/local_control/mod.rs:642` — `local_control_publication_supported()` returns `false` on Windows, so `refresh_for_settings` never starts the server.
- `app/src/local_control/mod.rs:240,314-431` — the broker exists only for Unix: it binds a `UnixListener`, sets the socket to `0600`, and checks the peer with `peer_cred()` (`ensure_same_user_peer`, line 410).
- `crates/local_control/src/client.rs:162-172` — on non-Unix platforms `request_credential_over_owner_ipc` returns `LocalControlDisabled`.
- `crates/local_control/src/discovery.rs:462-495` — `set_private_dir_permissions` / `set_private_permissions` return `LocalControlDisabled` on non-Unix, so `RegisteredInstance::register` fails.
- `crates/local_control/src/discovery.rs:280` — `discovery_dir()` falls back to `HOME` (usually unset on Windows), then `.`.
- `crates/local_control/src/discovery.rs:316` — `list_instances_from_dir` trusts any parseable record. On Unix that is safe because publication sets `0700`/`0600`. On Windows, new directories inherit broader ACLs by default.
- Protected Scripting storage already exists: `crates/warpui_extras/src/secure_storage/windows.rs` uses DPAPI.

## Proposed changes

### `crates/local_control/src/windows_security.rs` (new, `cfg(windows)`)

Windows primitives shared by the app and the client:

- `current_user_sid()` and `process_user_sid(pid)`: token user SID as a string (`OpenProcessToken`, `GetTokenInformation(TokenUser)`, `ConvertSidToStringSidW`).
- `ensure_process_user(pid, expected_sid)`: the Windows counterpart of `ensure_peer_uid`. Returns `UnauthorizedLocalClient` on mismatch.
- `pipe_client_process_id(handle)` and `pipe_server_process_id(handle)`: `GetNamedPipeClientProcessId` / `GetNamedPipeServerProcessId`.
- `set_private_acl(path, is_directory)`: applies a protected DACL, `D:P(A;[OICI];FA;;;<user>)(A;[OICI];FA;;;SY)(A;[OICI];FA;;;BA)`, with `SetNamedSecurityInfoW(DACL | PROTECTED_DACL)`. Directory entries are inheritable.
- `validate_private_acl(path)`: rejects a DACL that is not protected, a NULL DACL, an owner outside the allowed set, any allow ACE for another SID, and any unsupported ACE type. Deny ACEs are accepted, since they only narrow access.
- `BrokerPipeSecurity`: owns a `SECURITY_ATTRIBUTES` whose descriptor is `D:P(A;;GA;;;<user>)`, for pipe creation.

### `crates/local_control/src/discovery.rs`

- `discovery_dir()`: on Windows, use `%LOCALAPPDATA%\warp\local-control` when `WARP_LOCAL_CONTROL_DISCOVERY_DIR` is unset.
- `set_private_dir_permissions` / `set_private_permissions`: on Windows, apply and then re-validate the protected DACL. Other non-Unix platforms still fail closed.
- `list_instances_from_dir`: on Windows, return nothing if the directory ACL fails validation, and prune records whose own ACL fails, the same way malformed records are pruned. This is a no-op on Unix.
- `broker_socket_path()`: on Windows, map the validated, instance-derived broker filename to `\\.\pipe\warp-local-control-<filename>`. The record format and `validate_local_control_authority` are unchanged, so records stay compatible across platforms.

### `crates/local_control/src/client.rs`

- `CREDENTIAL_REQUEST_DELIMITER` (`\n`) and `MAX_CREDENTIAL_REQUEST_BYTES` (64 KiB) are shared with the app. Named pipes can't shut down just their write half, so requests are newline-terminated instead. Serialized JSON never contains a raw newline.
- `request_credential_over_pipe` (Windows):
  1. Opens the pipe with `security_qos_flags(SECURITY_IDENTIFICATION)`. While all pipe instances are busy (`ERROR_PIPE_BUSY`), it retries 20 times at 50 ms intervals.
  2. Checks that `GetNamedPipeServerProcessId` equals `instance.pid` and that this process runs as the current user. Only then does it write the request.
  3. Reads until the server closes its end. `ERROR_BROKEN_PIPE` is treated as the end of the response.

### `app/src/local_control/mod.rs`

- `local_control_publication_supported()` returns `cfg!(any(unix, windows))`.
- The `cfg(unix)` gates on the broker listener, `issue_credential`, `insert_credential`, and `serialize_credential_broker_response` also include Windows.
- Windows broker:
  - `bind_credential_broker` creates the first pipe instance with `first_pipe_instance(true)`, `reject_remote_clients(true)`, and `BrokerPipeSecurity`, so a pre-existing pipe name makes startup fail instead of being shared.
  - `run_credential_broker` accepts a connection, creates the next pipe instance, then handles the accepted client on its own task.
  - `handle_credential_broker_connection` runs `ensure_same_user_peer`, which compares the client process's SID with ours, before `read_credential_request`. That function enforces the delimiter and the size limit. Credential issuance is then shared with Unix.

### Dependencies

- `local_control` gets a Windows-only `windows` dependency (features `Win32_Foundation`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Pipes`, `Win32_System_SystemServices`, `Win32_System_Threading`), and a Windows-only `tokio` dev-dependency for the pipe tests. The app already depends on `tokio` named pipes through `net`.

## Testing and validation

Unit tests (Windows only unless noted), by product invariant:

| Invariant | Test |
|---|---|
| 4 | `windows_security_tests::private_acl_is_accepted_after_protection`, `private_sddl_grants_only_owner_system_and_administrators`, `discovery_tests::published_record_has_private_acl_on_windows` |
| 5 | `windows_security_tests::inherited_acl_is_rejected`, `record_with_only_inherited_acl_is_rejected`, `discovery_tests::unprotected_registry_is_not_trusted_on_windows`, `unprotected_record_is_pruned_on_windows`, `malformed_record_and_matching_broker_socket_are_pruned` (all platforms) |
| 6 | `discovery_tests::broker_reference_resolves_to_instance_bound_pipe_on_windows`, `windows_security_tests::broker_pipe_security_builds_attributes` |
| 7 | `mod_tests::credential_broker_rejects_pipe_peer_from_different_user`, `credential_broker_accepts_pipe_peer_from_same_user`, `windows_security_tests::process_user_check_*` |
| 8 | `client_tests::credential_client_rejects_pipe_served_by_unexpected_process` (asserts that nothing is sent) |
| 10 | `mod_tests::credential_request_reader_stops_at_delimiter`, `credential_request_reader_rejects_truncated_request`, `credential_request_reader_rejects_oversized_request` |
| 1, 3 | `client_tests::credential_client_exchanges_request_over_broker_pipe`, plus the existing catalog and auth tests, which are unchanged |
| 13 | The existing Unix tests are unchanged, and CI runs them on macOS and Linux |

Commands:
- `cargo nextest run -p local_control`
- `cargo nextest run -p warp --features warp_control_cli -E 'test(local_control)'`
- `cargo clippy -p local_control --all-targets --tests -- -D warnings`
- `cargo clippy -p warp --features warp_control_cli --lib --tests -- -D warnings`

Manual end-to-end check on Windows 11, with `warp-oss --features warp_control_cli` and Scripting enabled:
- `--warpctrl instance list`, `app active`, `pane list`, `pane split`, `pane focus`, `pane rename`, and `pane close` all succeed.
- `icacls` on the discovery directory shows only the user, SYSTEM, and Administrators, with no inherited entries.
- With Scripting disabled, no instance is listed.

## Risks and mitigations

- **Pipe-name squatting by another user.** Instance IDs are random and published only in the owner-only directory. The server requires the first pipe instance, and the client verifies the server's PID and SID before sending anything, at identification-level QoS.
- **PID reuse between reading the client PID and opening its token.** The pipe DACL already stops other users from connecting, so the SID check is defense in depth. A reused PID owned by another user would fail it.
- **Same-user malware.** Out of scope, as on Unix: the broker authenticates the OS account, not the calling program.
- **`is_pid_alive` on Windows still shells out to `tasklist`.** This is slow, but correct and unchanged here. It's a candidate for a follow-up that uses `OpenProcess` instead.

## Follow-ups

- Ship a `warpctrl` wrapper on Windows, in the installer or next to `warp.exe`, and wire up the **Install Warp Control CLI command** Scripting widget on Windows.
- Replace the `tasklist` liveness check with `OpenProcess` plus `GetExitCodeProcess`.
