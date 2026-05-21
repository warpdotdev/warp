use std::collections::HashMap;

use settings::Setting as _;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::subshell::util::ssh_command;
use warp::integration_testing::subshell::{
    assert_subshell_banner_is_showing, assert_subshell_is_bootstrapped,
    enter_local_subshell_command, enter_remote_subshell_command, enter_ssh_password,
    setup_gcloud_sdk, trigger_subshell_bootstrap, wait_for_password_prompt,
};
use warp::integration_testing::terminal::util::current_shell_starter_and_version;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::single_input_view_for_tab;
use warp::root_view::SubshellCommandArg;
use warp::terminal::shell::ShellType;
use warp::terminal::warpify::settings::AddedSubshellCommands;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::windowing::state::ApplicationStage;
use warpui::windowing::WindowManager;
use warpui::{async_assert, UpdateModel};

use super::{new_builder, Builder};
use crate::util::skip_if_powershell_core_2303;

/// Generates an integration test that asserts that a local subshell of the given shell type can be
/// successfully bootstrapped.
macro_rules! generate_can_bootstrap_local_subshell_for_shell {
    ($fn_name:ident, $shell:literal) => {
        /// Ensure a local subshell bootstraps successfully.
        pub fn $fn_name() -> Builder {
            new_builder()
                // We've noticed that these tests sometimes fail due to not
                // cleaning up files after the test, so we use a temp dir
                // to hedge against this.
                .use_tmp_filesystem_for_test_root_directory()
                // TODO(CORE-2730): Re-enable once powershell has subshell support
                .set_should_run_test(skip_if_powershell_core_2303)
                .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                .with_step(enter_local_subshell_command($shell))
                .with_step(assert_subshell_banner_is_showing())
                .with_step(trigger_subshell_bootstrap())
                .with_step(assert_subshell_is_bootstrapped(0, 0))
        }
    };
}

generate_can_bootstrap_local_subshell_for_shell!(test_can_bootstrap_local_bash_subshell, "bash");
generate_can_bootstrap_local_subshell_for_shell!(test_can_bootstrap_local_fish_subshell, "fish");
generate_can_bootstrap_local_subshell_for_shell!(test_can_bootstrap_local_zsh_subshell, "zsh");

macro_rules! generate_can_bootstrap_remote_subshell_for_shell {
    ($fn_name:ident, $shell:literal) => {
        /// Ensure a local subshell bootstraps successfully.
        pub fn $fn_name() -> Builder {
            new_builder()
                // TODO(CORE-2333) PowerShell has no SSH wrapper.
                .set_should_run_test(|| {
                    let (starter, _) = current_shell_starter_and_version();
                    starter.shell_type() != ShellType::PowerShell
                })
                .with_user_defaults(HashMap::from([(
                    AddedSubshellCommands::storage_key().to_owned(),
                    serde_json::to_string(&vec![ssh_command($shell, false)])
                        .expect("Can serialize Vec<String> to string"),
                )]))
                .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
                .with_step(setup_gcloud_sdk())
                .with_step(enter_remote_subshell_command($shell))
                .with_step(wait_for_password_prompt(0 /*tab_idx*/, $shell))
                .with_step(
                    enter_ssh_password().set_post_step_pause(std::time::Duration::from_millis(250)),
                )
                .with_step(trigger_subshell_bootstrap())
                .with_step(assert_subshell_is_bootstrapped(0, 0))
        }
    };
}

generate_can_bootstrap_remote_subshell_for_shell!(test_can_bootstrap_remote_zsh_subshell, "zsh");
generate_can_bootstrap_remote_subshell_for_shell!(test_can_bootstrap_remote_bash_subshell, "bash");
// TODO(CORE-348): Consider upgrading the fish version in the testing VM so we can enable this
// test.
// generate_can_bootstrap_remote_subshell_for_shell!(test_can_bootstrap_remote_fish_subshell, "fish");

// Test the flow of creating a new window and running a command that should create a subshell and
//  automaticall bootstrapping AKA "warpifying" that subshell.
pub fn test_can_auto_bootstrap() -> Builder {
    const SUBSHELL_COMMAND: &str = "zsh";

    new_builder()
        // TODO(CORE-2730): Re-enable once powershell has subshell support
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(TestStep::new("foo").add_assertion(|app, window_id| {
            app.update_model(
                &app.get_singleton_model_handle::<WindowManager>(),
                |window_manager, _| {
                    window_manager.overwrite_for_test(ApplicationStage::Active, Some(window_id));
                    AssertionOutcome::Success
                },
            )
        }))
        .with_step(
            new_step_with_default_assertions("Insert subshell command in new tab").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_new_tab_insert_subshell_command_and_bootstrap_if_supported",
                        SubshellCommandArg {
                            command: SUBSHELL_COMMAND.to_owned(),
                            shell_type: Some(ShellType::Zsh),
                        },
                    );
                },
            ),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Check that subshell command was inserted")
                .add_named_assertion(
                    "Check that input buffer contains the subshell command",
                    |app, window_id| {
                        let input_view = single_input_view_for_tab(app, window_id, 1);
                        input_view.read(app, |input, ctx| {
                            async_assert!(
                                input.buffer_text(ctx) == SUBSHELL_COMMAND,
                                "Subshell command was not inserted"
                            )
                        })
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions("run subshell command").with_keystrokes(&["enter"]),
        )
        .with_step(assert_subshell_is_bootstrapped(1, 0))
}
