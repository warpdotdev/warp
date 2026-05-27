//! Integration tests for the Rich Input Ctrl+Enter submit toggle (issue #11588).
//!
//! These tests drive the full event-dispatch path — keystrokes enter through the
//! editor's keymap binding layer — so they complement the unit tests in
//! `app/src/terminal/input_tests.rs` that call `input_ctrl_enter` directly.
//!
//! ## What we test
//!
//! | setting | key        | expected outcome              |
//! |---------|------------|-------------------------------|
//! | false   | Enter      | buffer cleared (submit fired)  |
//! | false   | Ctrl+Enter | buffer unchanged (pass-through)|
//! | true    | Enter      | buffer contains newline        |
//! | true    | Ctrl+Enter | buffer cleared (submit fired)  |
//!
//! "Buffer cleared" is the observable proxy for "submit fired", because
//! `TerminalView::submit_cli_agent_rich_input` calls
//! `input.clear_buffer_and_reset_undo_stack` immediately after emitting the
//! event (see `use_agent_footer/mod.rs:688`).

use std::collections::HashMap;

use settings::Setting as _;
use warp::{
    features::FeatureFlag,
    integration_testing::{
        input::{
            open_cli_agent_rich_input, rich_input_buffer_contains_newline,
            rich_input_buffer_does_not_contain_newline, rich_input_buffer_is_not_empty,
            rich_input_buffer_text_is_empty,
        },
        step::new_step_with_default_assertions,
        terminal::wait_until_bootstrapped_single_pane_for_tab,
    },
    settings::SubmitRichInputOnCtrlEnter,
};

use super::new_builder;
use crate::Builder;

// ---------------------------------------------------------------------------
// Setting = false (default): Enter submits, buffer is cleared
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = false`, pressing Enter inside the Rich Input
/// fires a submit and the buffer is cleared.
pub fn test_rich_input_enter_submits_when_ctrl_enter_setting_is_false() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_cli_agent_rich_input(0))
        .with_step(
            new_step_with_default_assertions(
                "Type 'hello', press Enter — buffer should be empty (submit fired)",
            )
            .with_typed_characters(&["hello"])
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_text_is_empty(0)),
        )
}

// ---------------------------------------------------------------------------
// Setting = false (default): Ctrl+Enter passes through, buffer unchanged
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = false`, pressing Ctrl+Enter inside the Rich
/// Input must NOT submit — the buffer text is preserved.
pub fn test_rich_input_ctrl_enter_passthrough_when_ctrl_enter_setting_is_false() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_cli_agent_rich_input(0))
        .with_step(
            new_step_with_default_assertions(
                "Type 'hello', press Ctrl+Enter — buffer should still contain text",
            )
            .with_typed_characters(&["hello"])
            .with_keystrokes(&["ctrl-enter"])
            .add_assertion(rich_input_buffer_is_not_empty(0)),
        )
}

// ---------------------------------------------------------------------------
// Setting = true: Enter inserts newline, buffer is NOT empty / contains '\n'
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = true`, pressing Enter inside the Rich Input
/// must insert a newline and NOT submit.
pub fn test_rich_input_enter_inserts_newline_when_ctrl_enter_setting_is_true() -> Builder {
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
                "Type 'line1', press Enter — buffer should contain newline (no submit)",
            )
            .with_typed_characters(&["line1"])
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_contains_newline(0)),
        )
}

// ---------------------------------------------------------------------------
// Setting = true: Ctrl+Enter submits, buffer is cleared; no stray newline
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Setting = true: Ctrl+Enter with text selected submits full buffer (PR #11723)
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = true`, when the user has text selected inside
/// the Rich Input and presses Ctrl+Enter, the buffer must be cleared (i.e.
/// the submit fired with the full text — the selection must not be dropped).
///
/// This is the end-to-end regression test for the silent-data-loss bug caught
/// in PR #11723 (issue #11588): under the broken implementation the editor's
/// `InsertNewLineIfMultiLine` action replaced the selection with `\n`, losing
/// the selected text before the backspace-then-submit path ran.
///
/// "Buffer cleared" is the observable proxy for "submit fired with full text"
/// because `TerminalView::submit_cli_agent_rich_input` calls
/// `input.clear_buffer_and_reset_undo_stack` immediately after emitting the
/// event.
pub fn test_rich_input_ctrl_enter_preserves_selection() -> Builder {
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
                "Type 'hello world', select all, press Ctrl+Enter — buffer must be cleared",
            )
            // Type the full message.
            .with_typed_characters(&["hello world"])
            // Select all with Ctrl+A so the entire text is selected.
            .with_keystrokes(&["ctrl-a"])
            // Press Ctrl+Enter: with the fix, this submits the full selected
            // text and the buffer is cleared.  Without the fix, the selected
            // text is replaced by '\n' first, resulting in a non-empty buffer
            // after the backspace.
            .with_keystrokes(&["ctrl-enter"])
            .add_assertion(rich_input_buffer_text_is_empty(0)),
        )
}

/// With `submit_on_ctrl_enter = true`, typing across two lines and pressing
/// Ctrl+Enter must submit (buffer cleared) with no stray leading/trailing
/// newline in the submitted payload.
///
/// This is the end-to-end regression test for the editor-inserts-newline-then-
/// backspace assumption described in issue #11588: if `backspace()` is not
/// called (or called incorrectly), the submitted text contains a trailing `\n`
/// and the buffer is not empty.
pub fn test_rich_input_ctrl_enter_submits_when_ctrl_enter_setting_is_true() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_cli_agent_rich_input(0))
        // Enter a two-line prompt: "line1" + newline (Enter) + "line2".
        .with_step(
            new_step_with_default_assertions("Type 'line1', press Enter to insert newline")
                .with_typed_characters(&["line1"])
                .with_keystrokes(&["enter"])
                .add_assertion(rich_input_buffer_contains_newline(0)),
        )
        .with_step(
            new_step_with_default_assertions(
                "Type 'line2', press Ctrl+Enter — buffer cleared (submit fired, no stray newline)",
            )
            .with_typed_characters(&["line2"])
            .with_keystrokes(&["ctrl-enter"])
            // Submit fires → clear_buffer_and_reset_undo_stack → buffer is empty.
            .add_assertion(rich_input_buffer_text_is_empty(0)),
        )
}

// ---------------------------------------------------------------------------
// Setting = true: Enter while slash-commands menu is open accepts the menu
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = true`, typing `/` opens the slash-commands
/// menu and pressing Enter must route to the menu-acceptance branch rather
/// than inserting a newline.
///
/// Observable proxy: the buffer must NOT contain a `\n` character after Enter.
///
/// This is the end-to-end regression test for the second bug in issue #11588:
/// before the fix, `update_cli_agent_enter_settings` set
/// `enter = InsertNewLineIfMultiLine` which caused the editor to call
/// `newline_internal` directly, bypassing `input_enter` and all its
/// menu-acceptance branches.
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
            // Typing '/' opens the slash-commands menu.
            .with_typed_characters(&["/"])
            // Enter must route to the menu-acceptance branch, not newline insertion.
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_does_not_contain_newline(0)),
        )
}
