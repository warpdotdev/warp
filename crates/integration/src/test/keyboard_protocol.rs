use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    assert_long_running_block_executing_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warpui_core::event::{KeyEventDetails, KeyState};
use warpui_core::integration::TestStep;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::keyboard::KeyCode;
use warpui_core::{async_assert, Event};

use super::new_builder;
use crate::Builder;

/// Helper: creates a setup closure that writes a Python script asset to the test directory.
macro_rules! setup_python_script {
    ($filename:expr, $asset_path:expr) => {
        |utils| {
            let script_path = utils.test_dir().join($filename);
            let script_content = include_bytes!($asset_path);
            std::fs::write(&script_path, script_content).expect("Failed to write test script");
        }
    };
}

/// Helper: creates a step that waits for "Protocol enabled" to appear in terminal output.
fn wait_for_protocol_enabled() -> TestStep {
    TestStep::new("Wait for protocol to be enabled")
        .add_assertion(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                let model = view.model.lock();
                let output = model.block_list().active_block().output_to_string();
                async_assert!(
                    output.contains("Protocol enabled"),
                    "Protocol should be enabled, but output was: {output}"
                )
            })
        })
        .set_timeout(Duration::from_secs(5))
}

/// Helper: creates an assertion closure that checks the terminal output contains `expected`.
fn assert_output_contains(
    expected: &'static str,
    description: &'static str,
) -> impl FnMut(&mut warpui_core::App, warpui_core::WindowId) -> warpui_core::integration::AssertionOutcome
{
    move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, _ctx| {
            let model = view.model.lock();
            let output = model.block_list().active_block().output_to_string();
            async_assert!(
                output.contains(expected),
                "{description}, but output was: {output}"
            )
        })
    }
}

/// Test that without keyboard protocol enabled, Shift+Enter sends \n
pub fn test_keyboard_protocol_disabled_shift_enter() -> Builder {
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys.py",
            "../../assets/read_keys.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys.py")
                .with_typed_characters(&["python3 ~/read_keys.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(
            TestStep::new("Wait for script to be ready")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model.block_list().active_block().output_to_string();

                        // Wait until the "Ready" message appears
                        async_assert!(
                            output.contains("Ready"),
                            "Script should be ready, but output was: {}",
                            output
                        )
                    })
                })
                .set_timeout(Duration::from_secs(5)),
        )
        .with_step(
            TestStep::new("Send Shift+Enter")
                .with_keystrokes(&["shift-enter"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model.block_list().active_block().output_to_string();

                        // Should see "Received byte: 0x0a" in the output (newline)
                        async_assert!(
                            output.contains("0x0a"),
                            "Expected Shift+Enter to send 0x0a (\\n), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            TestStep::new("Send plain Enter")
                .with_keystrokes(&["enter"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model.block_list().active_block().output_to_string();

                        // Should see "Received byte: 0x0d" in the output (carriage return)
                        async_assert!(
                            output.contains("0x0d"),
                            "Expected plain Enter to send 0x0d (\\r), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}

/// Test that when keyboard protocol is enabled, Shift+Enter sends CSI u sequence
pub fn test_keyboard_protocol_enabled_shift_enter() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_with_protocol.py",
            "../../assets/read_keys_with_protocol.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_with_protocol.py")
                .with_typed_characters(&["python3 ~/read_keys_with_protocol.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_long_running_block_executing_for_single_terminal_in_tab(true, 0)),
        )
        .with_step(wait_for_protocol_enabled())
        .with_step(
            TestStep::new("Send Shift+Enter")
                .with_keystrokes(&["shift-enter"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model
                            .block_list()
                            .active_block()
                            .output_to_string();

                        // Should see CSI sequence like '\x1b[13;2u'
                        // which is ESC [ 13 ; 2 u (Enter=13, Shift=2)
                        async_assert!(
                            output.contains("13;2u"),
                            "Expected Shift+Enter to send CSI u sequence (ESC [ 13 ; 2 u), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            TestStep::new("Send plain Enter")
                .with_keystrokes(&["enter"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model
                            .block_list()
                            .active_block()
                            .output_to_string();

                        // Plain Enter with no modifiers sends CSI u sequence.
                        // Per Kitty protocol, this can be either:
                        // - ESC [ 13 u (modifier omitted when no modifiers)
                        // - ESC [ 13 ; 1 u (explicit modifier=1)
                        // Check for the "Complete sequence" repr output to avoid
                        // matching the earlier Shift+Enter "13;2u" substring.
                        async_assert!(
                            output.contains("'\\x1b[13u'") || output.contains("'\\x1b[13;1u'"),
                            "Expected plain Enter to send CSI u sequence (ESC [ 13 u or ESC [ 13 ; 1 u), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit")
                .with_keystrokes(&["ctrl-c"]),
        )
}

/// Test that shifted printable keys encode the unshifted keycode in CSI-u.
pub fn test_keyboard_protocol_enabled_shifted_symbol_uses_unshifted_keycode() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_with_protocol.py",
            "../../assets/read_keys_with_protocol.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_with_protocol.py")
                .with_typed_characters(&["python3 ~/read_keys_with_protocol.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(wait_for_protocol_enabled())
        // Shift+@ → 50;2u (base key '2'=50 with Shift modifier=2)
        .with_step(
            TestStep::new("Send Shift+@ with key_without_modifiers='2'")
                .with_event(Event::KeyDown {
                    keystroke: Keystroke::parse("shift-@").unwrap(),
                    chars: "@".to_string(),
                    details: KeyEventDetails {
                        key_without_modifiers: Some("2".to_string()),
                        ..Default::default()
                    },
                    is_composing: false,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "50;2u",
                    "Expected Shift+@ to encode as 50;2u (base key '2'=50)",
                )),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}

/// Test alternate keys (flag 4) and associated text (flag 16) encoding.
/// With flags 29 (1+4+8+16), shift+A should produce CSI 97:65;2;65u
/// (base=97 'a', alternate=65 'A', shift modifier=2, text=65 'A').
pub fn test_keyboard_protocol_alternate_keys_and_text() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_alternate_text.py",
            "../../assets/read_keys_alternate_text.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_alternate_text.py")
                .with_typed_characters(&["python3 ~/read_keys_alternate_text.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(wait_for_protocol_enabled())
        .with_step(
            // Shift+A should produce: CSI 97:65;2;65u
            // - 97 = base key 'a' (unshifted)
            // - :65 = alternate key 'A' (shifted) from flag 4
            // - ;2 = shift modifier
            // - ;65 = associated text 'A' from flag 16
            TestStep::new("Send Shift+A")
                .with_keystrokes(&["shift-A"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model.block_list().active_block().output_to_string();

                        async_assert!(
                            output.contains("97:65;2;65u"),
                            "Expected Shift+A to produce CSI 97:65;2;65u (alternate + text), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            // Plain 'a' should produce: CSI 97;1;97u
            // - 97 = key 'a'
            // - ;1 = no modifiers (must be present because text field follows)
            // - ;97 = associated text 'a' from flag 16
            // No alternate key because shift is not held.
            TestStep::new("Send plain 'a'")
                .with_keystrokes(&["a"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model.block_list().active_block().output_to_string();

                        async_assert!(
                            output.contains("97;1;97u"),
                            "Expected plain 'a' to produce CSI 97;1;97u (with associated text), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit")
                .with_keystrokes(&["ctrl-c"]),
        )
}

/// Test kitty apply-mode semantics and query responses through terminal integration.
pub fn test_keyboard_protocol_query_and_apply_modes() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "query_keyboard_modes.py",
            "../../assets/query_keyboard_modes.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute query_keyboard_modes.py")
                .with_typed_characters(&["python3 ~/query_keyboard_modes.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(
            TestStep::new("Verify query responses")
                .set_timeout(Duration::from_secs(15))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let output = model.block_list().active_block().output_to_string();

                        // After set=1 → query should give ?1u
                        // After union 8 (onto 1) → query should give ?9u
                        // After diff 1 (from 9) → query should give ?8u
                        async_assert!(
                            output.contains("query_1=b'\\x1b[?1u'")
                                && output.contains("query_2=b'\\x1b[?9u'")
                                && output.contains("query_3=b'\\x1b[?8u'"),
                            "Expected query/apply responses (query_1=?1u, query_2=?9u, query_3=?8u), but output was: {}",
                            output
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}

/// Test flag 8 (report all keys as escape codes): printable chars become CSI u,
/// cursor keys remain legacy, and Ctrl+key combos include modifier.
pub fn test_keyboard_protocol_report_all_keys_printable_and_cursor() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_report_all.py",
            "../../assets/read_keys_report_all.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_report_all.py")
                .with_typed_characters(&["python3 ~/read_keys_report_all.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(wait_for_protocol_enabled())
        // Plain 'a' → ESC[97u
        .with_step(
            TestStep::new("Send plain 'a'")
                .with_keystrokes(&["a"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "97u",
                    "Expected plain 'a' to send CSI u with key code 97",
                )),
        )
        // Plain '1' → ESC[49u
        .with_step(
            TestStep::new("Send plain '1'")
                .with_keystrokes(&["1"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "49u",
                    "Expected plain '1' to send CSI u with key code 49",
                )),
        )
        // Up arrow → legacy ESC[A (cursor keys are not encoded via CSI u)
        .with_step(
            TestStep::new("Send Up arrow")
                .with_keystrokes(&["up"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "Legacy arrow",
                    "Expected Up arrow to use legacy encoding (ESC[A), not CSI u",
                )),
        )
        // Ctrl+a → ESC[97;5u
        .with_step(
            TestStep::new("Send Ctrl+a")
                .with_keystrokes(&["ctrl-a"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "97;5u",
                    "Expected Ctrl+a to send CSI u with key code 97 and Ctrl modifier (97;5u)",
                )),
        )
        // 'é' (U+00E9 = 233) → CSI 233u. Tests that multi-byte UTF-8 characters
        // are correctly handled via `key.chars().count() == 1`.
        .with_step(
            TestStep::new("Send 'é' (U+00E9)")
                .with_event(Event::KeyDown {
                    keystroke: Keystroke {
                        key: "é".to_string(),
                        ctrl: false,
                        alt: false,
                        shift: false,
                        cmd: false,
                        meta: false,
                    },
                    chars: "é".to_string(),
                    details: KeyEventDetails::default(),
                    is_composing: false,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "233u",
                    "Expected 'é' (U+00E9) to encode as CSI 233u",
                )),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}

/// Test flag 2 (report event types): with flags 1+2+8=11, press is the default event
/// type and is omitted per the Kitty spec. Pressing 'a' produces ESC[97u (same as
/// without flag 2). Event types only differ for repeat/release events.
pub fn test_keyboard_protocol_event_types() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_event_types.py",
            "../../assets/read_keys_event_types.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_event_types.py")
                .with_typed_characters(&["python3 ~/read_keys_event_types.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(wait_for_protocol_enabled())
        // With flags 11 (1+2+8), pressing 'a' produces ESC[97u.
        // Press is the default event type and is omitted.
        .with_step(
            TestStep::new("Send 'a' and verify event type encoding")
                .with_keystrokes(&["a"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "97u",
                    "Expected 'a' to be encoded as CSI 97u (press event type is default, omitted)",
                )),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}

/// Test standalone modifier key reporting with flags 1+2+8=11.
/// Sends ModifierKeyChanged events for ShiftLeft press/release and verifies
/// the CSI u encoding includes the correct key code and event type.
pub fn test_keyboard_protocol_modifier_key_reporting() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_event_types.py",
            "../../assets/read_keys_event_types.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_event_types.py")
                .with_typed_characters(&["python3 ~/read_keys_event_types.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(wait_for_protocol_enabled())
        // ShiftLeft press → ESC[57441;2:1u (key code 57441, modifiers=2 with self-bit, event_type=1 press)
        .with_step(
            TestStep::new("Send ShiftLeft press")
                .with_event(Event::ModifierKeyChanged {
                    key_code: KeyCode::ShiftLeft,
                    state: KeyState::Pressed,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "57441;2:1u",
                    "Expected ShiftLeft press to produce CSI u with key code 57441, modifiers=2 (self-bit), event type :1",
                )),
        )
        // ShiftLeft release → ESC[57441;2:3u (modifiers=2 with self-bit, event_type=3 release)
        .with_step(
            TestStep::new("Send ShiftLeft release")
                .with_event(Event::ModifierKeyChanged {
                    key_code: KeyCode::ShiftLeft,
                    state: KeyState::Released,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "57441;2:3u",
                    "Expected ShiftLeft release to produce CSI u with modifiers=2 (self-bit) and event type :3",
                )),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}

/// Test that all modifier keys include the correct self-bit in their CSI u encoding.
/// Per the Kitty spec, pressing a modifier key alone must set its own modifier bit:
/// - ShiftLeft (57441): modifiers = 1 + shift(1) = 2
/// - ControlLeft (57442): modifiers = 1 + ctrl(4) = 5
/// - AltLeft (57443): modifiers = 1 + alt(2) = 3
///
/// Uses flags 1+2+8=11 to enable event type reporting.
pub fn test_keyboard_protocol_modifier_self_bit() -> Builder {
    FeatureFlag::KittyKeyboardProtocol.set_enabled(true);
    new_builder()
        .with_setup(setup_python_script!(
            "read_keys_event_types.py",
            "../../assets/read_keys_event_types.py"
        ))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute read_keys_event_types.py")
                .with_typed_characters(&["python3 ~/read_keys_event_types.py"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(wait_for_protocol_enabled())
        // ControlLeft press → ESC[57442;5:1u (modifiers = 1 + ctrl(4) = 5)
        .with_step(
            TestStep::new("Send ControlLeft press")
                .with_event(Event::ModifierKeyChanged {
                    key_code: KeyCode::ControlLeft,
                    state: KeyState::Pressed,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "57442;5:1u",
                    "Expected ControlLeft press to produce CSI u with modifiers=5 (self-bit ctrl=4)",
                )),
        )
        // ControlLeft release → ESC[57442;5:3u
        .with_step(
            TestStep::new("Send ControlLeft release")
                .with_event(Event::ModifierKeyChanged {
                    key_code: KeyCode::ControlLeft,
                    state: KeyState::Released,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "57442;5:3u",
                    "Expected ControlLeft release to produce CSI u with modifiers=5 and event type :3",
                )),
        )
        // AltLeft press → ESC[57443;3:1u (modifiers = 1 + alt(2) = 3)
        .with_step(
            TestStep::new("Send AltLeft press")
                .with_event(Event::ModifierKeyChanged {
                    key_code: KeyCode::AltLeft,
                    state: KeyState::Pressed,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "57443;3:1u",
                    "Expected AltLeft press to produce CSI u with modifiers=3 (self-bit alt=2)",
                )),
        )
        // AltLeft release → ESC[57443;3:3u
        .with_step(
            TestStep::new("Send AltLeft release")
                .with_event(Event::ModifierKeyChanged {
                    key_code: KeyCode::AltLeft,
                    state: KeyState::Released,
                })
                .set_timeout(Duration::from_secs(5))
                .add_assertion(assert_output_contains(
                    "57443;3:3u",
                    "Expected AltLeft release to produce CSI u with modifiers=3 and event type :3",
                )),
        )
        .with_step(
            new_step_with_default_assertions("Send Ctrl+C to exit").with_keystrokes(&["ctrl-c"]),
        )
}
