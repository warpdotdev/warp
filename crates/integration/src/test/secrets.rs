use warp::integration_testing::clipboard::assert_clipboard_contains_string;
use warp::integration_testing::secret_redaction::{
    assert_secret_tooltip_open, assert_secrets_redacted_for_ai,
};
use warp::integration_testing::settings::toggle_setting;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    assert_selected_block_index_is_last_renderable, execute_command_for_single_terminal_in_tab,
    initialize_secret_regexes, open_context_menu_for_selected_block, run_alt_grid_program,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view;
use warp::settings_view::{PrivacyPageAction, SettingsAction};
use warp::terminal::model::index::Point;
use warp::terminal::model::terminal_model::WithinModel;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;

use super::{new_builder, Builder};
use crate::util::skip_if_powershell_core_2303;

pub fn test_secret_is_obfuscated_on_copy() -> Builder {
    let phone_number = "123-456-7890";
    let phone_number_obfuscated = "************";
    new_builder()
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo {phone_number}"),
            ExpectedExitStatus::Success,
            phone_number,
        ))
        .with_step(
            new_step_with_default_assertions("Select block")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_named_assertion(
                    "ensure block is selected",
                    assert_selected_block_index_is_last_renderable(),
                ),
        )
        .with_steps(open_context_menu_for_selected_block())
        .with_step(
            new_step_with_default_assertions("Arrow down and select copy Command")
                .with_keystrokes(&["down", "down", "enter"])
                .add_assertion(assert_clipboard_contains_string(format!(
                    "echo {phone_number_obfuscated}"
                ))),
        )
}

pub fn test_secret_tooltip_shows_on_click() -> Builder {
    let phone_number = "123-456-7890";
    new_builder()
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo {phone_number}"),
            ExpectedExitStatus::Success,
            phone_number,
        ))
        .with_step(
            // Note: ideally, we shouldn't hardcode a secret handle ID here but we're doing this
            // for now. This is affected by the addition/removal of new `GridType`s!
            new_step_with_default_assertions("Click on secret to show tooltip")
                .with_click_on_saved_position("terminal_view:first_cell_in_secret_1")
                .add_assertion(assert_secret_tooltip_open(true)),
        )
}

pub fn test_secret_tooltip_respects_safe_mode_setting() -> Builder {
    let phone_number = "123-456-7890";
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        ))) // Safe mode is now enabled.
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo {phone_number}"),
            ExpectedExitStatus::Success,
            phone_number,
        ))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        ))) // Safe mode is now disabled.
        .with_step(
            // Note: ideally, we shouldn't hardcode a secret handle ID here but we're doing this
            // for now. This is affected by the addition/removal of new `GridType`s!
            new_step_with_default_assertions("Click on secret to show tooltip")
                .with_click_on_saved_position("terminal_view:first_cell_in_secret_1")
                .add_assertion(assert_secret_tooltip_open(false)),
        )
}

pub fn test_copy_secret_respects_safe_mode_setting() -> Builder {
    let phone_number = "123-456-7890";
    new_builder()
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo {phone_number}"),
            ExpectedExitStatus::Success,
            phone_number,
        ))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(
            new_step_with_default_assertions("Select block")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_named_assertion(
                    "ensure block is selected",
                    assert_selected_block_index_is_last_renderable(),
                ),
        )
        .with_steps(open_context_menu_for_selected_block())
        .with_step(
            new_step_with_default_assertions("Arrow down and select copy Command")
                .with_keystrokes(&["down", "down", "enter"])
                .add_assertion(assert_clipboard_contains_string(format!(
                    "echo {phone_number}"
                ))),
        )
}

pub fn test_alt_screen_secret_detection() -> Builder {
    let phone_number = "123-456-7890";
    let exit_step = TestStep::new("Exit vim")
        .with_keystrokes(&["escape"])
        .with_typed_characters(&[":q!"])
        .with_keystrokes(&["enter"]);

    new_builder()
        // TODO(CORE-2732): Flakey on Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        .with_steps(run_alt_grid_program(
            "vim",
            0,
            0,
            exit_step,
            vec![
                TestStep::new("Type in secret").with_typed_characters(&["i", phone_number]),
                TestStep::new("Check that secret exists").add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view(app, window_id);

                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let secret =
                            model.secret_at_point(&WithinModel::AltScreen(Point::new(0, 0)));
                        async_assert!(secret.is_some(), "Secret exists")
                    })
                }),
                TestStep::new("Check that secret is obfuscated").add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view(app, window_id);

                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let secret =
                            model.secret_at_point(&WithinModel::AltScreen(Point::new(0, 0)));
                        let secret = secret
                            .expect("Secret existence verified by previous step")
                            .1;
                        async_assert!(secret.is_obfuscated(), "Secret is obfuscated")
                    })
                }),
            ],
        ))
}

pub fn test_secret_case_sensitivity() -> Builder {
    // Test the secret redaction respects case by default
    new_builder()
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        // AWS Access ID pattern is case-sensitive by default
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo 'AKIAABC123456789DEFG akiaabc123456789defg'".to_string(),
            ExpectedExitStatus::Success,
            "AKIAABC123456789DEFG akiaabc123456789defg",
        ))
        .with_step(
            new_step_with_default_assertions("Select block")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_named_assertion(
                    "ensure block is selected",
                    assert_selected_block_index_is_last_renderable(),
                ),
        )
        .with_steps(open_context_menu_for_selected_block())
        .with_step(
            new_step_with_default_assertions("Arrow down and select copy Command")
                .with_keystrokes(&["down", "down", "enter"])
                // Only the uppercase ID should be redacted since pattern requires uppercase
                .add_assertion(assert_clipboard_contains_string(
                    "echo '******************** akiaabc123456789defg'".to_string(),
                )),
        )
}

pub fn test_secrets_are_always_redacted_in_ai_inputs() -> Builder {
    let phone_number = "123-456-7890";
    let secret_api_key = "sk-1234567890abcdef";
    let expected_redacted_phone = "************";
    let expected_redacted_api_key = "******************";
    let test_command = "echo 'Phone: 123-456-7890 API: sk-1234567890abcdef'.";
    let test_output = "Phone: 123-456-7890 API: sk-1234567890abcdef.";

    new_builder()
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Test case 1: Strikethrough mode - secrets should be redacted from AI inputs
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        ))) // Enable safe mode (strikethrough by default since hide_secrets_in_block_list defaults to false)
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            test_command.to_string(),
            ExpectedExitStatus::Success,
            test_output,
        ))
        .with_step(
            new_step_with_default_assertions("Test strikethrough mode redaction").add_assertion(
                assert_secrets_redacted_for_ai(
                    test_output.to_string(),
                    expected_redacted_phone.to_string(),
                    expected_redacted_api_key.to_string(),
                    phone_number.to_string(),
                    secret_api_key.to_string(),
                ),
            ),
        )
        // Test case 2: Full obfuscation mode - secrets should also be redacted
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        ))) // Enable full hiding (Yes mode)
        .with_step(
            new_step_with_default_assertions("Test full obfuscation mode redaction").add_assertion(
                assert_secrets_redacted_for_ai(
                    test_output.to_string(),
                    expected_redacted_phone.to_string(),
                    expected_redacted_api_key.to_string(),
                    phone_number.to_string(),
                    secret_api_key.to_string(),
                ),
            ),
        )
}
