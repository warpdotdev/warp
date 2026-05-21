# Remote Server Integration Tests

## Context

The `SshRemoteServer` feature flag gates a new SSH session flow where a persistent binary (`remote-server-proxy`) runs on the remote host, replacing the legacy ControlMaster-based command execution. The feature has unit test coverage at the protocol layer (`crates/remote_server/src/client_tests.rs`) but no integration test coverage exercising the full client ↔ server lifecycle over a real SSH connection.

### Current integration test infra

The existing SSH integration tests (`crates/integration/src/test/ssh.rs`) cover the legacy warpification flow:
- Connect to a GCP-hosted Ubuntu VM (`ubuntu-14-04`) via IAP tunnel with password auth
- Helper steps in `app/src/integration_testing/subshell/` — `setup_gcloud_sdk()`, `enter_ssh_command()`, `enter_ssh_password()`, `wait_for_password_prompt()`
- Builder pattern: `new_builder().with_step(TestStep)` with assertion callbacks
- Feature flag gating via `set_should_run_test(|| FeatureFlag::X.is_enabled())`

### Remote server flow

When `SshRemoteServer` is enabled for a legacy SSH session (`app/src/terminal/writeable_pty/remote_server_controller.rs`):

1. `RemoteServerController` intercepts `SshInitShell`, stashes the bootstrap script
2. Runs `check_binary` via `RemoteServerManager` → if missing, `install_binary` → then `connect_session`
3. `connect_session` (`crates/remote_server/src/manager.rs:368`) spawns the proxy over SSH, performs the proto `Initialize` handshake → emits `SessionConnected { host_id }`
4. Bootstrap is flushed; `RemoteServerCommandExecutor` (`app/src/terminal/model/session/command_executor/remote_server_executor.rs`) is wired as the session's `CommandExecutor`
5. On CWD change, `navigate_to_directory` fires → returns `is_git` flag + triggers `RepoMetadataSnapshot` push

Key config: `SshExtensionInstallMode::AlwaysInstall` (setting in `app/src/terminal/warpify/settings.rs:85`) bypasses the choice block UI, needed for deterministic test flow.

### Binary deployment problem

The production install script (`crates/remote_server/src/install_remote_server.sh`) downloads from the CDN (`app.warp.dev/download/cli`). This fetches a published binary, not one built from the developer's branch. For integration tests, the binary must come from the current codebase so that changes to the remote-server protocol or logic are tested. The existing `script/deploy_remote_server` already solves this for local development — it cross-compiles for `x86_64-unknown-linux-musl` and uploads via rsync.

## Proposed changes

### 1. CI step: cross-compile and deploy binary to test VM

Add `script/deploy_remote_server_to_test_vm` that:

1. Cross-compiles the Oz CLI for the test VM target:
   ```
   cargo build -p warp --bin warp --target x86_64-unknown-linux-musl \
     --profile dev-remote \
     --features release_bundle,crash_reporting,standalone,agent_mode_debug
   ```
   Same build command as `script/deploy_remote_server`.

2. Uploads the binary to `ubuntu-14-04` at `~/.warp-dev/remote-server/oz-dev` via `sshpass` + `scp` through the GCP IAP tunnel (the test VM uses password auth; `sshpass` provides it non-interactively). Uses the same proxy command as the SSH integration tests (`app/src/integration_testing/subshell/util.rs:2`).

CI calls this script once before launching the integration test suite. Since `check_binary` will find the binary already present, the `RemoteServerController` flow becomes `check_binary → Ok(true) → connect_session`, skipping the CDN-based install.

### 2. Assertion helpers — `app/src/integration_testing/remote_server.rs`

New module with reusable test steps and action helpers:

- **`wait_for_remote_server_ready(tab_idx)`** — `TestStep` that polls until `Sessions::remote_server_setup_states` for the active session reaches `RemoteServerSetupState::Ready`.
- **`assert_remote_server_connected(tab_idx)`** — reads `RemoteServerManager` singleton, asserts the active session is in `RemoteSessionState::Connected`.
- **`assert_command_executor_is_remote_server(tab_idx)`** — downcasts the session's `CommandExecutor` via `as_any().downcast_ref::<RemoteServerCommandExecutor>()`.
- **`assert_remote_server_has_navigated(tab_idx)`** — asserts that `host_id_for_session` is populated for the active session.
- **`write_file_via_remote_server(tab_idx, path, content)`** — action callback that calls `RemoteServerClient::write_file` on a background thread (async → sync bridge via `tokio::runtime::Runtime::block_on`).
- **`load_repo_metadata_directory_via_remote_server(tab_idx, repo_path, dir_path)`** — action callback that calls `RemoteServerManager::load_remote_repo_metadata_directory` through the model handle.

Also adds `Session::command_executor()` accessor gated on `#[cfg(any(test, feature = "integration_tests"))]` (`app/src/terminal/model/session.rs`).

Registered in `app/src/integration_testing/mod.rs`.

### 3. Integration tests — `crates/integration/src/test/remote_server.rs`

All tests gated on `FeatureFlag::SshRemoteServer.is_enabled()` and configured with `with_user_defaults` setting `SshExtensionInstallMode` to `AlwaysInstall`.

#### Test A — Connection and handshake (`test_remote_server_connect_bash` / `_zsh`)

Validates the core flow: SSH → binary check → proto handshake → executor wiring.

Steps:
1. `wait_until_bootstrapped_single_pane_for_tab(0)` — local shell ready
2. `setup_gcloud_sdk()`
3. `enter_ssh_command(shell)` + `wait_for_password_prompt` + `enter_ssh_password`
4. `wait_for_remote_server_ready(0)` — covers check → connect → handshake
5. `wait_until_bootstrapped_single_pane_for_tab(0)` — remote shell bootstrapped
6. `assert_remote_server_connected(0)` — manager has `Connected` state with a `HostId`
7. `assert_command_executor_is_remote_server(0)` — session uses `RemoteServerCommandExecutor`

#### Test B — Repo metadata (`test_remote_server_navigate_to_repo`)

Validates the full navigate-to-directory flow: create git repo on remote → `cd` into it → `NavigatedToDirectory` response received → session has `host_id` tracked.

After Test A setup, plus:
8. Create a git repo on the remote via `execute_command("mkdir -p /tmp/warp-test-repo && cd /tmp/warp-test-repo && git init -b main ...")`
9. `execute_command("cd /tmp/warp-test-repo")` — triggers CWD change → `navigate_to_directory`
10. `assert_remote_server_has_navigated(0)` — host_id present for active session
11. `assert_remote_server_connected(0)` — session still healthy after navigation

#### Test C — Completions routing (`test_remote_server_completions`)

Validates that completions run through `RemoteServerCommandExecutor::execute_command` (the `RunCommand` proto path) rather than falling back to the legacy `RemoteCommandExecutor`.

After Test A setup, plus:
8. Run a command to trigger completions loading
9. `assert_command_executor_is_remote_server(0)` — confirm executor type

#### Test D — File write via proto client API (`test_remote_server_file_operations`)

Validates `WriteFile` through the `RemoteServerClient` proto API. Uses `write_file_via_remote_server` helper to dispatch the async write from an action callback, then reads the file back via a shell command (which goes through `RemoteServerCommandExecutor::run_command`) to confirm content integrity.

After Test A setup, plus:
8. `write_file_via_remote_server(0, "/tmp/warp-rs-test-file.txt", "hello from proto")` — writes via proto
9. `execute_command("cat /tmp/warp-rs-test-file.txt")` — reads back via `RunCommand`, asserts `"hello from proto"`
10. Clean up and verify executor type

#### Test E — Lazy loading repo metadata (`test_remote_server_lazy_load_directory`)

Validates the `LoadRepoMetadataDirectory` proto round-trip: navigate to a git repo → create a subdirectory → call `load_remote_repo_metadata_directory` for that subdirectory → verify the response flows through without error.

After Test A setup, plus:
8. Create a git repo with a `subdir/nested` file on the remote
9. `cd /tmp/warp-lazy-repo` → triggers `NavigatedToDirectory` and full indexing
10. `load_repo_metadata_directory_via_remote_server(0, repo_path, "subdir")` — triggers lazy-load proto request
11. `assert_remote_server_connected(0)` — connection still healthy
12. `execute_command("cat subdir/nested")` — verifies subdirectory content accessible

### 4. Wire into the test runner

- Add `mod remote_server;` + `pub use remote_server::*;` in `crates/integration/src/test.rs`
- Register test functions in `crates/integration/tests/integration/shell_integration_tests.rs`
- Register in `crates/integration/src/bin/integration.rs` for the manual runner

## Testing and validation

The tests in this spec *are* the validation — they verify the remote-server feature works end-to-end over a real SSH connection. Specifically:

- **Test A** proves the install-check → handshake → executor-wiring pipeline works against a real remote host, catching protocol mismatches or connection failures that unit tests with mock streams cannot.
- **Test B** proves the `NavigatedToDirectory` → repo metadata pipeline works through the proto, catching serialization issues or git-detection regressions on the remote.
- **Test C** proves completions are routed through the remote-server binary rather than silently falling back to the legacy ControlMaster executor, which would mask remote-server regressions.
- **Test D** proves the `WriteFile` proto message works end-to-end by writing via the client API and reading back via `RunCommand`, catching serialization or `FileModel` regressions invisible to shell-only tests.
- **Test E** proves the `LoadRepoMetadataDirectory` lazy-loading proto round-trip works, catching regressions in the subdirectory expansion path that is distinct from the initial `NavigatedToDirectory` indexing.

All tests run against a binary built from the current branch (via the CI deploy step), ensuring protocol changes are tested before merge.

## Parallelization

- **CI script** (step 1) and **assertion helpers** (step 2) can be built in parallel — they touch disjoint files.
- **Test module** (step 3) depends on the assertion helpers but the test groups (A–E) are independent functions that can be written in parallel once the shared helpers exist.
- **Test runner wiring** (step 4) is trivial and can be done alongside any other step.
