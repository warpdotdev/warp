//! Integration tests for the Rich Input Ctrl+Enter submit toggle (issue #11588).
//!
//! These tests drive the full keystroke-dispatch path and complement the unit
//! tests in `app/src/terminal/input_tests.rs`.

use std::collections::HashMap;

use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::input::{
    open_cli_agent_rich_input, rich_input_buffer_contains_newline,
    rich_input_buffer_does_not_contain_newline, rich_input_buffer_text_is_empty,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::settings::SubmitRichInputOnCtrlEnter;

use super::new_builder;
use crate::Builder;

// ---------------------------------------------------------------------------
// Setting = true: end-to-end wiring guard
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = true`, Enter inserts a newline and Ctrl+Enter
/// submits (buffer cleared).  This is the full-stack wiring guard for the
/// toggle: it proves that the setting actually propagates to editor behaviour
/// (issue #11588).
pub fn test_rich_input_toggle_on_enter_inserts_newline_and_ctrl_enter_submits() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_cli_agent_rich_input(0))
        // Enter inserts a newline (not a submit).
        .with_step(
            new_step_with_default_assertions(
                "Type 'line1', press Enter — buffer should contain newline (no submit)",
            )
            .with_typed_characters(&["line1"])
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_contains_newline(0)),
        )
        // Ctrl+Enter submits: buffer is cleared.
        .with_step(
            new_step_with_default_assertions(
                "Type 'line2', press Ctrl+Enter — buffer cleared (submit fired)",
            )
            .with_typed_characters(&["line2"])
            .with_keystrokes(&["ctrl-enter"])
            .add_assertion(rich_input_buffer_text_is_empty(0)),
        )
}

// ---------------------------------------------------------------------------
// Setting = true: Enter while slash-commands menu is open accepts the menu
// ---------------------------------------------------------------------------

/// Regression (#11588): with toggle ON, typing `/` opens the slash-commands
/// menu and pressing Enter must route to menu acceptance, not newline insertion.
pub fn test_rich_input_enter_accepts_menu_item_when_toggle_is_true() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_cli_agent_rich_input(0))
        .with_step(
            new_step_with_default_assertions(
                "Type '/', press Enter — buffer must NOT contain a newline (menu branch taken)",
            )
            .with_typed_characters(&["/"])
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_does_not_contain_newline(0)),
        )
}
