use std::collections::HashMap;

use regex::Regex;
use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::subshell::{
    accept_tmux_install, assert_subshell_banner_is_showing, assert_subshell_is_bootstrapped,
    enter_ssh_command, enter_ssh_password, run_exit_command, setup_gcloud_sdk,
    trigger_subshell_bootstrap, wait_for_password_prompt,
};
use warp::integration_testing::terminal::util::{
    current_shell_starter_and_version, nonce, ExactLine, ExpectedExitStatus,
};
use warp::integration_testing::terminal::{
    assert_active_block_output_for_single_terminal_in_tab,
    assert_long_running_block_executing_for_single_terminal_in_tab,
    execute_command_for_single_terminal_in_tab, validate_block_output,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{single_terminal_view, single_terminal_view_for_tab};
use warp::terminal::model::bootstrap::BootstrapStage;
use warp::terminal::session_settings::{StartupShell, StartupShellOverride};
use warp::terminal::shell::ShellType;
use warpui::integration::{AssertionCallback, AssertionOutcome, TestStep};
use warpui::{async_assert, async_assert_eq};

use super::new_builder;
use crate::Builder;

/// Verifies that the active block is part of a remote session.
fn assert_active_block_is_remote(user: &'static str, host: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, ctx| {
            let model = view.model.lock();
            let active_block = model.block_list().active_block();

            let Some(session_id) = active_block.session_id() else {
                return AssertionOutcome::PreconditionFailed(
                    "Active block returned None from shell_host()".into(),
                );
            };
            let Some(session) = view.sessions(ctx).get(session_id) else {
                return AssertionOutcome::PreconditionFailed(
                    "Active block should be part of a known session".into(),
                );
            };

            match async_assert!(
                !session.is_local(),
                "Active block should be part of a remote session"
            ) {
                AssertionOutcome::Success => {}
                failure => return failure,
            };

            let Some(shell_host) = active_block.shell_host() else {
                return AssertionOutcome::PreconditionFailed(
                    "Active block returned None from shell_host()".into(),
                );
            };

            match async_assert_eq!(
                shell_host.user,
                user,
                "Remote session did not have the expected user"
            ) {
                AssertionOutcome::Success => {}
                failure => return failure,
            };

            async_assert_eq!(
                shell_host.hostname,
                host,
                "Remote session did not have the expected host"
            )
        })
    })
}

/// Assertion that the MotD message is shown. How we expect it to be shown
/// depends on whether or not the remote shell can be bootstrapped.
fn assert_motd_shown(bootstrapped: bool) -> AssertionCallback {
    let motd_regex = Regex::new("Welcome to Ubuntu").expect("Regex should compile");
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view(app, window_id);
        terminal_view.read(app, |view, _| {
            let model = view.model.lock();

            let motd_output = if bootstrapped {
                let motd_block = model
                    .block_list()
                    .blocks()
                    .iter()
                    .rev()
                    .find(|block| block.bootstrap_stage() == BootstrapStage::ScriptExecution)
                    .expect("MotD block should exist");
                // Because of how we move blocks through their stages, output from
                // RC files and the MotD goes into the command grid.
                motd_block.command_to_string()
            } else {
                // For non-bootstrapped blocks, the MotD is part of the SSH
                // session output in the active block.
                model.block_list().active_block().output_to_string()
            };

            async_assert!(
                motd_regex.is_match(&motd_output),
                "Expected output to match {motd_regex:?}, but was:\n{motd_output:?}"
            )
        })
    })
}

/// Verifies that the current session is using a login shell.
fn verify_login_shell(shell: &str) -> TestStep {
    let command = match shell {
        "zsh" => "[[ -o login ]]",
        "fish" => "status --is-login",
        // For other shells, we don't actually start a login shell but do source /etc/profile.
        _ => "test \"$WARP_PROFILE_LOADED\" = true",
    };

    match shell {
        "bash" | "zsh" => execute_command_for_single_terminal_in_tab(
            0,
            command.into(),
            ExpectedExitStatus::Success,
            (),
        )
        .add_assertion(assert_motd_shown(true /* bootstrapped */)),
        _ => {
            // For non-bootstrapped shells, run the command directly and verify
            // the exit status.
            let nonce = nonce();
            let expected_output = ExactLine::from(format!("{nonce}: 0"));

            TestStep::new("Verify login shell")
                .with_typed_characters(&[&format!("{command}; echo \"{nonce}\": $?")])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    expected_output,
                    0,
                ))
                .add_assertion(assert_motd_shown(false /* bootstrapped */))
        }
    }
}

/// A macro to generate a test function to validate that we are able to
/// bootstrap a given remote shell when using ssh.
macro_rules! generate_can_bootstrap_legacy_ssh_test_for_shell {
    ($fn_name:ident, $shell:literal) => {
        /// Ensure we can successfully ssh into a $shell remote shell and bootstrap it
        /// successfully.
        pub fn $fn_name() -> Builder {
            new_builder()
                // TODO(CORE-2333) PowerShell has no SSH wrapper.
                .set_should_run_test(|| {
                    if FeatureFlag::SSHTmuxWrapper.is_enabled() {
                        return false;
                    }
                    let (starter, _) = current_shell_starter_and_version();
                    starter.shell_type() != ShellType::PowerShell
                })
                .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                .with_step(setup_gcloud_sdk())
                .with_step(enter_ssh_command($shell))
                .with_step(wait_for_password_prompt(0 /*tab_idx*/, $shell))
                .with_step(
                    enter_ssh_password().set_post_step_pause(std::time::Duration::from_millis(250)),
                )
                .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                .with_step(
                    new_step_with_default_assertions(
                        "Assert active block is part of a remote session",
                    )
                    .add_assertion(assert_active_block_is_remote($shell, "ubuntu-14-04")),
                )
                .with_step(verify_login_shell($shell))
        }
    };
}

/// A macro to generate a test function to validate that we are able to
/// bootstrap a given remote shell when using ssh.
macro_rules! generate_can_bootstrap_tmux_ssh_test_for_shell {
    ($fn_name:ident, $shell:literal, $install_tmux:literal) => {
        /// Ensure we can successfully ssh into a $shell remote shell and bootstrap it
        /// successfully.
        pub fn $fn_name() -> Builder {
            fn warpify(builder: Builder) -> Builder {
                builder
                    .with_step(enter_ssh_command($shell))
                    .with_step(wait_for_password_prompt(0 /*tab_idx*/, $shell))
                    .with_step(
                        enter_ssh_password()
                            .set_post_step_pause(std::time::Duration::from_millis(250)),
                    )
                    .with_step(assert_subshell_banner_is_showing())
                    .with_step(trigger_subshell_bootstrap())
            }

            fn assert_warpification(builder: Builder) -> Builder {
                builder
                    .with_step(assert_subshell_is_bootstrapped(0, 0))
                    .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                    .with_step(
                        new_step_with_default_assertions(
                            "Assert active block is part of a remote session",
                        )
                        .add_assertion(assert_active_block_is_remote($shell, "ubuntu-14-04")),
                    )
                    .with_step(verify_login_shell($shell))
            }

            let builder = new_builder()
                // TODO(CORE-2333) PowerShell has no SSH wrapper.
                .set_should_run_test(|| {
                    if !FeatureFlag::SSHTmuxWrapper.is_enabled() {
                        return false;
                    }
                    let (starter, _) = current_shell_starter_and_version();
                    starter.shell_type() != ShellType::PowerShell
                })
                .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                .with_step(setup_gcloud_sdk());
            // Install Tmux
            let builder = warpify(builder).with_step(
                accept_tmux_install().set_post_step_pause(std::time::Duration::from_secs(3)),
            );
            // Quit SSH Session once we validate warpificaiton works with Tmux Install
            let builder = assert_warpification(builder).with_step(run_exit_command());

            // Validate we can Warpify when Tmux is already installed
            assert_warpification(warpify(builder))
        }
    };
}

/// A macro to generate a test function to validate that we are able to
/// successfully start (but not bootstrap) an ssh connection with the given
/// remote shell.  Verifies that the ssh connection worked by asserting that
/// there is still a long-running block after entering the password, and that
/// attempting to run `exit` returns us to the bootstrapped local shell.
macro_rules! generate_long_running_block_ssh_test_for_shell {
    ($fn_name:ident, $shell:literal, prompt_regex: $prompt_regex:literal) => {
        /// Ensure we can successfully ssh into a $shell remote shell and bootstrap it
        /// successfully.
        pub fn $fn_name() -> Builder {
            new_builder()
                // TODO(CORE-2333) PowerShell has no SSH wrapper.
                .set_should_run_test(|| {
                    let (starter, _) = current_shell_starter_and_version();
                    starter.shell_type() != ShellType::PowerShell
                })
                .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                .with_step(setup_gcloud_sdk())
                .with_step(enter_ssh_command($shell))
                .with_step(wait_for_password_prompt(0 /*tab_idx*/, $shell))
                .with_step(enter_ssh_password())
                .with_step(
                    TestStep::new("Assert prompt is awaiting input")
                        .add_assertion(
                            assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                        )
                        .add_assertion(move |app, window_id| {
                            let regex = Regex::new($prompt_regex)
                                .expect("regex should not fail to compile");
                            validate_block_output(&regex, 0, 0, window_id, app)
                        }),
                )
                .with_step(verify_login_shell($shell))
                .with_step(TestStep::new("Exit ssh session").with_typed_characters(&["exit\n"]))
                .with_step(new_step_with_default_assertions(
                    "Assert ssh session has completed",
                ))
        }
    };
}

// Generate test methods to validate expected ssh behavior for a variety of
// remote shells.
generate_can_bootstrap_legacy_ssh_test_for_shell!(test_legacy_ssh_into_bash, "bash");
generate_can_bootstrap_legacy_ssh_test_for_shell!(test_legacy_ssh_into_zsh, "zsh");
generate_can_bootstrap_tmux_ssh_test_for_shell!(test_tmux_ssh_into_bash, "bash", false);
generate_can_bootstrap_tmux_ssh_test_for_shell!(test_tmux_ssh_into_zsh, "zsh", false);
generate_can_bootstrap_tmux_ssh_test_for_shell!(test_install_tmux_ssh_into_bash, "bash", true);
generate_can_bootstrap_tmux_ssh_test_for_shell!(test_install_tmux_ssh_into_zsh, "zsh", true);
generate_long_running_block_ssh_test_for_shell!(test_ssh_into_fish, "fish", prompt_regex: r"\nfish@ubuntu-14-04 ~>$");
generate_long_running_block_ssh_test_for_shell!(test_ssh_into_sh, "sh", prompt_regex: r"\n\$ $");
generate_long_running_block_ssh_test_for_shell!(test_ssh_into_ash, "ash", prompt_regex: r"\n\$ $");

/// Tests a regression with the startup shell setting and SSH proxies.
/// See WAR-6337 for details - if `$SHELL` is not set to a valid executable file
/// path, SSH fails to execute proxy commands (like the one this test uses for
/// gcloud).
pub fn test_ssh_with_shell_override() -> Builder {
    new_builder()
        // TODO(CORE-2333) PowerShell has no SSH wrapper.
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            starter.shell_type() != ShellType::PowerShell
        })
        .with_user_defaults(HashMap::from([(
            StartupShellOverride::storage_key().to_owned(),
            serde_json::to_string(&StartupShell::Zsh).expect("Can serialize setting as JSON"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(setup_gcloud_sdk())
        .with_step(enter_ssh_command("bash"))
        .with_step(wait_for_password_prompt(0, "bash"))
        .with_step(enter_ssh_password())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert active block is part of a remote session")
                .add_assertion(assert_active_block_is_remote("bash", "ubuntu-14-04")),
        )
        .with_step(verify_login_shell("bash"))
}
