use warp::cmd_or_ctrl_shift;
use warp::integration_testing::terminal::util::current_shell_starter_and_version;
use warp::integration_testing::terminal::{
    assert_context_menu_is_open, initialize_secret_regexes,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warp::settings_view::{PrivacyPageAction, SettingsAction};
use warp::terminal::model::index::Point;
use warp::terminal::model::terminal_model::{BlockIndex, WithinBlock, WithinModel};
use warp::terminal::shell::ShellType;
use warp::terminal::GridType;
use warpui_core::{async_assert, async_assert_eq};

use super::new_builder;
use crate::test::integration_testing::block_filtering::{
    open_block_filter_editor, open_block_filter_editor_for_long_running_command,
    open_block_filter_editor_via_keybinding,
    open_block_filter_editor_via_keybinding_long_running_command, LongRunningCommandTestCase,
    SecretTestCase, SimpleTestCase,
};
use crate::test::integration_testing::secret_redaction::assert_secret_tooltip_open;
use crate::test::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, hover_over_block_zero,
};
use crate::test::{new_step_with_default_assertions, toggle_setting, TestStep};
use crate::Builder;

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_keybinding() -> Builder {
    new_builder()
        .set_should_run_test(|| cfg!(target_os = "macos"))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(SimpleTestCase::execute_command())
        .with_step(open_block_filter_editor_via_keybinding())
        .with_step(SimpleTestCase::perform_filter_query())
}

pub fn test_block_filtering_keybinding_with_long_running_command() -> Builder {
    new_builder()
        .set_should_run_test(|| cfg!(target_os = "macos"))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_steps(LongRunningCommandTestCase::enter_input_into_cat())
        .with_step(open_block_filter_editor_via_keybinding_long_running_command())
        .with_step(LongRunningCommandTestCase::perform_filter_query())
        .with_step(LongRunningCommandTestCase::exit_long_running_command())
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_toolbelt_icon() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(SimpleTestCase::execute_command())
        .with_step(hover_over_block_zero())
        .with_step(
            new_step_with_default_assertions("Open block filter editor via toolbelt icon")
                .with_click_on_saved_position("filter_button_for_block_0")
                .add_named_assertion("Assert that block filter is open", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        async_assert_eq!(
                            view.active_filter_editor_block_index(),
                            Some(BlockIndex::zero())
                        )
                    })
                }),
        )
        .with_step(SimpleTestCase::perform_filter_query())
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_context_menu() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(SimpleTestCase::execute_command())
        .with_step(hover_over_block_zero())
        .with_step(
            new_step_with_default_assertions("Click on context menu button")
                .with_click_on_saved_position("context_menu_button_0")
                .add_assertion(assert_context_menu_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Select context menu action")
                .with_click_on_saved_position("Toggle block filter")
                .add_named_assertion("Assert that block filter is open", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        async_assert_eq!(
                            view.active_filter_editor_block_index(),
                            Some(BlockIndex::zero())
                        )
                    })
                }),
        )
        .with_step(SimpleTestCase::perform_filter_query())
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_toggle_filter() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(SimpleTestCase::execute_command())
        .with_step(open_block_filter_editor())
        .with_step(SimpleTestCase::perform_filter_query())
        .with_step(hover_over_block_zero())
        .with_step(
            new_step_with_default_assertions("Click on context menu button")
                .with_click_on_saved_position("context_menu_button_0")
                .add_assertion(assert_context_menu_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Toggle block filter off")
                .with_click_on_saved_position("Toggle block filter")
                .add_named_assertion(
                    "Assert that the block filter editor is not open",
                    |app, window_id| {
                        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                        terminal_view.read(app, |view, _ctx| {
                            async_assert!(view.active_filter_editor_block_index().is_none())
                        })
                    },
                )
                .add_named_assertion(
                    "Assert that all lines are present after toggling off filter",
                    |app, window_id| {
                        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                        terminal_view.read(app, |view, _ctx| {
                            let model = view.model.lock();
                            let displayed_output_rows = model
                                .block_list()
                                .last_non_hidden_block()
                                .expect("No last non-hidden block found.")
                                .displayed_output_rows();
                            async_assert!(displayed_output_rows.is_none())
                        })
                    },
                ),
        )
        .with_step(hover_over_block_zero())
        .with_step(
            new_step_with_default_assertions("Click on context menu button")
                .with_click_on_saved_position("context_menu_button_0")
                .add_assertion(assert_context_menu_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Toggle block filter on")
                .with_click_on_saved_position("Toggle block filter")
                .add_named_assertion(
                    "Assert that block filter is applied",
                    SimpleTestCase::assert_filter_is_applied(),
                )
                .add_named_assertion("Assert that block filter is open", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        async_assert_eq!(
                            view.active_filter_editor_block_index(),
                            Some(BlockIndex::zero())
                        )
                    })
                }),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_toggle_filter_while_find_active() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(SimpleTestCase::execute_command())
        .with_step(
            new_step_with_default_assertions("Open find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .with_typed_characters(&["line"])
                .add_named_assertion("Assert that there 6 find matches", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let num_matches = terminal_view.read(app, |view, ctx| {
                        let find_model = view.find_model().as_ref(ctx);
                        find_model.visible_block_list_match_count()
                    });
                    async_assert_eq!(
                        num_matches,
                        6,
                        "Expected six matches but got {:?}",
                        num_matches
                    )
                }),
        )
        .with_step(open_block_filter_editor())
        .with_step(SimpleTestCase::perform_filter_query().add_named_assertion(
            "Assert that we have 4 find matches after filtering",
            |app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                let num_matches = terminal_view.read(app, |view, ctx| {
                    let find_model = view.find_model().as_ref(ctx);
                    find_model.visible_block_list_match_count()
                });
                async_assert_eq!(
                    num_matches,
                    4,
                    "Expected four matches but got {:?}",
                    num_matches
                )
            },
        ))
        .with_step(
            new_step_with_default_assertions("Open find bar and clear find query")
                .with_keystrokes(&[cmd_or_ctrl_shift("a"), "backspace".to_string()])
                .add_named_assertion(
                    "Assert that there 6 find matches again",
                    |app, window_id| {
                        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                        let num_matches = terminal_view.read(app, |view, ctx| {
                            let find_model = view.find_model().as_ref(ctx);
                            find_model.visible_block_list_match_count()
                        });
                        async_assert_eq!(
                            num_matches,
                            6,
                            "Expected six matches but got {:?}",
                            num_matches
                        )
                    },
                ),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_filter_then_find() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(SimpleTestCase::execute_command())
        .with_step(open_block_filter_editor())
        .with_step(SimpleTestCase::perform_filter_query())
        .with_step(
            new_step_with_default_assertions("Open find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .with_typed_characters(&["line"])
                .add_named_assertion("Assert that there 4 find matches", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let num_matches = terminal_view.read(app, |view, ctx| {
                        let find_model = view.find_model().as_ref(ctx);
                        find_model.visible_block_list_match_count()
                    });
                    async_assert_eq!(
                        num_matches,
                        4,
                        "Expected four matches but got {:?}",
                        num_matches
                    )
                }),
        )
}

pub fn test_block_filtering_with_secrets() -> Builder {
    new_builder()
        // TODO(REV-569): Fish flaking on linux
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            !matches!(
                starter.shell_type(),
                ShellType::Fish | ShellType::PowerShell
            )
        })
        .with_step(initialize_secret_regexes())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleSafeMode,
        )))
        .with_step(toggle_setting(SettingsAction::PrivacyPageToggle(
            PrivacyPageAction::ToggleHideSecretsInBlockList,
        )))
        .with_step(SecretTestCase::execute_command())
        .with_step(open_block_filter_editor())
        .with_step(SecretTestCase::perform_filter_query())
        .with_step(
            // Note: ideally, we shouldn't hardcode a secret handle ID here but we're doing this
            // for now. This is affected by the addition/removal of new `GridType`s!
            new_step_with_default_assertions("Click on secret to show tooltip")
                .with_click_on_saved_position("terminal_view:first_cell_in_secret_1")
                .add_assertion(assert_secret_tooltip_open(true))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);

                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let secret =
                            model.secret_at_point(&WithinModel::BlockList(WithinBlock::new(
                                Point::new(0, 24),
                                BlockIndex::zero(),
                                GridType::Output,
                            )));
                        async_assert!(secret.is_some(), "Secret exists")
                    })
                }),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_active_block() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_steps(LongRunningCommandTestCase::enter_input_into_cat())
        .with_step(open_block_filter_editor_for_long_running_command())
        .with_step(LongRunningCommandTestCase::perform_filter_query())
        .with_step(LongRunningCommandTestCase::exit_long_running_command())
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_block_filtering_clear_blocklist() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_steps(LongRunningCommandTestCase::enter_input_into_cat())
        .with_step(open_block_filter_editor_for_long_running_command())
        .with_step(LongRunningCommandTestCase::perform_filter_query())
        .with_step(
            TestStep::new("Escape block filter editor and clear blocklist")
                .with_keystrokes(&["escape", cmd_or_ctrl_shift("k").as_str()])
                .add_named_assertion(
                    "Assert that only the cursor line is included in the displayed rows",
                    |app, window_id| {
                        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                        terminal_view.read(app, |view, _ctx| {
                            let model = view.model.lock();
                            let displayed_output_rows = model
                                .block_list()
                                .active_block()
                                .displayed_output_rows()
                                .expect("No displayed output rows found.")
                                .collect::<Vec<_>>();
                            async_assert_eq!(displayed_output_rows, vec![0])
                        })
                    },
                ),
        )
        .with_step(LongRunningCommandTestCase::exit_long_running_command())
}
