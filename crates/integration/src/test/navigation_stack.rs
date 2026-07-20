use std::collections::HashMap;
use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::assertions::assert_binding_display_string;
use warp::integration_testing::command_palette::{
    TestStepsExt, open_command_palette_and_run_action,
};
use warp::integration_testing::pane_group::{assert_focused_pane_index, close_pane_by_index};
use warp::integration_testing::settings::toggle_setting;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    execute_command_for_single_terminal_in_tab, validate_block_output_on_finished_block,
    wait_until_bootstrapped_pane, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{
    pane_group_view, single_terminal_view_for_tab, terminal_view, workspace_view,
};
use warp::integration_testing::window::{
    add_and_save_window, assert_num_windows_open, close_window, save_active_window_id,
};
use warp::integration_testing::workspace::assert_tab_count;
use warp::integration_testing::{self};
use warp::settings_view::{FeaturesPageAction, SettingsAction};
use warp::terminal::block_list_viewport::ScrollPosition;
use warp::terminal::view::TerminalAction;
use warp::workspace::WorkspaceAction;
use warp::workspace::nav_stack::NavigationStack;
use warp::workspace::tab_settings::TabSettings;
use warp::{CodeEditorViewAction, cmd_or_ctrl_shift};
use warpui_core::integration::AssertionCallback;
use warpui_core::keymap::PerPlatformKeystroke;
use warpui_core::units::{Lines, Pixels};
use warpui_core::windowing::WindowManager;
use warpui_core::{
    App, ReadModel, SingletonEntity, TypedActionView, UpdateView, WindowId, async_assert,
    async_assert_eq,
};

use super::{Builder, TEST_ONLY_ASSETS, new_builder};
use crate::util::write_all_rc_files_for_test;

const NAVIGATE_BACK: PerPlatformKeystroke = PerPlatformKeystroke {
    mac: "ctrl--",
    linux_and_windows: "alt-left",
};

const NAVIGATE_FORWARD: PerPlatformKeystroke = PerPlatformKeystroke {
    mac: "ctrl-shift--",
    linux_and_windows: "alt-right",
};

fn assert_can_go_back(expected: bool) -> AssertionCallback {
    Box::new(move |app: &mut App, _window_id: WindowId| {
        let handle = NavigationStack::handle(app);
        app.read_model(&handle, |stack, _| {
            async_assert_eq!(
                stack.can_go_back(),
                expected,
                "Expected can_go_back={expected}, got {}",
                stack.can_go_back()
            )
        })
    })
}

fn assert_active_code_editor_focused(
    tab_index: usize,
    pane_index: usize,
    expected: bool,
) -> AssertionCallback {
    Box::new(move |app: &mut App, window_id: WindowId| {
        app.update(|_| {});
        let pane_group = pane_group_view(app, window_id, tab_index);
        let (code_view, editor_view) = pane_group.read(app, |pane_group, ctx| {
            let code_view = pane_group
                .code_view_at_pane_index(pane_index, ctx)
                .expect("should have code view at pane index");
            let editor_view = code_view
                .as_ref(ctx)
                .active_code_editor_view(ctx)
                .expect("should have active code editor view");
            (code_view, editor_view)
        });
        let (
            active_window,
            pane_group_window,
            editor_window,
            pane_group_focused,
            code_view_focused,
            editor_view_focused,
        ) = app.update(|ctx| {
            (
                ctx.windows().active_window(),
                pane_group.window_id(ctx),
                editor_view.window_id(ctx),
                pane_group.is_self_or_child_focused(ctx),
                code_view.is_self_or_child_focused(ctx),
                editor_view.is_self_or_child_focused(ctx),
            )
        });
        let active_editor_focused = active_window == Some(editor_window) && editor_view_focused;
        async_assert_eq!(
            active_editor_focused,
            expected,
            "Expected active code editor focused={expected}, got {active_editor_focused} (active_window={active_window:?}, pane_group_window={pane_group_window:?}, editor_window={editor_window:?}, pane_group_focused={pane_group_focused}, code_view_focused={code_view_focused}, editor_view_focused={editor_view_focused})"
        )
    })
}

fn assert_active_code_editor_text_contains(
    tab_index: usize,
    pane_index: usize,
    expected_substring: &'static str,
) -> AssertionCallback {
    Box::new(move |app: &mut App, window_id: WindowId| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        let editor_view = pane_group.read(app, |pane_group, ctx| {
            let code_view = pane_group
                .code_view_at_pane_index(pane_index, ctx)
                .expect("should have code view at pane index");
            code_view
                .as_ref(ctx)
                .active_code_editor_view(ctx)
                .expect("should have active code editor view")
        });
        let text = editor_view.read(app, |view, ctx| view.text(ctx).as_str().to_owned());
        async_assert!(
            text.contains(expected_substring),
            "Expected active code editor text to contain {expected_substring:?}, got {text:?}"
        )
    })
}

fn assert_visible_pane_count(tab_index: usize, expected: usize) -> AssertionCallback {
    Box::new(move |app: &mut App, window_id: WindowId| {
        let pane_group = pane_group_view(app, window_id, tab_index);
        pane_group.read(app, |pane_group, _ctx| {
            async_assert_eq!(
                pane_group.visible_pane_count(),
                expected,
                "Expected visible_pane_count={expected}, got {}",
                pane_group.visible_pane_count()
            )
        })
    })
}

fn clear_nav_stack() -> warpui_core::integration::TestStep {
    new_step_with_default_assertions("Clear nav stack").with_action(|app, _, _| {
        let handle = NavigationStack::handle(app);
        app.update(|ctx| {
            handle.update(ctx, |stack, _| {
                while stack.discard_back().is_some() {}
                while stack.discard_forward().is_some() {}
            });
        });
    })
}

fn assert_can_go_forward(expected: bool) -> AssertionCallback {
    Box::new(move |app: &mut App, _window_id: WindowId| {
        let handle = NavigationStack::handle(app);
        app.read_model(&handle, |stack, _| {
            async_assert_eq!(
                stack.can_go_forward(),
                expected,
                "Expected can_go_forward={expected}, got {}",
                stack.can_go_forward()
            )
        })
    })
}

fn assert_nav_stack_entry_count(expected: usize) -> AssertionCallback {
    Box::new(move |app: &mut App, _window_id: WindowId| {
        let handle = NavigationStack::handle(app);
        app.read_model(&handle, |stack, _| {
            async_assert_eq!(
                stack.entry_count(),
                expected,
                "Expected nav stack entry_count={expected}, got {}",
                stack.entry_count()
            )
        })
    })
}

fn assert_saved_position_visible(position_id: &'static str, expected: bool) -> AssertionCallback {
    Box::new(move |app: &mut App, window_id: WindowId| {
        let is_visible = app.read(|ctx| {
            ctx.element_position_by_id_at_last_frame(window_id, position_id)
                .is_some()
        });
        async_assert_eq!(
            is_visible,
            expected,
            "Expected saved position {position_id} visible={expected}, got {is_visible}"
        )
    })
}

fn assert_show_navigation_buttons_setting(expected: bool) -> AssertionCallback {
    Box::new(move |app: &mut App, _window_id: WindowId| {
        app.update(|ctx| {
            async_assert_eq!(*TabSettings::as_ref(ctx).show_navigation_buttons, expected)
        })
    })
}

/// Verifies the navigation stack is empty when the app starts up.
pub fn test_nav_stack_empty_on_startup() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Nav stack should be empty on startup")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
}

/// Verifies that clearing the navigation stack does not move the user away
/// from their current tab or terminal scroll position.
pub fn test_nav_stack_clear_command_preserves_tab_and_scroll_context() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    let lots_of_lines = (1..100).fold(String::new(), |mut s, _| {
        s.push_str("a\\n");
        s
    });
    let long_echo = format!("printf \"{lots_of_lines}\"");

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            long_echo,
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Click block header to scroll up")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected FixedAtPosition after clicking header, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Open a second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Navigate back to tab 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
        .with_steps(
            open_command_palette_and_run_action("Clear Navigation Stack")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(0))
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected FixedAtPosition after clearing nav stack, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
}

/// Verifies that clearing the navigation stack does not change the currently
/// focused pane within the active tab.
pub fn test_nav_stack_clear_command_preserves_focused_pane() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("Focus pane 0")
                .with_keystrokes(&["cmdorctrl-meta-left"])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions("Focus pane 1")
                .with_keystrokes(&["cmdorctrl-meta-right"])
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_can_go_back(true)),
        )
        .with_steps(
            open_command_palette_and_run_action("Clear Navigation Stack")
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
}

/// Verifies that after clearing the navigation stack, subsequent navigation
/// actions create fresh history again.
pub fn test_nav_stack_clear_command_allows_new_history_afterward() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open a second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_steps(
            open_command_palette_and_run_action("Clear Navigation Stack")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
        .with_step(
            new_step_with_default_assertions("Open a third tab after clearing history")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions("New navigation history is recorded again")
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(1)),
        )
}

/// Verifies that when the feature flag is disabled, nav-stack keybindings are
/// not registered and the tab-bar buttons do not render.
pub fn test_nav_stack_feature_flag_gates_bindings_and_buttons() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(false);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_binding_display_string(
            "workspace:navigate_back",
            None,
        ))
        .with_step(assert_binding_display_string(
            "workspace:navigate_forward",
            None,
        ))
        .with_step(
            new_step_with_default_assertions("Navigation buttons should be hidden")
                .add_assertion(assert_saved_position_visible(
                    "workspace:navigate_back_button",
                    false,
                ))
                .add_assertion(assert_saved_position_visible(
                    "workspace:navigate_forward_button",
                    false,
                )),
        )
}

/// Verifies that nav buttons are shown by default when enabled and can be
/// toggled off and back on from Settings > Features.
pub fn test_nav_stack_navigation_buttons_setting_toggle() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Navigation buttons are shown by default")
                .add_assertion(assert_saved_position_visible(
                    "workspace:navigate_back_button",
                    true,
                ))
                .add_assertion(assert_saved_position_visible(
                    "workspace:navigate_forward_button",
                    true,
                )),
        )
        .with_step(toggle_setting(SettingsAction::FeaturesPageToggle(
            FeaturesPageAction::ToggleShowNavigationButtons,
        )))
        .with_step(
            new_step_with_default_assertions("Navigation button setting is disabled")
                .add_assertion(assert_show_navigation_buttons_setting(false)),
        )
        .with_step(toggle_setting(SettingsAction::FeaturesPageToggle(
            FeaturesPageAction::ToggleShowNavigationButtons,
        )))
        .with_step(
            new_step_with_default_assertions("Navigation buttons are shown again when re-enabled")
                .add_assertion(assert_show_navigation_buttons_setting(true))
                .add_assertion(assert_saved_position_visible(
                    "workspace:navigate_back_button",
                    true,
                ))
                .add_assertion(assert_saved_position_visible(
                    "workspace:navigate_forward_button",
                    true,
                )),
        )
}

pub fn test_nav_stack_scroll_updates_back_button_state() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    let lots_of_lines = (1..100).fold(String::new(), |mut s, _| {
        s.push_str("a\\n");
        s
    });
    let long_echo = format!("printf \"{lots_of_lines}\"");

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            long_echo,
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Click block header to scroll up")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(view.scroll_position(), ScrollPosition::FixedAtPosition { .. }),
                            "Expected FixedAtPosition after clicking header, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Scroll down via action").with_action(
                move |app, window_id, _| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    app.update_view(&tv, |view, ctx| {
                        view.handle_action(
                            &TerminalAction::Scroll {
                                delta: Lines::new(10.0),
                            },
                            ctx,
                        );
                    });
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Wait for nav button state to update")
                .set_post_step_pause(Duration::from_secs(2))
                .add_assertion(assert_can_go_back(true)),
        )
        .with_step(
            new_step_with_default_assertions("Click back button to restore prior scroll")
                .with_click_on_saved_position("workspace:navigate_back_button")
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(view.scroll_position(), ScrollPosition::FixedAtPosition { .. }),
                            "Expected scroll restored to FixedAtPosition after clicking back, got {:?}",
                            view.scroll_position()
                        )
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
}

/// Verifies that Go Back and Go Forward are accessible from the command
/// palette in addition to their keybindings.
pub fn test_nav_stack_command_palette_back_forward() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open a second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_steps(
            open_command_palette_and_run_action("Go Back")
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
        .with_steps(
            open_command_palette_and_run_action("Go Forward")
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(false)),
        )
}
/// Verifies that the command palette can clear the entire navigation stack.
pub fn test_nav_stack_command_palette_clear() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open a second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_steps(
            open_command_palette_and_run_action("Go Back")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(true)),
        )
        .with_steps(
            open_command_palette_and_run_action("Clear Navigation Stack")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
}

/// Verifies that switching tabs records a navigation entry and that
/// navigating back restores the previous tab.
pub fn test_nav_stack_tab_switch_records_and_restores() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Open a second tab.
        .with_step(
            new_step_with_default_assertions("Open a second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Should be on tab 1 with a nav entry recorded")
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_can_go_forward(false)),
        )
        // Navigate back — should return to tab 0.
        .with_step(
            new_step_with_default_assertions("Navigate back to tab 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
        // Navigate forward — should return to tab 1.
        .with_step(
            new_step_with_default_assertions("Navigate forward to tab 1")
                .with_per_platform_keystroke(NAVIGATE_FORWARD)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(false)),
        )
}

/// Verifies that the tab-bar back and forward buttons navigate through
/// history just like the keyboard shortcuts do.
pub fn test_nav_stack_tab_bar_buttons_navigate() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open a second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Click the back button")
                .with_click_on_saved_position("workspace:navigate_back_button")
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
        .with_step(
            new_step_with_default_assertions("Click the forward button")
                .with_click_on_saved_position("workspace:navigate_forward_button")
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(false)),
        )
}

/// Verifies that navigating back then performing a new action clears the
/// forward stack, consistent with browser/IDE undo semantics.
pub fn test_nav_stack_new_action_clears_forward() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Open tabs 1 and 2.
        .with_step(
            new_step_with_default_assertions("Open second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Open third tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions("Now on tab 2, can go back")
                .add_assertion(assert_can_go_back(true)),
        )
        // Go back once (from tab 2 to tab 1).
        .with_step(
            new_step_with_default_assertions("Go back to tab 1")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
        // Now switch to tab 0 manually — this should clear the forward stack.
        .with_step(
            new_step_with_default_assertions("Switch to tab 0 manually, clearing forward stack")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_can_go_back(true)),
        )
}

/// Verifies that the feature flag gates all navigation behavior.
/// When the flag is off, switching tabs should NOT record entries.
pub fn test_nav_stack_feature_flag_gates_recording() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(false);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open second tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("With flag disabled, nav stack should remain empty")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
}

/// Verifies that splitting panes and switching pane focus within a single
/// tab records navigation entries so the user can go back.
pub fn test_nav_stack_pane_operations_no_entry() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Nav stack empty on startup")
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
        .with_step(
            new_step_with_default_assertions("Split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("Nav entry recorded after split")
                .add_assertion(assert_nav_stack_entry_count(1))
                .add_assertion(assert_can_go_back(true)),
        )
        .with_step(
            new_step_with_default_assertions("Switch focus to pane 0")
                .with_keystrokes(&["cmdorctrl-meta-left"])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions("Nav entry recorded after focus switch")
                .add_assertion(assert_nav_stack_entry_count(2))
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_can_go_forward(false)),
        )
        .with_step(
            new_step_with_default_assertions("Navigate back restores pane 1")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_can_go_forward(true)),
        )
        .with_step(
            new_step_with_default_assertions("Navigate back restores pane 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(assert_focused_pane_index(0, 0))
                .add_assertion(assert_can_go_back(false)),
        )
}

/// Verifies that when switching tabs with split panes, the focused pane
/// is captured in the navigation entry and restored on navigate back.
pub fn test_nav_stack_pane_focus_tracking() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Split tab 0 so pane 1 is focused.
        .with_step(
            new_step_with_default_assertions("Split pane in tab 0")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        // Open tab 1 — records entry {tab:0, pane:pane_1}.
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Should be on tab 1 with back available")
                .add_assertion(assert_can_go_back(true)),
        )
        // Navigate back — should restore tab 0 with pane 1 focused.
        .with_step(
            new_step_with_default_assertions("Navigate back to tab 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_can_go_forward(true)),
        )
        // Navigate forward — should restore tab 1.
        .with_step(
            new_step_with_default_assertions("Navigate forward to tab 1")
                .with_per_platform_keystroke(NAVIGATE_FORWARD)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(false)),
        )
}

/// Verifies that pane focus is correctly tracked across multiple tab switches.
/// Split tab 0, refocus pane 0, open tabs 1 and 2, then navigate all the way
/// back and verify pane 0 is focused in tab 0.
pub fn test_nav_stack_pane_focus_preserved_across_tabs() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Split tab 0 → pane 1 focused.
        .with_step(
            new_step_with_default_assertions("Split pane in tab 0")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        // Switch focus back to pane 0.
        .with_step(
            new_step_with_default_assertions("Focus pane 0")
                .with_keystrokes(&["cmdorctrl-meta-left"])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        // Open tab 1 — records entry {tab:0, pane:pane_0}.
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        // Open tab 2 — records entry {tab:1, pane:tab1_pane0}.
        .with_step(
            new_step_with_default_assertions("Open tab 2")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        // Navigate back to tab 1.
        .with_step(
            new_step_with_default_assertions("Back to tab 1")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                }),
        )
        // Navigate back to tab 0 — pane 0 should be focused.
        .with_step(
            new_step_with_default_assertions("Back to tab 0 with pane 0 focused")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_focused_pane_index(0, 0))
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_can_go_forward(true)),
        )
}

/// Verifies that navigating back from window 2 restores focus to window 1
/// when the back stack entry belongs to window 1.
pub fn test_nav_stack_multi_window_isolation() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Save window 1 id")
                .add_assertion(save_active_window_id("window_1")),
        )
        .with_step(
            new_step_with_default_assertions("Open tab 1 in window 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Verify nav entry exists")
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_nav_stack_entry_count(1)),
        )
        // Open window 2 — window 1 loses focus, recording another entry.
        .with_step(add_and_save_window("window_2"))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Cross-window entry recorded")
                .add_assertion(assert_nav_stack_entry_count(2)),
        )
        // Navigate back from window 2 — should focus window 1.
        .with_step(
            new_step_with_default_assertions("Navigate back focuses window 1")
                .with_per_platform_keystroke(NAVIGATE_BACK),
        )
        .with_step(
            new_step_with_default_assertions("Active window is window 1")
                .add_named_assertion_with_data_from_prior_step(
                    "Window 1 active",
                    |app, _window_id, data| {
                        let window_1_id: WindowId = *data
                            .get("window_1")
                            .expect("window_1 should be in step data");
                        let active = app.read(|ctx| WindowManager::as_ref(ctx).active_window());
                        async_assert_eq!(
                            active,
                            Some(window_1_id),
                            "Expected window 1 to be active"
                        )
                    },
                )
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_can_go_forward(true)),
        )
}

/// Verifies that scroll position is captured in navigation entries and
/// restored when navigating back.
pub fn test_nav_stack_scroll_position_restored() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    let lots_of_lines = (1..100).fold(String::new(), |mut s, _| {
        s.push_str("a\\n");
        s
    });
    let long_echo = format!("printf \"{lots_of_lines}\"");

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Create a block with enough output to scroll.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            long_echo,
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Verify at bottom").add_assertion(|app, window_id| {
                let tv = single_terminal_view_for_tab(app, window_id, 0);
                tv.read(app, |view, _| {
                    async_assert!(
                        matches!(
                            view.scroll_position(),
                            ScrollPosition::FollowsBottomOfMostRecentBlock
                        ),
                        "Expected FollowsBottomOfMostRecentBlock"
                    )
                })
            }),
        )
        // Click the block header to scroll to top (changes to FixedAtPosition).
        .with_step(
            new_step_with_default_assertions("Click block header to scroll up")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected FixedAtPosition after clicking header, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
        // Open tab 1 — records entry with FixedAtPosition scroll snapshot.
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        // Navigate back — should restore tab 0 with FixedAtPosition scroll.
        .with_step(
            new_step_with_default_assertions("Navigate back to tab 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected scroll position to be restored to FixedAtPosition, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
}

/// Verifies that session restoration across multiple tabs does NOT populate
/// the navigation stack. Restoring saved state should be invisible to the
/// user's navigation history.
pub fn test_nav_stack_session_restore_no_entries() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "three_tabs.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions(
                "After restoring 3 tabs the nav stack should remain empty",
            )
            .add_assertion(assert_can_go_back(false))
            .add_assertion(assert_can_go_forward(false))
            .add_assertion(assert_nav_stack_entry_count(0)),
        )
}

/// Verifies that switching between windows records navigation entries
/// and that navigating back restores the previous window's state.
pub fn test_nav_stack_cross_window_focus() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Save window 1 id")
                .add_assertion(save_active_window_id("window_1")),
        )
        .with_step(
            new_step_with_default_assertions("Open tab 1 in window 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Nav entry from tab switch")
                .add_assertion(assert_nav_stack_entry_count(1)),
        )
        // Open window 2 — focus moves from window 1 to window 2.
        // This should record an entry for window 1's current state (tab 1).
        .with_step(add_and_save_window("window_2"))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Cross-window entry should have been recorded")
                .add_assertion(assert_nav_stack_entry_count(2)),
        )
        // Navigate back from window 2 — should focus window 1.
        .with_step(
            new_step_with_default_assertions("Navigate back to window 1")
                .with_per_platform_keystroke(NAVIGATE_BACK),
        )
        .with_step(
            new_step_with_default_assertions("Active window should be window 1")
                .add_named_assertion_with_data_from_prior_step(
                    "Window 1 is active",
                    |app, _window_id, data| {
                        let window_1_id: WindowId = *data
                            .get("window_1")
                            .expect("window_1 should be in step data");
                        let active = app.read(|ctx| WindowManager::as_ref(ctx).active_window());
                        async_assert_eq!(
                            active,
                            Some(window_1_id),
                            "Expected window 1 to be active after navigate back"
                        )
                    },
                )
                .add_assertion(assert_can_go_forward(true)),
        )
}

/// Exercises forward navigation thoroughly: back through tabs, forward one
/// at a time with state assertions, then verifies a new action clears the
/// forward stack.
pub fn test_nav_stack_forward_after_back() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Open tabs 1 and 2.
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Open tab 2")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions("On tab 2, back stack has 2 entries")
                .add_assertion(assert_nav_stack_entry_count(2))
                .add_assertion(assert_can_go_back(true))
                .add_assertion(assert_can_go_forward(false)),
        )
        // Go back to tab 1.
        .with_step(
            new_step_with_default_assertions("Back to tab 1")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(true))
                .add_assertion(assert_can_go_back(true)),
        )
        // Forward back to tab 2.
        .with_step(
            new_step_with_default_assertions("Forward to tab 2")
                .with_per_platform_keystroke(NAVIGATE_FORWARD)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 2)
                    })
                })
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_can_go_back(true)),
        )
        // Go back twice: 2→1→0.
        .with_step(
            new_step_with_default_assertions("Back to tab 1 again")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Back to tab 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(true)),
        )
        // Forward from 0→1.
        .with_step(
            new_step_with_default_assertions("Forward to tab 1")
                .with_per_platform_keystroke(NAVIGATE_FORWARD)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(assert_can_go_forward(true))
                .add_assertion(assert_can_go_back(true)),
        )
        // Open a NEW tab — this should clear the forward stack.
        .with_step(
            new_step_with_default_assertions("Open tab 3, clearing forward stack")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(3))
        .with_step(
            new_step_with_default_assertions("Forward stack cleared after new action")
                .add_assertion(assert_can_go_forward(false))
                .add_assertion(assert_can_go_back(true)),
        )
}

/// Verifies that when switching pane focus (e.g. after a split), the
/// departing pane's scroll position is captured in the navigation entry
/// and restored when navigating back.
pub fn test_nav_stack_pane_focus_scroll_captured() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    let lots_of_lines = (1..100).fold(String::new(), |mut s, _| {
        s.push_str("a\\n");
        s
    });
    let long_echo = format!("printf \"{lots_of_lines}\"");

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            long_echo,
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Verify at bottom").add_assertion(|app, window_id| {
                let tv = single_terminal_view_for_tab(app, window_id, 0);
                tv.read(app, |view, _| {
                    async_assert!(
                        matches!(
                            view.scroll_position(),
                            ScrollPosition::FollowsBottomOfMostRecentBlock
                        ),
                        "Expected FollowsBottomOfMostRecentBlock"
                    )
                })
            }),
        )
        // Click block header to scroll up — changes to FixedAtPosition.
        .with_step(
            new_step_with_default_assertions("Click block header to scroll up")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected FixedAtPosition after clicking header, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
        // Split pane — new pane 1 gets focus, recording pane 0's scroll.
        .with_step(
            new_step_with_default_assertions("Split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("Nav entry recorded after split")
                .add_assertion(assert_nav_stack_entry_count(1))
                .add_assertion(assert_can_go_back(true)),
        )
        // Navigate back — should restore pane 0 with FixedAtPosition scroll.
        .with_step(
            new_step_with_default_assertions("Navigate back to pane 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(assert_focused_pane_index(0, 0))
                .add_assertion(|app, window_id| {
                    let tv = terminal_view(app, window_id, 0, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected scroll restored to FixedAtPosition, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
}

/// Verifies that scrolling within a single pane records a debounced
/// navigation entry and that navigating back restores the original
/// scroll position.
pub fn test_nav_stack_scroll_within_pane() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    let lots_of_lines = (1..100).fold(String::new(), |mut s, _| {
        s.push_str("a\\n");
        s
    });
    let long_echo = format!("printf \"{lots_of_lines}\"");

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            long_echo,
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Verify at bottom").add_assertion(|app, window_id| {
                let tv = single_terminal_view_for_tab(app, window_id, 0);
                tv.read(app, |view, _| {
                    async_assert!(
                        matches!(
                            view.scroll_position(),
                            ScrollPosition::FollowsBottomOfMostRecentBlock
                        ),
                        "Expected FollowsBottomOfMostRecentBlock"
                    )
                })
            }),
        )
        // Click block header to scroll up — changes to FixedAtPosition.
        .with_step(
            new_step_with_default_assertions("Click block header to scroll up")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected FixedAtPosition after clicking header, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
        // Dispatch a scroll action to trigger the UserScrolled event from the FixedAtPosition state.
        .with_step(
            new_step_with_default_assertions("Scroll down via action").with_action(
                move |app, window_id, _| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    app.update_view(&tv, |view, ctx| {
                        view.handle_action(
                            &TerminalAction::Scroll {
                                delta: Lines::new(10.0),
                            },
                            ctx,
                        );
                    });
                },
            ),
        )
        // Navigate back — flush() commits the pending debounced entry, then
        // go_back restores the FixedAtPosition from the block header click.
        .with_step(
            new_step_with_default_assertions("Navigate back to original scroll")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let tv = single_terminal_view_for_tab(app, window_id, 0);
                    tv.read(app, |view, _| {
                        async_assert!(
                            matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            ),
                            "Expected scroll restored to FixedAtPosition, got {:?}",
                            view.scroll_position()
                        )
                    })
                }),
        )
}

/// Verifies that scrolling in a code editor pane records a debounced navigation
/// entry, that navigating back to the code pane restores real editor focus, and
/// that the original scroll position can still be restored afterward.
pub fn test_nav_stack_code_editor_scroll() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    let lines: String = (1..=200)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    new_builder()
        .with_setup(move |utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));
            std::fs::write(test_dir.join("big_file.txt"), &lines)
                .expect("Failed to create big_file.txt");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open file tree panel").with_action(|app, _, _| {
                let window_id = app.read(|ctx| {
                    ctx.windows()
                        .active_window()
                        .expect("should have active window")
                });
                let workspace = workspace_view(app, window_id);
                app.update(|ctx| {
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        workspace.id(),
                        &WorkspaceAction::ToggleProjectExplorer,
                    );
                });
            }),
        )
        .with_step(
            new_step_with_default_assertions("Click big_file.txt in file tree")
                .with_click_on_saved_position("file_tree_item:big_file.txt")
                .add_assertion(|app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pg, _ctx| {
                        async_assert_eq!(
                            pg.pane_count(),
                            2,
                            "Expected 2 panes after opening file (terminal + editor)"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Scroll code editor down").with_action(
                |app, window_id, _| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    let editor_view = pane_group.read(app, |pg, ctx| {
                        let cv = pg
                            .code_view_at_pane_index(1, ctx)
                            .expect("should have code view at pane 1");
                        cv.as_ref(ctx)
                            .active_code_editor_view(ctx)
                            .expect("should have active code editor view")
                    });
                    app.update_view(&editor_view, |view, ctx| {
                        view.handle_action(
                            &CodeEditorViewAction::ScrollVertical(Pixels::new(-500.0)),
                            ctx,
                        );
                    });
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Focus terminal pane")
                .with_action(|app, window_id, _| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    let pane_id = pane_group.read(app, |pane_group, _ctx| {
                        pane_group
                            .pane_id_from_index(0)
                            .expect("should have terminal pane at index 0")
                    });
                    pane_group.update(app, |pane_group, ctx| {
                        pane_group.focus_pane_by_id(pane_id, ctx);
                    });
                })
                .add_assertion(assert_focused_pane_index(0, 0))
                .add_assertion(assert_active_code_editor_focused(0, 1, false)),
        )
        .with_step(
            new_step_with_default_assertions("Navigate back restores code editor focus")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .set_post_step_pause(Duration::from_millis(250))
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_active_code_editor_focused(0, 1, true)),
        )
        .with_step(
            new_step_with_default_assertions("Typing works immediately after navigation restore")
                .with_typed_characters(&["navfocus"])
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_active_code_editor_text_contains(0, 1, "navfocus")),
        )
        .with_step(
            new_step_with_default_assertions("Navigate back restores scroll")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    pane_group.read(app, |pg, ctx| {
                        let cv = pg
                            .code_view_at_pane_index(1, ctx)
                            .expect("should have code view");
                        let render_state = cv
                            .as_ref(ctx)
                            .active_editor_render_state(ctx)
                            .expect("should have render state");
                        let scroll_top = render_state.as_ref(ctx).viewport().scroll_top();
                        async_assert!(
                            scroll_top == Pixels::zero(),
                            "Expected scroll restored to top (0), got {scroll_top:?}"
                        )
                    })
                })
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_active_code_editor_focused(0, 1, true)),
        )
}

/// Verifies that a nav-stack entry pointing at a temporarily closed pane restores
/// that pane when the user navigates back before undo-close cleanup expires.
pub fn test_nav_stack_restores_closed_pane_when_available() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    FeatureFlag::UndoClosedPanes.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("Focus pane 0")
                .with_keystrokes(&["cmdorctrl-meta-left"])
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(
            close_pane_by_index(0, 1)
                .add_assertion(assert_focused_pane_index(0, 0))
                .add_assertion(assert_visible_pane_count(0, 1))
                .add_assertion(assert_can_go_back(true)),
        )
        .with_step(
            new_step_with_default_assertions("Navigate back restores closed pane")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(assert_focused_pane_index(0, 1))
                .add_assertion(assert_visible_pane_count(0, 2))
                .add_assertion(assert_can_go_forward(true)),
        )
}

/// Verifies that a nav-stack entry pointing at a closed tab restores that tab
/// when the tab is still within the undo-close grace period.
pub fn test_nav_stack_restores_closed_tab_when_available() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "echo \"restored tab\"".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Switch back to tab 0")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_back(true)),
        )
        .with_step(
            new_step_with_default_assertions("Close tab 1")
                .with_hover_over_saved_position("close_tab_button:1")
                .with_click_on_saved_position("close_tab_button:1")
                .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Navigate back restores closed tab")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(assert_tab_count(2))
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                })
                .add_assertion(|app, window_id| {
                    validate_block_output_on_finished_block("restored tab", 1, 0, window_id, app)
                }),
        )
}

/// Verifies that a nav-stack entry pointing at a recently closed window reopens
/// that window when navigating back before undo-close cleanup expires.
pub fn test_nav_stack_restores_closed_window_when_available() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Save window 1 id")
                .add_assertion(save_active_window_id("window_1")),
        )
        .with_step(add_and_save_window("window_2"))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Focus window 1 again").with_action(|app, _, data| {
                let window_1: WindowId = *data
                    .get("window_1")
                    .expect("window_1 should be in step data");
                app.update(|ctx| {
                    WindowManager::as_ref(ctx).show_window_and_focus_app(window_1);
                });
            }),
        )
        .with_step(close_window("window_2", 1).set_post_step_pause(Duration::from_millis(500)))
        .with_step(
            new_step_with_default_assertions("Navigate back reopens window 2")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .set_post_step_pause(Duration::from_millis(500)),
        )
        .with_step(
            new_step_with_default_assertions("Window 2 restored and focused")
                .add_assertion(assert_num_windows_open(2))
                .add_named_assertion_with_data_from_prior_step(
                    "window 2 is active again",
                    |app, _window_id, data| {
                        let window_2: WindowId = *data
                            .get("window_2")
                            .expect("window_2 should be in step data");
                        let active = app.read(|ctx| WindowManager::as_ref(ctx).active_window());
                        async_assert_eq!(active, Some(window_2))
                    },
                ),
        )
}

/// Verifies that once a closed tab expires out of undo-close, its stale nav-stack
/// entry is pruned instead of restoring or leaving back-navigation stuck.
pub fn test_nav_stack_prunes_expired_closed_tab_entries() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            "UndoCloseGracePeriod".to_owned(),
            serde_json::to_string(&Duration::from_secs(1))
                .expect("Duration should serialize to JSON"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(clear_nav_stack())
        .with_step(
            new_step_with_default_assertions("Switch back to tab 0")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_assertion(assert_can_go_back(true)),
        )
        .with_step(
            new_step_with_default_assertions("Close tab 1")
                .with_hover_over_saved_position("close_tab_button:1")
                .with_click_on_saved_position("close_tab_button:1")
                .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Wait for closed tab to expire")
                .set_timeout(Duration::from_secs(3)),
        )
        .with_step(
            new_step_with_default_assertions("Expired closed-tab entry pruned")
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_nav_stack_entry_count(0)),
        )
        .with_step(
            new_step_with_default_assertions("Back is a no-op after prune")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(assert_tab_count(1))
                .add_assertion(|app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                }),
        )
}

/// Verifies that navigating back and forward through multiple tabs works
/// correctly for a longer history.
pub fn test_nav_stack_multiple_back_forward() -> Builder {
    FeatureFlag::NavigationStack.set_enabled(true);
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open tab 1")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Open tab 2")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions("Open tab 3")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(3))
        // Now go back three times: 3→2→1→0
        .with_step(
            new_step_with_default_assertions("Back to tab 2")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 2)
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Back to tab 1")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Back to tab 0")
                .with_per_platform_keystroke(NAVIGATE_BACK)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 0)
                    })
                })
                .add_assertion(assert_can_go_back(false))
                .add_assertion(assert_can_go_forward(true)),
        )
        // Now go forward twice: 0→1→2
        .with_step(
            new_step_with_default_assertions("Forward to tab 1")
                .with_per_platform_keystroke(NAVIGATE_FORWARD)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 1)
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Forward to tab 2")
                .with_per_platform_keystroke(NAVIGATE_FORWARD)
                .add_assertion(|app, window_id| {
                    let workspace =
                        warp::integration_testing::view_getters::workspace_view(app, window_id);
                    workspace.read(app, |view, _ctx| {
                        async_assert_eq!(view.active_tab_index(), 2)
                    })
                })
                .add_assertion(assert_can_go_forward(true)),
        )
}
