use std::path::PathBuf;
use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::pane_group::assert_focused_pane_index;
use warp::integration_testing::settings::set_window_custom_size;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    validate_block_output, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::type_getters::get_launch_config_ui_location;
use warp::integration_testing::window::assert_num_windows_open;
use warp::integration_testing::workspace::{assert_focused_tab_index, assert_tab_count};
use warp::integration_testing::{self};
use warp::search::command_palette::launch_config;
use warp::search::data_source::Query;
use warp::search::SyncDataSource;
use warp::workspace::NEW_TAB_BUTTON_POSITION_ID;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::{async_assert, ModelHandle};

use super::{assert_approx_eq, new_builder, TEST_ONLY_ASSETS};
use crate::Builder;

/// Adds a launch config to the mocked out warp config directory and verifies that
/// the launch config appears in the launch config palette.
pub fn test_add_launch_config_to_warp_config() -> Builder {
    new_builder()
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some((10).to_string()));

            std::fs::create_dir_all(integration_testing::launch_configs::launch_configs_dir())
                .expect("Should be able to create launch configs dir");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Launch config palette should be empty").add_named_assertion(
                "Launch config palette should be empty",
                |app, _| {
                    let launch_config_data_source: ModelHandle<launch_config::DataSource> = app
                        .models_of_type()
                        .first()
                        .expect("launch config must exist")
                        .clone();
                    launch_config_data_source.read(app, |palette, app| {
                        // Note that this can be a synchronous assertion because unlike the next test step,
                        // we don't have concurrency with a WarpConfig watcher thread
                        assert_eq!(
                            palette.run_query(&Query::from(""), app).unwrap().len(),
                            0,
                            "There should not be any launch configs in the palette"
                        );
                    });
                    AssertionOutcome::Success
                },
            ),
        )
        .with_step(
            TestStep::new("Write a new launch config")
                .with_setup(|_utils| {
                    integration_testing::create_file_from_assets(
                        TEST_ONLY_ASSETS,
                        "test_launch_config.yaml",
                        &integration_testing::launch_configs::launch_configs_dir()
                            .join("test_launch_config.yaml"),
                    );
                })
                .add_named_assertion(
                    "The added launch config should be in the palette",
                    |app, _| {
                        let launch_config_data_source: ModelHandle<launch_config::DataSource> = app
                            .models_of_type()
                            .first()
                            .expect("launch config must exist")
                            .clone();
                        let num_configs = launch_config_data_source.read(app, |palette, ctx| {
                            palette.run_query(&Query::from(""), ctx).unwrap().len()
                        });
                        async_assert!(
                            num_configs == 1,
                            "Expected to find one launch config, instead found {}",
                            num_configs
                        )
                    },
                ),
        )
}

pub fn test_with_launch_config() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we have only 1 window open at start")
            .add_assertion(assert_num_windows_open(1)),
        )
        .with_step(
            new_step_with_default_assertions("Opening a configuration template").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config:
                                warp::launch_configs::launch_config::make_mock_single_window_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: false,
                        },
                    );
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Assert the new window matches template")
                .add_named_assertion("Created a new window", move |app, _| {
                    assert_eq!(app.window_ids().len(), 2);
                    AssertionOutcome::Success
                })
                .add_assertion(assert_tab_count(2))
                .add_named_assertion("Validate first tab", move |app, window_id| {
                    validate_block_output("test_command", 0, 0, window_id, app)
                })
                .add_named_assertion("Validate second tab", move |app, window_id| {
                    validate_block_output("test_command_on_another_tab", 1, 0, window_id, app)
                }),
        )
}

// TODO(CORE-2300): Once we remove FeatureFlag::ShellSelector, we should remove this test.
pub fn test_open_launch_config_from_add_tab_menu_legacy() -> Builder {
    new_builder()
        .set_should_run_test(|| !FeatureFlag::ShellSelector.is_enabled())
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some((10).to_string()));

            // Write a new launch config file. Launch config is named "Launch Config"
            let dir = integration_testing::launch_configs::launch_configs_dir();
            std::fs::create_dir_all(&dir).expect("Should be able to create launch configs dir");
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test_launch_config.yaml",
                &dir.join("test_launch_config.yaml"),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Right click on new tab button")
                .with_right_click_on_saved_position(NEW_TAB_BUTTON_POSITION_ID),
        )
        .with_step(
            new_step_with_default_assertions("Press Launch Config menu item")
                // Since we only have one launch config, it should be the third menu item and the
                // second one is disabled.
                .with_keystrokes(&["down", "down", "enter"]),
        )
        .with_step(
            new_step_with_default_assertions("Assert that three new windows are created")
                .add_assertion(assert_num_windows_open(4)),
        )
}

pub fn test_launch_config_single_child_branch() -> Builder {
    use warp::launch_configs::launch_config::{
        LaunchConfig, PaneMode, PaneTemplateType, SplitDirection, TabTemplate, WindowTemplate,
    };
    use warpui::actions::StandardAction;

    /// Create a launch config that has a branch with a single child
    fn create_launch_config() -> LaunchConfig {
        LaunchConfig {
            name: "Mocked config".to_owned(),
            active_window_index: Some(0),
            windows: vec![WindowTemplate {
                active_tab_index: Some(0),
                tabs: vec![TabTemplate {
                    title: Some("First tab".to_owned()),
                    layout: PaneTemplateType::PaneBranchTemplate {
                        split_direction: SplitDirection::Horizontal,
                        panes: vec![PaneTemplateType::PaneTemplate {
                            is_focused: Some(true),
                            cwd: PathBuf::from("/some/path"),
                            commands: Vec::new(),
                            pane_mode: PaneMode::Terminal,
                            shell: None,
                        }],
                    },
                    color: None,
                }],
            }],
        }
    }

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Opening a launch config with single child branch")
                .with_action(move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config: create_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: false,
                        },
                    );
                }),
        )
        .with_step(
            new_step_with_default_assertions("Close the open pane with standard action")
                .add_assertion(|app, window_id| {
                    app.dispatch_standard_action(window_id, StandardAction::Close);

                    // If we get here without panicking, then we are successful
                    AssertionOutcome::Success
                }),
        )
}

pub fn test_open_launch_config_with_custom_size() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we only have 1 window open at start")
            .add_assertion(assert_num_windows_open(1)),
        )
        .with_step(set_window_custom_size(40, 20))
        .with_step(
            new_step_with_default_assertions("Open a launch configuration").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config:
                                warp::launch_configs::launch_config::make_mock_single_window_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: false,
                        },
                    )
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Assert the new window uses the custom size")
                .add_named_assertion("Validate window size", move |app, window_id| {
                    let size = app
                        .window_bounds(&window_id)
                        .expect("Window should exist")
                        .size();
                    // This doesn't correspond clearly to the given rows and columns due to line
                    // height and padding. There's also some platform-specific variance and room
                    // for floating-point error.
                    assert_approx_eq!(f32, size.x(), 192., epsilon = 2.);
                    assert_approx_eq!(f32, size.y(), 644., epsilon = 2.);
                    AssertionOutcome::Success
                }),
        )
}

pub fn test_open_launch_config_in_active_window() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we only have 1 window, 1 tab open at start")
                .add_assertion(assert_num_windows_open(1))
                .add_assertion(assert_tab_count(1))
        )
        .with_step(
            new_step_with_default_assertions("Open a launch configuration").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config:
                                warp::launch_configs::launch_config::make_mock_single_window_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: true,
                        },
                    )
                },
            )
            // Add a post-step pause so that we can make sure any windows were opened
            // in time, if they were going to be.
            .set_post_step_pause(Duration::from_secs(1))
        )
        .with_step(
            new_step_with_default_assertions("Assert we only have 1 window, 3 tabs (1 old, 2 new) after launching")
                .add_assertion(assert_num_windows_open(1))
                .add_assertion(assert_tab_count(3))
        )
}

pub fn test_with_launch_config_with_active_tab_index() -> Builder {
    use warp::launch_configs::launch_config::{
        LaunchConfig, PaneMode, PaneTemplateType, SplitDirection, TabTemplate, WindowTemplate,
    };

    fn create_launch_config() -> LaunchConfig {
        LaunchConfig {
            name: "Mocked config".to_owned(),
            active_window_index: Some(0),
            windows: vec![WindowTemplate {
                active_tab_index: Some(1),
                tabs: vec![
                    TabTemplate {
                        title: None,
                        layout: PaneTemplateType::PaneBranchTemplate {
                            split_direction: SplitDirection::Horizontal,
                            panes: vec![PaneTemplateType::PaneTemplate {
                                is_focused: Some(true),
                                cwd: PathBuf::from("/some/path"),
                                commands: Vec::new(),
                                pane_mode: PaneMode::Terminal,
                                shell: None,
                            }],
                        },
                        color: None,
                    };
                    3
                ],
            }],
        }
    }

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we have only 1 window open at start")
                .add_assertion(assert_num_windows_open(1)),
        )
        .with_step(
            new_step_with_default_assertions("Opening a configuration template").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config: create_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: false,
                        },
                    );
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Assert the new window matches template")
                .add_assertion(assert_tab_count(3))
                .add_assertion(assert_focused_tab_index(1)),
        )
}

pub fn test_with_launch_config_with_active_pane() -> Builder {
    use warp::launch_configs::launch_config::{
        LaunchConfig, PaneMode, PaneTemplateType, SplitDirection, TabTemplate, WindowTemplate,
    };

    fn create_launch_config() -> LaunchConfig {
        LaunchConfig {
            name: "Mocked config".to_owned(),
            active_window_index: Some(0),
            windows: vec![WindowTemplate {
                active_tab_index: Some(0),
                tabs: vec![TabTemplate {
                    title: None,
                    layout: PaneTemplateType::PaneBranchTemplate {
                        split_direction: SplitDirection::Horizontal,
                        panes: vec![
                            PaneTemplateType::PaneTemplate {
                                is_focused: Some(false),
                                cwd: PathBuf::from("/some/path"),
                                commands: Vec::new(),
                                pane_mode: PaneMode::Terminal,
                                shell: None,
                            },
                            PaneTemplateType::PaneBranchTemplate {
                                split_direction: SplitDirection::Vertical,
                                panes: vec![
                                    PaneTemplateType::PaneTemplate {
                                        is_focused: Some(false),
                                        cwd: PathBuf::from("/some/path"),
                                        commands: Vec::new(),
                                        pane_mode: PaneMode::Terminal,
                                        shell: None,
                                    },
                                    PaneTemplateType::PaneTemplate {
                                        is_focused: Some(true),
                                        cwd: PathBuf::from("/some/path"),
                                        commands: Vec::new(),
                                        pane_mode: PaneMode::Terminal,
                                        shell: None,
                                    },
                                ],
                            },
                        ],
                    },
                    color: None,
                }],
            }],
        }
    }

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we have only 1 window open at start")
                .add_assertion(assert_num_windows_open(1)),
        )
        .with_step(
            new_step_with_default_assertions("Opening a configuration template").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config: create_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: false,
                        },
                    );
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Assert the bottom right pane is selected")
                .add_assertion(assert_tab_count(1))
                .add_assertion(assert_focused_tab_index(0))
                .add_assertion(assert_focused_pane_index(0, 2)),
        )
}

pub fn test_with_launch_config_with_no_active_pane() -> Builder {
    use warp::launch_configs::launch_config::{
        LaunchConfig, PaneMode, PaneTemplateType, SplitDirection, TabTemplate, WindowTemplate,
    };

    fn create_launch_config() -> LaunchConfig {
        LaunchConfig {
            name: "Mocked config".to_owned(),
            active_window_index: Some(0),
            windows: vec![WindowTemplate {
                active_tab_index: Some(0),
                tabs: vec![TabTemplate {
                    title: None,
                    layout: PaneTemplateType::PaneBranchTemplate {
                        split_direction: SplitDirection::Horizontal,
                        panes: vec![
                            PaneTemplateType::PaneTemplate {
                                is_focused: Some(false),
                                cwd: PathBuf::from("/some/path"),
                                commands: Vec::new(),
                                pane_mode: PaneMode::Terminal,
                                shell: None,
                            },
                            PaneTemplateType::PaneBranchTemplate {
                                split_direction: SplitDirection::Vertical,
                                panes: vec![
                                    PaneTemplateType::PaneTemplate {
                                        is_focused: Some(false),
                                        cwd: PathBuf::from("/some/path"),
                                        commands: Vec::new(),
                                        pane_mode: PaneMode::Terminal,
                                        shell: None,
                                    },
                                    PaneTemplateType::PaneTemplate {
                                        is_focused: Some(false),
                                        cwd: PathBuf::from("/some/path"),
                                        commands: Vec::new(),
                                        pane_mode: PaneMode::Terminal,
                                        shell: None,
                                    },
                                ],
                            },
                        ],
                    },
                    color: None,
                }],
            }],
        }
    }

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we have only 1 window open at start")
                .add_assertion(assert_num_windows_open(1)),
        )
        .with_step(
            new_step_with_default_assertions("Opening a configuration template").with_action(
                move |app, _, _| {
                    app.dispatch_global_action(
                        "root_view:open_launch_config",
                        warp::root_view::OpenLaunchConfigArg {
                            launch_config: create_launch_config(),
                            ui_location: get_launch_config_ui_location(),
                            open_in_active_window: false,
                        },
                    );
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Assert the leftmost/topmost pane is focused")
                .add_assertion(assert_tab_count(1))
                .add_assertion(assert_focused_tab_index(0))
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
}
