//! Integration tests for bootstrapping logic.

use settings::Setting as _;
use version_compare::Cmp;
use warp::{
    cmd_or_ctrl_shift,
    integration_testing::{
        input::{
            input_contains_string, input_editor_is_focused, input_editor_is_not_focused,
            input_is_empty,
        },
        step::new_step_with_default_assertions,
        tab::tab_title_step,
        terminal::{
            assert_active_block_command_for_single_terminal_in_tab,
            assert_long_running_block_executing_for_single_terminal_in_tab,
            execute_command_for_single_terminal_in_tab,
            util::{current_shell_starter_and_version, ExpectedExitStatus},
            wait_until_bootstrapped_single_pane_for_tab,
        },
        view_getters::{single_input_view_for_tab, single_terminal_view_for_tab},
    },
    terminal::session_settings::HonorPS1,
    terminal::shell::{self, ShellType},
    workspace::Workspace,
};
use warpui::{
    async_assert, async_assert_eq, clipboard::ClipboardContent, integration::TestStep, ViewHandle,
};

use crate::util::{write_all_rc_files_for_test, write_rc_files_for_test, ShellRcType};

use super::{new_builder, Builder};

/// Ensures that config files are only sourced once when bootstrapping a new session.
pub fn test_rc_files_only_sourced_once_during_bootstrapping() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_all_rc_files_for_test(dir, r"echo 'foo' >> ~/rc_output");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Currently, when starting the application, we invoke a one-off non-interactive, login
        // shell that is not _associated_ to any particular session (see `LocalShell::new`).
        // Since this test only cares about the fact that the config files are sourced
        // once _per session_, we clear the effects of this one-off initialization
        // and then ensure that starting a session only sources config files once.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "rm ~/rc_output".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Add a new session")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "cat ~/rc_output".to_string(),
            ExpectedExitStatus::Success,
            "foo",
        ))
}

pub fn test_unescaped_prompt_bootstraps() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r"export PATH=/Applications/VMware\\\ Fusion.app/Contents/Public:$PATH",
                [ShellRcType::Bash, ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
}

// TODO: test doesn't work on fish because no block is created from output of config.fish
pub fn test_paste_and_type_characters_before_bootstrap() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            let (starter, _version) = current_shell_starter_and_version();
            !matches!(starter.shell_type(), ShellType::Fish)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(&dir, "echo -n 'Enter some user input: ' && read", [ShellRcType::Zsh, ShellRcType::Bash, ShellRcType::Fish]);
            write_rc_files_for_test(&dir, "Read-Host 'Enter some user input'", [ShellRcType::PowerShell]);
            // On Ubuntu (and possibly other Linux distros), a message is
            // printed out during shell initialization telling the user how to
            // use `sudo`. This interferes with our expected pty contents, so suppress the message.
            if cfg!(any(target_os = "linux", target_os = "freebsd")) {
                std::fs::File::create(dir.join(".sudo_as_admin_successful"))
                    .expect("should not fail to create file in home directory");
            }
        })
        .with_step(
            TestStep::new("Wait for rc file to run")
                .add_named_assertion("Long running block executing", assert_long_running_block_executing_for_single_terminal_in_tab(false, 0))
                // Output pre-bootstrap writes to the block's command, not the output, as nothing causes
                // us to finish the command grid.
                .add_named_assertion("Validate block contents", assert_active_block_command_for_single_terminal_in_tab("Enter some user input: ", 0))
        )
        .with_step(
            TestStep::new("Warp input should not start focused, since .rc file is reading user input")
                .add_assertion(input_editor_is_not_focused(0))
        )
        .with_step(
            TestStep::new("Populate the clipboard")
                .with_action(|app, _, _| {
                    app.update(|app| {
                        app.clipboard().write(ClipboardContent::plain_text("this is the pasted text".to_string()))
                    })
                })
                .add_assertion(|app, _| {
                    app.update(|app| {
                        async_assert_eq!(
                            app.clipboard().read().plain_text,
                            String::from("this is the pasted text"),
                            "Clipboard should be populated",
                        )
                    })
                })
                // Make sure the input is not focused before we paste, even though we checked this in an earlier step.
                // The input is sometimes focused for a brief moment causing the paste to go to the wrong place without this check for some reason.
                .add_assertion(input_editor_is_not_focused(0)),
        )
        .with_step(
            TestStep::new("Pasted text go into the pty and not warp input")
                .with_keystrokes(&[cmd_or_ctrl_shift("v")])
                .add_named_assertion("Input should be empty", input_is_empty(0))
                .add_named_assertion("Pasted text should go to pty", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _| {
                        async_assert_eq!(
                            view.model.lock().block_list().active_block().command_to_string(),
                            String::from("Enter some user input: this is the pasted text"),
                            "Paste should go to pty"
                        )
                    })
                })
                // Make sure the input is not focused before we type, even though we checked this in an earlier step.
                // The input is sometimes focused for a brief moment causing typed characters to go to the wrong place without this check for some reason.
                .add_named_assertion("Input should not be focused", input_editor_is_not_focused(0)),
        )
        .with_step(
            TestStep::new("Typed characters should go to the pty and not warp input")
                .with_typed_characters(&["these are some typed characters"])
                .add_named_assertion("Input should be empty", input_is_empty(0))
                .add_named_assertion("Typed characters should go to pty", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _| {
                        async_assert_eq!(
                            view.model.lock().block_list().active_block().command_to_string(),
                            String::from("Enter some user input: this is the pasted textthese are some typed characters"),
                            "Typed characters should go to pty"
                        )
                    })
                })
        )
        .with_step(
            TestStep::new("Click on input to focus the input box")
                .with_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    input.read(app, |input, _| {
                        input.save_position_id()
                    })
                })
                .add_assertion(input_editor_is_focused(0)),
        )
        .with_step(
            TestStep::new("Pasted text should go in input since input is focused")
                .with_keystrokes(&[cmd_or_ctrl_shift("v")])
                .add_assertion(input_contains_string(0, "this is the pasted text".to_owned()))
        )
        .with_step(
            TestStep::new("Typed characters should go in input since input is focused")
                .with_typed_characters(&["these are some typed characters"])
                .add_assertion(input_contains_string(0, "this is the pasted textthese are some typed characters".to_owned())),
        )
        .with_step(
            TestStep::new("Focus the terminal view")
                .with_click_on_saved_position_fn(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _| {
                        view.content_element_position_id().to_owned()
                    })
                })
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let focused_view_id = app.focused_view_id(window_id).expect("Focused view should exist");
                    async_assert!(focused_view_id == terminal_view.id(), "Terminal should be focused")
                }),
        )
        .with_step(
            TestStep::new("Press enter to finish user input and allow bootstrapping to finish")
                .with_keystrokes(&["enter"]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Warp input should be focused and keep buffered text")
                .add_assertion(input_editor_is_focused(0))
                .add_assertion(input_contains_string(0, "this is the pasted textthese are some typed characters".to_owned()))
        )
}

pub fn test_bootstrap_with_no_script_execution_block() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Ensure there are no visible blocks")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(move |app, window_id| {
                    let views: Vec<ViewHandle<Workspace>> = app.views_of_type(window_id).unwrap();
                    let workspace = views.first().unwrap();
                    let terminal_view = workspace.read(app, |workspace, _| {
                        workspace
                            .get_pane_group_view_unchecked(0)
                            .read(app, |pane_group, ctx| {
                                pane_group
                                    .terminal_view_at_pane_index(0, ctx)
                                    .expect("View should be defined at pane 0")
                                    .clone()
                            })
                    });
                    terminal_view.read(app, |view, _| {
                        async_assert!(
                            view.selected_blocks_tail_index().is_none(),
                            "There should not be any blocks the user can select"
                        )
                    })
                }),
        )
}

pub fn test_instant_prompt_bootstrap() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Zsh)
        })
        .with_setup(|utils| {
            // If the instant prompt var is not set to off, hang forever.
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"if [[ "$POWERLEVEL9K_INSTANT_PROMPT" != "off" ]]; then read; fi"#,
                [ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
}

/// Ensure this issue doesn't happen again.
/// https://github.com/warpdotdev/Warp/issues/2636
/// Bootstrapping was failing when PROMPT_COMMAND was an array
pub fn test_bash_bootstraps_with_prompt_command_array() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on bash
            let (starter, version) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Bash)
                && version_compare::compare_to(version, "5.1", Cmp::Ge).unwrap_or(false)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
PROMPT_COMMAND=('printf "\033]0;TEST_TAB_TITLE\a"' 'echo hello')
"#,
                [ShellRcType::Bash],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0, /*tab_idx*/
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(tab_title_step(
            "Assert the user's tab title used",
            "TEST_TAB_TITLE".to_string(),
        ))
}

/// Ensures that the zsh bootstrap script's precmd/preexec hooks work correctly when the user
/// has `setopt nounset` (set -u) enabled. This option causes the shell to error when referencing
/// unset variables, so we need to use `${VAR:-}` syntax for external environment variables.
///
/// The test enables nounset after bootstrap completes, then runs commands to trigger the
/// precmd and preexec hooks which reference various environment variables.
pub fn test_zsh_bootstraps_with_nounset_option() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Zsh)
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Enable nounset after bootstrap, then run a command to trigger precmd/preexec hooks
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "setopt nounset".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        // Run another command to verify precmd/preexec work with nounset enabled
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo 'nounset test passed'".to_string(),
            ExpectedExitStatus::Success,
            "nounset test passed",
        ))
}

pub fn test_bash_bootstraps_with_prompt_command_array_that_sets_ps1() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on bash
            let (starter, version) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Bash)
                && version_compare::compare_to(version, "5.1", Cmp::Ge).unwrap_or(false)
        })
        .with_user_defaults(std::collections::HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
function custom_prompt() {
    PS1="darmok@tanagra$ "
}
PROMPT_COMMAND=('printf "\033]0;SHAKA WHEN THE WALLS FELL\a"' 'custom_prompt')
"#,
                [ShellRcType::Bash],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0, /*tab_idx*/
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(tab_title_step(
            "Assert the user's tab title used",
            "SHAKA WHEN THE WALLS FELL".to_string(),
        ))
        .with_step(
            new_step_with_default_assertions("Check PS1 value").add_assertion(
                move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let prompt = terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let block = model
                            .block_list()
                            .blocks()
                            .last()
                            .expect("After bootstrapping, we should have a block");
                        block.prompt_to_string()
                    });
                    async_assert_eq!(
                        prompt,
                        "darmok@tanagra$ ",
                        "prompt should be 'darmok@tanagra$ ' but got '{prompt}' instead"
                    )
                },
            ),
        )
}
