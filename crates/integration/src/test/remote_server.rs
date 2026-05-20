use std::collections::HashMap;
use std::time::Duration;

use settings::Setting as _;
use warp::{
    features::FeatureFlag,
    integration_testing::{
        remote_server::{
            assert_command_executor_is_remote_server, assert_remote_server_connected,
            assert_remote_server_has_navigated,
            assert_remote_server_loaded_repo_metadata_directory,
            load_repo_metadata_directory_via_remote_server, record_remote_server_lazy_load_events,
            record_remote_server_navigation_events, wait_for_remote_server_ready,
            write_file_via_remote_server,
        },
        step::new_step_with_default_assertions,
        subshell::{
            enter_remote_server_ssh_command, enter_ssh_password, setup_gcloud_sdk,
            wait_for_remote_server_password_prompt,
        },
        terminal::{
            execute_command_for_single_terminal_in_tab, run_completer,
            util::{current_shell_starter_and_version, ExpectedExitStatus},
            wait_until_bootstrapped_single_pane_for_tab,
        },
    },
    terminal::{
        shell::ShellType,
        warpify::settings::{SshExtensionInstallMode, SshExtensionInstallModeSetting},
    },
};
use warpui::integration::TestStep;

use super::{new_builder, Builder};

/// Common builder configuration for remote server tests: enables the
/// `SshRemoteServer` feature flag for these tests and sets the install mode to
/// `AlwaysInstall` so the binary check → connect flow runs without user
/// interaction.
fn remote_server_builder() -> Builder {
    FeatureFlag::SshRemoteServer.set_enabled(true);
    new_builder()
        .set_should_run_test(|| {
            if !cfg!(target_os = "linux") {
                return false;
            }
            let (starter, _) = current_shell_starter_and_version();
            starter.shell_type() != ShellType::PowerShell
        })
        .with_user_defaults(HashMap::from([(
            SshExtensionInstallModeSetting::storage_key().to_owned(),
            serde_json::to_string(&SshExtensionInstallMode::AlwaysInstall)
                .expect("Can serialize SshExtensionInstallMode"),
        )]))
}

/// Appends the common SSH → remote server connection steps to a builder.
fn with_ssh_connect_steps(builder: Builder, shell: &'static str) -> Builder {
    builder
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(setup_gcloud_sdk())
        .with_step(enter_remote_server_ssh_command(shell))
        .with_step(wait_for_remote_server_password_prompt(0, shell))
        .with_step(enter_ssh_password().set_post_step_pause(Duration::from_millis(250)))
        .with_step(wait_for_remote_server_ready(0))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
}

// ---------------------------------------------------------------------------
// Test A — Connection and handshake
// ---------------------------------------------------------------------------

macro_rules! generate_remote_server_connect_test {
    ($fn_name:ident, $shell:literal) => {
        /// Validates SSH → binary check → proto handshake → executor wiring.
        pub fn $fn_name() -> Builder {
            let builder = with_ssh_connect_steps(remote_server_builder(), $shell);
            builder
                .with_step(
                    new_step_with_default_assertions("Assert remote server session is connected")
                        .add_assertion(assert_remote_server_connected(0)),
                )
                .with_step(
                    new_step_with_default_assertions(
                        "Assert command executor is RemoteServerCommandExecutor",
                    )
                    .add_assertion(assert_command_executor_is_remote_server(0)),
                )
        }
    };
}

generate_remote_server_connect_test!(test_remote_server_connect_bash, "bash");
generate_remote_server_connect_test!(test_remote_server_connect_zsh, "zsh");

// ---------------------------------------------------------------------------
// Test B — Repo metadata (navigate to a git repo)
// ---------------------------------------------------------------------------

/// Validates the full navigate-to-directory flow:
/// SSH → create git repo on remote → cd into it → NavigatedToDirectory response
/// received → RepoMetadataSnapshot pushed with non-empty tree.
pub fn test_remote_server_navigate_to_repo() -> Builder {
    let builder = with_ssh_connect_steps(remote_server_builder(), "bash");
    builder
        .with_step(record_remote_server_navigation_events())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir -p /tmp/warp-test-repo && cd /tmp/warp-test-repo && git init -b main && git config user.email test@test.com && git config user.name TestUser && touch file && git add file && git commit -m init".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cd /tmp/warp-test-repo".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        // Assert the remote server received a successful navigation response
        // for the expected repo path.
        .with_step(
            new_step_with_default_assertions(
                "Assert remote server has navigated to directory",
            )
            .set_timeout(Duration::from_secs(15))
            .add_named_assertion_with_data_from_prior_step(
                "remote server navigated to expected repo path",
                assert_remote_server_has_navigated(0, "/tmp/warp-test-repo"),
            ),
        )
        // Verify the connection is still healthy after navigation.
        .with_step(
            new_step_with_default_assertions(
                "Assert host has tracked sessions after navigation",
            )
            .add_assertion(assert_remote_server_connected(0)),
        )
}

// ---------------------------------------------------------------------------
// Test C — Completions routing
// ---------------------------------------------------------------------------

/// Validates that completions are routed through the `RemoteServerCommandExecutor`
/// (the `RunCommand` proto path) rather than falling back to the legacy
/// `RemoteCommandExecutor`.
pub fn test_remote_server_completions() -> Builder {
    let builder = with_ssh_connect_steps(remote_server_builder(), "bash");
    builder
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "touch /tmp/warp-rs-completion-target".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        // Trigger the actual tab-completion request path for a remote file.
        .with_step(
            run_completer(0, "cat /tmp/warp-rs-completion-t").set_timeout(Duration::from_secs(15)),
        )
        // Verify the executor is still the remote server executor after
        // completions have been triggered.
        .with_step(
            new_step_with_default_assertions(
                "Assert command executor is RemoteServerCommandExecutor after completions",
            )
            .add_assertion(assert_command_executor_is_remote_server(0)),
        )
}

// ---------------------------------------------------------------------------
// Test D — File read/write/delete via proto client API
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test E — Lazy loading repo metadata directory
// ---------------------------------------------------------------------------

/// Validates the `LoadRepoMetadataDirectory` proto round-trip:
/// navigate to a git repo → create a subdirectory → call
/// `load_remote_repo_metadata_directory` for that subdirectory → verify
/// the request completes without error (the response flows through the
/// manager's `RepoMetadataDirectoryLoaded` event).
pub fn test_remote_server_lazy_load_directory() -> Builder {
    let builder = with_ssh_connect_steps(remote_server_builder(), "bash");
    builder
        .with_step(record_remote_server_navigation_events())
        .with_step(record_remote_server_lazy_load_events())
        // Create a git repo with a subdirectory on the remote.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "rm -rf /tmp/warp-lazy-repo && mkdir -p /tmp/warp-lazy-repo/subdir && cd /tmp/warp-lazy-repo && git init -b main && git config user.email test@test.com && git config user.name TestUser && touch file subdir/nested && git add . && git commit -m init".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        // Navigate into the repo so NavigatedToDirectory fires and the
        // server indexes it.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cd /tmp/warp-lazy-repo".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Wait for navigation to complete",
            )
            .set_timeout(Duration::from_secs(15))
            .add_named_assertion_with_data_from_prior_step(
                "remote server navigated to expected lazy-load repo path",
                assert_remote_server_has_navigated(0, "/tmp/warp-lazy-repo"),
            ),
        )
        // Trigger lazy-load of the subdirectory via the proto API.
        .with_step(
            TestStep::new("Load subdirectory via LoadRepoMetadataDirectory")
                .with_action(load_repo_metadata_directory_via_remote_server(
                    0,
                    "/tmp/warp-lazy-repo".to_string(),
                    "/tmp/warp-lazy-repo/subdir".to_string(),
                ))
                // Give the async request a moment to complete.
                .set_post_step_pause(Duration::from_secs(2)),
        )
        .with_step(
            new_step_with_default_assertions("Assert lazy-load directory response succeeded")
                .set_timeout(Duration::from_secs(15))
                .add_named_assertion_with_data_from_prior_step(
                    "remote server loaded repo metadata directory",
                    assert_remote_server_loaded_repo_metadata_directory(0),
                ),
        )
        // Verify the connection is still healthy after the lazy-load.
        .with_step(
            new_step_with_default_assertions(
                "Assert remote server still connected after lazy load",
            )
            .add_assertion(assert_remote_server_connected(0)),
        )
        // Verify the subdirectory content is accessible by reading
        // the nested file through the remote server executor.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cat /tmp/warp-lazy-repo/subdir/nested && echo 'file exists'".into(),
            ExpectedExitStatus::Success,
            "file exists",
        ))
}

// ---------------------------------------------------------------------------
// Test F — File write via proto client API
// ---------------------------------------------------------------------------

/// Validates `WriteFile` through the `RemoteServerClient` proto API.
/// Uses `write_file_via_remote_server` helper to write a file on the remote
/// host, then reads it back via a shell command (which goes through
/// `RemoteServerCommandExecutor::run_command`) to confirm the content.
pub fn test_remote_server_file_operations() -> Builder {
    let builder = with_ssh_connect_steps(remote_server_builder(), "bash");
    builder
        // Step 1: Write a file using RemoteServerClient::write_file proto API
        .with_step(
            TestStep::new("Write file via RemoteServerClient proto API")
                .with_action(write_file_via_remote_server(
                    0,
                    "/tmp/warp-rs-test-file.txt".to_string(),
                    "hello from proto".to_string(),
                ))
                // Give the async write a moment to complete
                .set_post_step_pause(Duration::from_secs(2)),
        )
        // Step 2: Verify the file was written by reading it via shell command
        // (which goes through RemoteServerCommandExecutor::run_command).
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cat /tmp/warp-rs-test-file.txt".into(),
            ExpectedExitStatus::Success,
            "hello from proto",
        ))
        // Step 3: Clean up
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "rm -f /tmp/warp-rs-test-file.txt".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Assert command executor is still RemoteServerCommandExecutor after file ops",
            )
            .add_assertion(assert_command_executor_is_remote_server(0)),
        )
}
