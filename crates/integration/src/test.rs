//! Note that for all of these tests, you need to also update
//! src/integration.rs and src/bin/integration.rs in order to register them
//! to be run.

mod agent_mode;
mod ai_assistant;
mod block_filtering;
mod bootstrapping;
mod code_review;
mod ctrl_d;
mod file_tree;
mod goto_line;
mod history;
mod input;
mod keyboard_protocol;
mod launch_configs;
mod notebooks;
mod pane_restoration;
#[cfg(target_os = "macos")]
mod preview_config_migration;
mod remote_server;
mod rules;
mod secrets;
mod session_restoration;
mod settings_file_errors;
mod settings_file_hot_reload;
mod settings_file_migration;
mod settings_private;
mod ssh;
mod subshell;
mod sync_inputs;
mod typeahead;
mod video_recording;
mod websockets;
mod workflows;
mod workspace;

pub use agent_mode::*;
pub use ai_assistant::*;
pub use block_filtering::*;
pub use bootstrapping::*;
pub use code_review::*;
pub use ctrl_d::*;
pub use file_tree::*;
use float_cmp::assert_approx_eq;
pub use goto_line::*;
pub use history::*;
pub use input::*;
pub use keyboard_protocol::*;
pub use launch_configs::*;
pub use notebooks::*;
pub use pane_restoration::*;
#[cfg(target_os = "macos")]
pub use preview_config_migration::*;
pub use remote_server::*;
pub use rules::*;
pub use secrets::*;
pub use session_restoration::*;
pub use settings_file_errors::*;
pub use settings_file_hot_reload::*;
pub use settings_file_migration::*;
pub use settings_private::*;
pub use ssh::*;
pub use subshell::*;
pub use sync_inputs::*;
pub use typeahead::*;
pub use video_recording::*;
pub use websockets::*;
pub use workflows::*;
pub use workspace::*;

use std::{borrow::Cow, collections::HashMap, path::PathBuf, rc::Rc, time::Duration};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use pathfinder_geometry::{rect::RectF, vector::Vector2F};
use rust_embed::RustEmbed;
use settings::Setting as _;
use shell::ShellType;
use warpui::{
    async_assert, async_assert_eq,
    integration::{AssertionOutcome, StepData, TestStep},
    keymap::{Keystroke, Trigger},
    platform::{OperatingSystem, TerminationMode},
    windowing::WindowManager,
    AssetProvider, Event, SingletonEntity, UpdateView, ViewHandle,
};

use warp::{terminal::find::TerminalFindModel, util::bindings::CustomAction, AgentModeEntrypoint};

use sysinfo::{Pid, ProcessesToUpdate, System};
use version_compare::Cmp;
use warpui::units::Lines;

use crate::util::{skip_if_powershell_core_2303, ShellRcType};

use crate::builder::cargo_target_tmpdir;
use crate::user_defaults;
use crate::Builder;
use sum_tree::SeekBias;
use warp::integration_testing::terminal::assert_focused_editor_in_tab;
use warp::integration_testing::{
    settings::assert_theme_chooser_contains,
    tab::{assert_pane_title, assert_tab_title},
};
use warp::settings::CtrlTabBehavior;
use warp::terminal::keys_settings::KeysSettings;
use warp::terminal::{
    model::{blocks::BlockHeightSummary, terminal_model::BlockIndex},
    view::TerminalViewState,
};
use warp::workflows::categories::CategoriesView;
use warp::{
    appearance::Appearance,
    cmd_or_ctrl_shift,
    integration_testing::{
        assertions::{assert_binding_display_string, go_offline, go_online},
        block::{
            assert_block_visible, assert_bottom_of_block_approx_at, assert_num_blocks_in_model,
            BlockPosition, LinePosition,
        },
        clipboard::assert_clipboard_contains_string,
        context_chips::assert_working_dir_is_present,
        input::open_input_context_menu,
        navigation_palette::{
            check_recency, navigate_to_other_session_step, open_navigation_palette_step,
            RecentSession,
        },
        settings::toggle_setting,
        step::{
            assert_no_pending_model_events, new_step_with_default_assertions,
            new_step_with_default_assertions_for_pane,
        },
        tab::tab_title_step,
        terminal::{
            assert_active_block_output_for_single_terminal_in_tab,
            assert_active_block_received_precmd,
            assert_command_executed_for_single_terminal_in_tab, assert_context_menu_is_open,
            assert_gap_exists, assert_input_at_bottom_of_terminal, assert_input_at_top_of_terminal,
            assert_input_mode, assert_input_not_at_either_edge_of_terminal,
            assert_long_running_block_executing_for_single_terminal_in_tab, assert_model_term_mode,
            assert_pane_group_has_state, assert_scroll_position,
            assert_selected_block_index_is_first_renderable,
            assert_selected_block_index_is_last_renderable,
            assert_single_terminal_in_tab_bootstrapped, assert_snackbar_is_not_visible,
            assert_snackbar_is_visible, assert_view_has_text_selection,
            assert_waterfall_gap_empty_background_rendered,
            execute_command_for_single_terminal_in_tab, execute_echo, execute_long_running_command,
            execute_python_interpreter_in_tab, performance_test, run_alt_grid_program,
            run_completer, util::current_shell_starter_and_version, util::ExpectedExitStatus,
            validate_git_branch, wait_until_bootstrapped_pane,
            wait_until_bootstrapped_single_pane_for_tab,
        },
        view_getters::{
            single_input_suggestions_view_for_tab, single_input_view_for_tab,
            single_terminal_view_for_tab,
        },
        view_of_type,
        window::{add_window, add_window_and_check_bounds, close_window, save_active_window_id},
    },
    settings::{TabBehavior, INPUT_MODE},
    settings_view::FeaturesPageAction,
    terminal::{
        alt_screen_reporting::MouseReportingEnabled,
        block_list_viewport::{InputMode, ScrollPosition},
        model::grid::grid_handler::TermMode,
        session_settings::SessionSettings,
        session_settings::{HonorPS1, StartupShellOverride},
        view::BlockVisibilityMode,
    },
};

use warp::terminal::view::ALIAS_EXPANSION_BANNER_SEEN_KEY;
use warp::{
    features::FeatureFlag,
    integration_testing::{
        find::{Find, FindWithinBlockState},
        pane_group::assert_focused_pane_index,
        settings::set_window_custom_size,
        terminal::assert_terminal_bootstrapping,
        view_getters::pane_group_view,
        window::add_and_save_window,
    },
};
use warp::{
    integration_testing::warp_drive::{
        assert_is_left_panel_open, assert_warp_drive_is_closed, assert_warp_drive_is_open,
    },
    settings::CompletionsOpenWhileTyping,
};
use warp::{
    integration_testing::{
        self,
        input::{input_contains_string, input_is_empty},
        terminal::{
            clear_blocklist_to_remove_bootstrapped_blocks, open_context_menu_for_selected_block,
        },
    },
    settings::MonospaceFontSize,
};
use warp::{
    integration_testing::{assertions::join_a_workspace, view_getters::single_terminal_view},
    terminal::view::TerminalAction,
};
use warp::{
    integration_testing::{
        command_palette::{
            close_command_palette, open_command_palette, open_command_palette_and_run_action,
            TestStepsExt,
        },
        view_getters::single_terminal_pane_view_for_tab,
    },
    pane_group::AGENT_MODE_PANE_DEFAULT_MINIMUM_WIDTH,
};
use warp::{
    integration_testing::{terminal::util::ExactLine, workspace::assert_tab_count},
    terminal::available_shells::AvailableShells,
};
use warp::{
    integration_testing::{
        terminal::{
            assert_active_block_output, assert_alt_grid_active, assert_alt_screen_output,
            assert_long_running_block_executing, assert_terminal_bootstrapped,
            execute_long_running_command_for_pane,
        },
        view_getters::workspace_view,
    },
    workspace::WorkspaceAction,
};
use warp::{settings_view::SettingsAction, terminal::block_list_viewport::ScrollLines};
use warp::{
    settings_view::{keybindings::KeybindingsView, SettingsSection, SettingsView},
    terminal::{
        input::{Input, InputSuggestionsMode},
        model::{
            ansi::{Handler, InitShellValue},
            blocks::{BlockHeightItem, TotalIndex},
            grid::Dimensions,
        },
        shell, TerminalView,
    },
    workspace::{Workspace, NEW_SESSION_MENU_BUTTON_POSITION_ID, NEW_TAB_BUTTON_POSITION_ID},
};
use warpui::event::KeyState;
use warpui::keymap::PerPlatformKeystroke;
use warpui::platform::keyboard::KeyCode;

const ADD_NEXT_OCCURRENCE_KEYBINDING: &str = "ctrl-g";

#[derive(Clone, Copy, RustEmbed)]
#[folder = "tests/data/"]
pub struct TestOnlyAssets;

pub static TEST_ONLY_ASSETS: TestOnlyAssets = TestOnlyAssets;

impl AssetProvider for TestOnlyAssets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Self as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

use super::util::{self, write_all_rc_files_for_test, write_rc_files_for_test};

fn new_builder() -> Builder {
    Builder::new()
}

/// Adds a workflow file, containing two workflows, to the mocked out warp
/// config directory and verifies that the workflows appear in the workflow menu.
pub fn test_add_workflows_to_warp_config() -> Builder {
    new_builder()
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some((10).to_string()));

            std::fs::create_dir_all(integration_testing::workflow::workflows_dir())
                .expect("Should be able to create workflows dir");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Should have no local workflows").add_named_assertion(
                "Should have no local workflows",
                |app, window_id| {
                    let workflows: ViewHandle<CategoriesView> = view_of_type(app, window_id, 0);

                    workflows.read(app, |workflows, _| {
                        // Note that this can be a synchronous assertion because unlike the next test step,
                        // we don't have concurrency with a WarpConfig watcher thread
                        assert_eq!(
                            workflows.local_workflows().count(),
                            0,
                            "There should not be any local workflows"
                        );
                    });
                    AssertionOutcome::Success
                },
            ),
        )
        .with_step(
            TestStep::new("Write a new file containing two workflows")
                .with_setup(|_utils| {
                    integration_testing::create_file_from_assets(
                        TEST_ONLY_ASSETS,
                        "test_workflow.yaml",
                        &integration_testing::workflow::workflows_dir().join("test_workflow.yaml"),
                    );
                })
                .add_named_assertion(
                    "The two added workflows should be in the view",
                    |app, window_id| {
                        let workflows: ViewHandle<CategoriesView> = view_of_type(app, window_id, 0);

                        let num_workflows =
                            workflows.read(app, |workflows, _| workflows.local_workflows().count());
                        async_assert!(
                            num_workflows == 2,
                            "Expected to find two workflows, instead found {}",
                            num_workflows
                        )
                    },
                ),
        )
}

pub fn test_launch_warp_with_theme_in_warp_config() -> Builder {
    new_builder()
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some((10).to_string()));

            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test_theme.yaml",
                &integration_testing::themes::themes_dir().join("test_theme.yaml"),
            );
        })
        .with_step(assert_theme_chooser_contains("Test Theme", 1))
}

/// Adds a theme to the mocked out warp config directory and verifies that
/// the theme appears in the theme picker.
pub fn test_add_theme_to_warp_config() -> Builder {
    new_builder()
        .with_setup(move |utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some((10).to_string()));

            std::fs::create_dir_all(integration_testing::themes::themes_dir())
                .expect("Should be able to create themes dir");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_theme_chooser_contains("Test Theme", 0))
        .with_step(TestStep::new("Write a new theme").with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test_theme.yaml",
                &integration_testing::themes::themes_dir().join("test_theme.yaml"),
            );
        }))
        .with_step(assert_theme_chooser_contains("Test Theme", 1))
        .with_step(
            TestStep::new("Write another new theme").with_setup(|_utils| {
                integration_testing::create_file_from_assets(
                    TEST_ONLY_ASSETS,
                    "test_theme.yaml",
                    &integration_testing::themes::themes_dir().join("test_theme_2.yaml"),
                );
            }),
        )
        .with_step(assert_theme_chooser_contains("Test Theme", 2))
        .with_step(
            TestStep::new("Write another new theme with a name in the .yaml").with_setup(
                |_utils| {
                    integration_testing::create_file_from_assets(
                        TEST_ONLY_ASSETS,
                        "test_theme_with_name.yaml",
                        &integration_testing::themes::themes_dir().join("test_theme_3.yaml"),
                    );
                },
            ),
        )
        .with_step(assert_theme_chooser_contains("test_theme", 1))
        .with_step(assert_theme_chooser_contains("Test Theme", 2))
}

pub fn test_palette_opens_when_theme_chooser_is_open() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_steps(
            open_command_palette_and_run_action("Open Theme Picker").add_assertion(
                |app, window_id| {
                    let views = app.views_of_type(window_id).unwrap();
                    let workspace: &ViewHandle<Workspace> = views.first().unwrap();
                    workspace.read(app, |view, _| {
                        async_assert!(view.is_theme_chooser_open(), "Theme chooser should be open")
                    })
                },
            ),
        )
        .with_step(open_command_palette())
}

/// Manually executed test that runs a long-line.sh script. Useful for debugging the performance
/// of long output commands. Worth combining with cargo-flamegraph when running.
pub fn test_with_long_line() -> Builder {
    let test_name = "long-line.sh";
    new_builder()
        .with_setup(move |utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                test_name,
                &utils.test_dir().join(test_name),
            )
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(performance_test(0, test_name, 3))
}

/// Manually executed test that runs a 24-bit-color.sh script. Useful for debugging the performance
/// of background in the grid. Worth combining with cargo-flamegraph when running.
pub fn test_with_24_bit_color() -> Builder {
    let test_name = "24-bit-color.sh";
    new_builder()
        .with_setup(move |utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                test_name,
                &utils.test_dir().join(test_name),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(performance_test(0, test_name, 1))
}

/// Manually executed benchmark to see the memory usage of the block list.
/// This workload executes `cd` and `ls` over and over again. It ends with a
/// test step that pauses for five minutes, so you have time to check the
/// memory usage. It does this by continually failing a test step.
pub fn make_1000_blocks_memory_benchmark() -> Builder {
    let mut builder = new_builder().with_step(wait_until_bootstrapped_single_pane_for_tab(0));
    for _ in 0..1000 {
        builder = builder.with_step(execute_echo(0));
    }
    builder.with_step(
        TestStep::new("always fail")
            .add_assertion(|_, _| AssertionOutcome::failure("always fail".to_string()))
            .set_timeout(Duration::from_secs(300)),
    )
}

pub fn test_completions_with_autocd() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            let (starter, version) = current_shell_starter_and_version();
            match starter.shell_type() {
                ShellType::Zsh | ShellType::Fish => true,
                // autocd was not added until Bash 4.0--the `shopt -s autocd` line below will fail
                // in all versions of Bash before 4.0.
                ShellType::Bash => {
                    version_compare::compare_to(version, "4", Cmp::Ge).unwrap_or(false)
                }
                // TODO(PLAT-751)
                ShellType::PowerShell => false,
            }
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(&dir, "setopt autocd", [ShellRcType::Zsh]);
            write_rc_files_for_test(&dir, "shopt -s autocd", [ShellRcType::Bash]);
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            // cd into the out_dir to avoid relying on anything within the filesystem.
            concat!("cd ", env!("OUT_DIR")).into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Enter 't' and hit tab")
                .with_typed_characters(&["t"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion("Assert autocd completions", move |app, window_id| {
                    let input_suggestions =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions.read(app, |view, _ctx| {
                        async_assert!(
                            view.items().iter().any(|item| item.text() == "tmp/"),
                            "tmp/ should be suggested"
                        )
                    })
                }),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
}

pub fn test_single_command() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_echo(0))
}

pub fn test_open_and_close_settings() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open settings tab")
                .with_keystrokes(&["cmdorctrl-,"])
                .add_assertion(assert_tab_count(2))
                .add_assertion(assert_tab_title(1, "Settings"))
                .add_assertion(assert_pane_title(1, 0, "Settings"))
                .add_assertion(move |app, window_id| {
                    let settings_views: Vec<ViewHandle<SettingsView>> = app
                        .views_of_type(window_id)
                        .expect("Settings view must exist");
                    assert_eq!(settings_views.len(), 1);

                    let settings_view = settings_views.first().expect("Settings view must exist");
                    settings_view.read(app, |view, _| {
                        async_assert_eq!(
                            view.current_settings_section(),
                            SettingsSection::default()
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Close the settings tab with close tab button")
                .with_hover_over_saved_position("close_tab_button:1")
                .with_click_on_saved_position("close_tab_button:1")
                .add_assertion(assert_tab_count(1))
                .add_assertion(assert_tab_title(0, "~")),
        )
}

pub fn test_open_and_close_theme_creator_modal() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_steps(
            open_command_palette_and_run_action("Open Theme Picker").add_assertion(
                |app, window_id| {
                    let views = app.views_of_type(window_id).unwrap();
                    let workspace: &ViewHandle<Workspace> = views.first().unwrap();
                    workspace.read(app, |view, _| {
                        async_assert!(view.is_theme_chooser_open(), "Theme chooser should be open")
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Click on button to open theme creator modal")
                .with_click_on_saved_position("create_theme_button")
                .add_assertion(move |app, window_id| {
                    let views: Vec<ViewHandle<Workspace>> = app.views_of_type(window_id).unwrap();
                    let workspace = views.first().unwrap();
                    workspace.read(app, |view, _| {
                        async_assert!(
                            view.is_theme_creator_modal_open(),
                            "Theme creator modal should be open"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Click on cancel button to close theme creator modal")
                .with_click_on_saved_position("theme_creator_cancel_button")
                .add_assertion(move |app, window_id| {
                    let views: Vec<ViewHandle<Workspace>> = app.views_of_type(window_id).unwrap();
                    let workspace = views.first().unwrap();
                    workspace.read(app, |view, _| {
                        async_assert!(
                            !view.is_theme_creator_modal_open(),
                            "Theme creator modal should be closed"
                        )
                    })
                }),
        )
}

pub fn test_suggestions_menu_positioning() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Make sure there are multiple items in the working directory,
        // otherwise the tab completion menu will not appear (as there is
        // nothing for the user to choose between).
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir some_dir && touch some_file".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Open suggestions")
                .with_typed_characters(&["ls", " "])
                .with_keystrokes(&["tab"])
                .add_named_assertion_with_data_from_prior_step(
                    "Assert buffer text and save suggestion position",
                    |app, window_id, step_data| {
                        let input_view = single_input_view_for_tab(app, window_id, 0);
                        let outcome = input_view.read(app, |view, ctx| {
                            view.buffer_text(ctx);
                            async_assert!(
                                view.buffer_text(ctx) == *"ls ",
                                "Input box should contain 'ls '"
                            )
                        });

                        app.update(|ctx| {
                            let presenter = ctx.presenter(window_id).expect("window should exist");
                            let suggestions_menu_x = presenter
                                .borrow()
                                .position_cache()
                                .get_position("input_suggestions:index_0")
                                .unwrap()
                                .origin_x();
                            step_data.insert("suggestions_menu_x", suggestions_menu_x);
                        });
                        outcome
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions("Open Warp Drive")
                .with_click_on_saved_position("workspace:toggle_left_panel")
                .add_assertion(assert_is_left_panel_open()),
        )
        .with_step(
            new_step_with_default_assertions("Assert that suggestions menu updated")
                .add_named_assertion_with_data_from_prior_step(
                    "Assert suggestions menu shifted to the right",
                    |app, window_id, step_data| {
                        app.update(|ctx| {
                            let presenter = ctx.presenter(window_id).expect("window should exist");
                            let suggestions_menu_x = presenter
                                .borrow()
                                .position_cache()
                                .get_position("input_suggestions:index_0")
                                .unwrap()
                                .origin_x();
                            assert!(
                                suggestions_menu_x
                                    > *step_data
                                        .get("suggestions_menu_x")
                                        .expect("data should have been set in earlier step"),
                                "Suggestions menu should have adjusted rightward"
                            );
                        });
                        AssertionOutcome::Success
                    },
                ),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_click_on_prompt_to_focus_input() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            String::new(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Click on block to unfocus the input box")
                .with_click_on_saved_position("block_index:0")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                !input.editor().is_focused(ctx),
                                "Input box should not be focused"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Click on prompt and verify input box is focused")
                .with_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("prompt_area_{}", input.id())
                })
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box should be focused"
                            )
                        })
                    })
                }),
        )
}

pub fn test_clear() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Clear viewport using ctrl-l")
                .with_keystrokes(&["ctrl-l"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();

                        // A gap should exist in the model.
                        let num_gaps = model
                            .block_list()
                            .block_heights()
                            .cursor::<TotalIndex, ()>()
                            .filter(|item| matches!(item, BlockHeightItem::Gap { .. }))
                            .count();
                        async_assert_eq!(
                            1,
                            num_gaps,
                            "Block list should have one gap but it has {}",
                            num_gaps
                        )
                    })
                }),
        )
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Select last block and verify it's selected")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
        .with_step(
            new_step_with_default_assertions("Hit up arrow to navigate up")
                .with_keystrokes(&["up"])
                .add_assertion(assert_selected_block_index_is_first_renderable()),
        )
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions(
                "Run clear again and ensure there is still only one block",
            )
            .with_keystrokes(&["ctrl-l"])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let terminal_view: &ViewHandle<TerminalView> = views.first().unwrap();
                terminal_view.read(app, |view, _ctx| {
                    let model = view.model.lock();

                    // A single gap should exist in the model.
                    let num_gaps = model
                        .block_list()
                        .block_heights()
                        .cursor::<TotalIndex, ()>()
                        .filter(|item| matches!(item, BlockHeightItem::Gap { .. }))
                        .count();
                    async_assert_eq!(
                        1,
                        num_gaps,
                        "Block list should have one gap but it has {}",
                        num_gaps
                    )
                })
            }),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "clear".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Clear viewport using `clear` command leaves one gap")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        // A gap should exist in the model.
                        let num_gaps = model
                            .block_list()
                            .block_heights()
                            .cursor::<TotalIndex, ()>()
                            .filter(|item| matches!(item, BlockHeightItem::Gap { .. }))
                            .count();
                        async_assert_eq!(
                            1,
                            num_gaps,
                            "Block list should have one gap but it has {}",
                            num_gaps
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("`clear` block should not be visible").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        // Similar logic to `is_block_visible` in terminal/view.rs
                        let model = view.model.lock();
                        let block_list = model.block_list();

                        let last_block = match block_list.last_non_hidden_block() {
                            None => {
                                return AssertionOutcome::failure(
                                    "No non-hidden block found".to_string(),
                                );
                            }
                            Some(last_block) => last_block,
                        };

                        let visible_rows = view.content_element_height_lines(ctx);
                        let top_offset = view
                            .viewport_state(model.block_list(), InputMode::PinnedToBottom, ctx)
                            .scroll_top_in_lines();

                        let mut block_heights_cursor = block_list
                            .block_heights()
                            .cursor::<BlockIndex, BlockHeightSummary>();
                        block_heights_cursor.seek(&last_block.index(), SeekBias::Right);

                        // BlockVisibilityMode::CommandAndPromptVisible
                        let command_bottom_offset = block_heights_cursor.start().height;

                        let block_is_visible = top_offset < command_bottom_offset
                            && (top_offset + visible_rows) > command_bottom_offset;

                        async_assert!(
                            !block_is_visible,
                            "Expected `clear` command block to not be visible"
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions(
                "Select last block (`clear`) and verify it's selected",
            )
            .with_keystrokes(&["cmdorctrl-up"])
            .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_waterfall_input_alt_grid() -> Builder {
    let mut builder = new_builder()
        .with_user_defaults(HashMap::from([(
            INPUT_MODE.to_owned(),
            serde_json::to_string(&InputMode::Waterfall)
                .expect("input_mode value should convert to json string"),
        )]))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
                .add_assertion(assert_input_mode(InputMode::Waterfall)),
        )
        .with_step(
            clear_blocklist_to_remove_bootstrapped_blocks()
                .add_assertion(assert_waterfall_gap_empty_background_rendered(true)),
        );

    let steps = run_alt_grid_program(
        "vim",
        0,
        0,
        TestStep::new("Close vim")
            .with_typed_characters(&[":q"])
            .with_keystrokes(&["enter"]),
        vec![TestStep::new("waterfall background should not be rendered")
            .add_assertion(assert_waterfall_gap_empty_background_rendered(false))],
    );
    builder = builder.with_steps(steps);

    // Add one more step for letting the block list get back to a normal state
    builder = builder.with_step(new_step_with_default_assertions("return to block list"));
    builder
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_waterfall_input() -> Builder {
    new_builder()
        .with_user_defaults(HashMap::from([
            (
                INPUT_MODE.to_owned(),
                serde_json::to_string(&InputMode::Waterfall)
                    .expect("input_mode value should convert to json string"),
            ),
            // Ensure the alias expansion doesn't appear (the use of "echo" could cause it to appear
            // in Powershell since `echo` is an alias for `Write-Output`).
            (
                ALIAS_EXPANSION_BANNER_SEEN_KEY.to_owned(),
                serde_json::to_string(&true).expect("bool should convert to JSON string"),
            ),
        ]))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
                .add_assertion(assert_input_mode(InputMode::Waterfall)),
        )
        .with_step(
            clear_blocklist_to_remove_bootstrapped_blocks()
                .add_assertion(assert_num_blocks_in_model(1)),
        )
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Clear viewport using ctrl-l")
                .with_keystrokes(&["ctrl-l"])
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_at_top_of_terminal())
                .add_assertion(assert_block_visible(
                    BlockPosition::LastBlock,
                    BlockVisibilityMode::TopOfBlockVisible,
                    false,
                ))
                .add_assertion(assert_bottom_of_block_approx_at(
                    BlockPosition::LastBlock,
                    LinePosition::AtScrollTop,
                )),
        )
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Select last block and verify it's selected")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(assert_selected_block_index_is_last_renderable())
                .add_assertion(assert_input_not_at_either_edge_of_terminal()),
        )
        .with_step(
            new_step_with_default_assertions("Hit up arrow to navigate up")
                .with_keystrokes(&["up"])
                .add_assertion(assert_selected_block_index_is_first_renderable())
                .add_assertion(assert_input_not_at_either_edge_of_terminal()),
        )
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions(
                "Run clear again and ensure there is still only one block",
            )
            .with_keystrokes(&["ctrl-l"])
            .add_assertion(assert_gap_exists(true))
            .add_assertion(assert_input_at_top_of_terminal())
            .add_assertion(assert_block_visible(
                BlockPosition::LastBlock,
                BlockVisibilityMode::TopOfBlockVisible,
                false,
            ))
            .add_assertion(assert_bottom_of_block_approx_at(
                BlockPosition::LastBlock,
                LinePosition::AtScrollTop,
            )),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "clear".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Clear viewport using `clear` command leaves one gap")
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_at_top_of_terminal()),
        )
        .with_step(
            new_step_with_default_assertions("`clear` block should not be visible").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        // Similar logic to `is_block_visible` in terminal/view.rs
                        let model = view.model.lock();
                        let block_list = model.block_list();

                        let last_block = match block_list.last_non_hidden_block() {
                            None => {
                                return AssertionOutcome::failure(
                                    "No non-hidden block found".to_string(),
                                );
                            }
                            Some(last_block) => last_block,
                        };

                        let visible_rows = view.content_element_height_lines(ctx);
                        let top_offset = view
                            .viewport_state(model.block_list(), InputMode::Waterfall, ctx)
                            .scroll_top_in_lines();

                        let mut block_heights_cursor = block_list
                            .block_heights()
                            .cursor::<BlockIndex, BlockHeightSummary>();
                        block_heights_cursor.seek(&last_block.index(), SeekBias::Right);

                        let command_bottom_offset = block_heights_cursor.start().height;

                        let block_is_visible = top_offset < command_bottom_offset
                            && (top_offset + visible_rows) > command_bottom_offset;

                        async_assert!(
                            !block_is_visible,
                            "Expected `clear` command block to not be visible"
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions(
                "Select last block (`clear`) and verify it's selected",
            )
            .with_keystrokes(&["cmdorctrl-up"])
            .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
}

pub fn test_waterfall_input_text_selection() -> Builder {
    new_builder()
        .with_user_defaults(HashMap::from([(
            INPUT_MODE.to_owned(),
            serde_json::to_string(&InputMode::Waterfall)
                .expect("input_mode value should convert to json string"),
        )]))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
                .add_assertion(assert_input_mode(InputMode::Waterfall)),
        )
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Clear viewport using ctrl-l")
                .with_keystrokes(&["ctrl-l"])
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_at_top_of_terminal()),
        )
        // Run three commands after the clear
        .with_step(execute_echo(0))
        .with_step(execute_echo(0))
        .with_step(execute_echo(0).add_assertion(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                async_assert!(!view.is_selecting(), "Should not be selecting",)
            })
        }))
        .with_step(
            // Drag from the top left to the bottom right.  The exact numbers are overshoots here but
            // are big enough to capture all of the blocks.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: Vector2F::new(415., 50.),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::MouseMoved {
                    position: Vector2F::new(400., 300.),
                    cmd: false,
                    shift: false,
                    is_synthetic: false,
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: Vector2F::new(400., 300.),
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false)),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_waterfall_input_scrolling() -> Builder {
    let mut builder = new_builder()
        .with_user_defaults(HashMap::from([
            (
                INPUT_MODE.to_owned(),
                serde_json::to_string(&InputMode::Waterfall)
                    .expect("input_mode value should convert to json string"),
            ),
            // Ensure the alias expansion doesn't appear (the use of "echo" could cause it to appear
            // in Powershell since `echo` is an alias for `Write-Output`).
            (
                ALIAS_EXPANSION_BANNER_SEEN_KEY.to_owned(),
                serde_json::to_string(&true).expect("bool should convert to JSON string"),
            ),
        ]))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
                .add_assertion(assert_input_mode(InputMode::Waterfall)),
        )
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Clear viewport using ctrl-l")
                .with_keystrokes(&["ctrl-l"])
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_at_top_of_terminal()),
        )
        // Fill up the blocklist with a single long block.
        .with_step(create_long_block())
        // The gap should be gone
        .with_step(execute_echo(0).add_assertion(assert_gap_exists(false)));

    fn clear_and_arrow_up_to_top(builder: Builder) -> Builder {
        builder
            .with_step(
                new_step_with_default_assertions("Clear viewport using ctrl-l")
                    .with_keystrokes(&["ctrl-l"])
                    .add_assertion(assert_gap_exists(true))
                    .add_assertion(assert_input_at_top_of_terminal()),
            )
            .with_step(
                new_step_with_default_assertions("Hit up arrow to navigate up")
                    .with_keystrokes(&["cmdorctrl-up"])
                    .add_assertion(assert_input_not_at_either_edge_of_terminal()),
            )
            // Select to the top block - should force scrolling up
            .with_step(
                new_step_with_default_assertions("Navigate to top")
                    .with_keystrokes(&["up"; 20])
                    .add_assertion(|app, window_id| {
                        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                        terminal_view.read(app, |view, _ctx| {
                            let scrolled_up = matches!(
                                view.scroll_position(),
                                ScrollPosition::FixedAtPosition { .. }
                            );
                            async_assert!(scrolled_up, "Should be scrolled up",)
                        })
                    }),
            )
    }

    builder = clear_and_arrow_up_to_top(builder);

    // Make sure executing a small command keeps the input at the bottom.
    builder = builder
        .with_step(
            new_step_with_default_assertions("Focus the input box")
                .with_keystrokes(&[cmd_or_ctrl_shift("l")]),
        )
        .with_step(execute_echo(0).add_assertion(assert_input_at_bottom_of_terminal()));

    // Arrow up should scroll the view up.
    builder = clear_and_arrow_up_to_top(builder);

    // Make sure executing a large command that clears the gap keeps the input at the bottom.
    builder = builder
        .with_step(
            new_step_with_default_assertions("Focus the input box before long command")
                .with_keystrokes(&[cmd_or_ctrl_shift("l")]),
        )
        .with_step(
            create_long_block()
                .add_assertion(assert_gap_exists(false))
                .add_assertion(assert_input_at_bottom_of_terminal()),
        );

    builder
}

pub fn test_waterfall_input_after_command_execution() -> Builder {
    new_builder()
        .with_user_defaults(HashMap::from([(
            INPUT_MODE.to_owned(),
            serde_json::to_string(&InputMode::Waterfall)
                .expect("input_mode value should convert to json string"),
        )]))
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
                .add_assertion(assert_input_mode(InputMode::Waterfall)),
        )
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        // Fill up the blocklist with a single long block.
        .with_step(create_long_block())
        // The gap should be gone
        .with_step(execute_echo(0).add_assertion(assert_gap_exists(false)))
        .with_step(
            new_step_with_default_assertions("Clear viewport using ctrl-l")
                .with_keystrokes(&["ctrl-l"])
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_at_top_of_terminal())
                .add_assertion(assert_block_visible(
                    BlockPosition::LastBlock,
                    BlockVisibilityMode::TopOfBlockVisible,
                    false,
                ))
                .add_assertion(assert_bottom_of_block_approx_at(
                    BlockPosition::LastBlock,
                    LinePosition::AtScrollTop,
                )),
        )
        // Now make sure that when we execute a command the scroll position is preserved
        .with_step(
            execute_echo(0)
                // The gap should still exist
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_not_at_either_edge_of_terminal())
                .add_assertion(assert_block_visible(
                    BlockPosition::LastBlock,
                    BlockVisibilityMode::TopOfBlockVisible,
                    true,
                ))
                .add_assertion(assert_bottom_of_block_approx_at(
                    BlockPosition::LastBlock,
                    LinePosition::AtTopOfInput,
                ))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let waterfall_gap_scrolled = matches!(
                            view.scroll_position(),
                            ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock { .. }
                        );
                        async_assert!(waterfall_gap_scrolled, "Should have gap scrolling")
                    })
                }),
        )
        // Now try a long running command and make sure that the input goes away
        .with_step(
            TestStep::new("Execute sleep")
                .with_typed_characters(&["sleep 999"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                )
                .add_assertion(assert_gap_exists(true))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let waterfall_gap_scrolled = matches!(
                            view.scroll_position(),
                            ScrollPosition::WaterfallGapFollowsBottomOfMostRecentBlock { .. }
                        );
                        async_assert!(waterfall_gap_scrolled, "Should have gap scrolling")
                    })
                })
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.input_size_at_last_frame(ctx)
                                .expect("Input should have been laid out")
                                .y(),
                            0.,
                            "Input should have zero height"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Check ctrl-c terminates the command")
                .with_keystrokes(&["ctrl-c"])
                .set_timeout(Duration::from_secs(10))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        async_assert!(
                            !model
                                .block_list()
                                .active_block()
                                .is_active_and_long_running(),
                            "Check if the command has terminated"
                        )
                    })
                })
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        async_assert!(
                            view.input_size_at_last_frame(ctx).unwrap_or_default().y() > 0.,
                            "Input should have non-zero height"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Hit up arrow to navigate up")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(assert_input_not_at_either_edge_of_terminal()),
        )
        // Select to the top block - should force scrolling up
        .with_step(new_step_with_default_assertions("Navigate to top").with_keystrokes(&["up"; 30]))
        .with_step(
            new_step_with_default_assertions("Check scroll position at top / input at bottom")
                .with_keystrokes(&["up"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let ScrollPosition::FixedAtPosition {
                            scroll_lines: ScrollLines::ScrollTop(scroll_top_in_lines),
                        } = view.scroll_position()
                        else {
                            return AssertionOutcome::failure(
                                "Should be fixed at position".to_string(),
                            );
                        };
                        async_assert_eq!(
                            Lines::zero(),
                            scroll_top_in_lines,
                            "Should be scrolled to_top"
                        )
                    })
                })
                .add_assertion(assert_input_at_bottom_of_terminal()),
        )
        // Execute a command and make sure the bottom of it is in the right place
        .with_step(
            execute_echo(0)
                .add_assertion(assert_gap_exists(true))
                .add_assertion(assert_input_at_bottom_of_terminal()),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_text_input_on_block_list() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
           clear_blocklist_to_remove_bootstrapped_blocks()
        )
        .with_step(execute_command_for_single_terminal_in_tab(0, String::new(), ExpectedExitStatus::Success, ()))
        .with_step(
            new_step_with_default_assertions("Click on block to unfocus the input box")
                .with_click_on_saved_position("block_index:0")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                !input.editor().is_focused(ctx),
                                "Input box should not be focused"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Mock keydown on 'a' and ensure input box is focused with the character a entered",
            )
            .with_keystrokes(&["a"])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let terminal_view: &ViewHandle<TerminalView> = views.first().unwrap();
                terminal_view.read(app, |view, _ctx| {
                    view.input().read(app, |input, ctx| {
                        assert!(
                            input.editor().is_focused(ctx),
                            "Input box should be focused."
                        );
                        async_assert!(
                            input.buffer_text(ctx) == *"a",
                            "Input box buffer had had {}",
                            input.buffer_text(ctx)
                        )
                    })
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Click on block to focus the terminal view")
                .with_click_on_saved_position("block_index:0")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let focused_view_id = app.focused_view_id(window_id).expect("Focused view should exist");
                    async_assert!(focused_view_id == terminal_view.id(), "Terminal should be focused")
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Regression test step to ensure control characters (e.g. escape) don't appear in input",
            )
            .with_event(Event::KeyDown {
                keystroke: Keystroke {
                    key: "escape".to_string(),
                    ..Default::default()
                },
                chars: "\u{1b}".to_string(),
                details: Default::default(),
                is_composing: false,
            })
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).expect("Should be able to retrieve view");
                let terminal_view: &ViewHandle<TerminalView> = views.first().expect("Should be a terminal view");
                terminal_view.read(app, |view, _ctx| {
                    view.input().read(app, |input, ctx| {
                        assert!(
                            input.editor().is_focused(ctx),
                            "Input box should be focused."
                        );
                        async_assert!(
                            input.buffer_text(ctx) == *"a",
                            "Input box buffer should be unchanged but had {}",
                            input.buffer_text(ctx)
                        )
                    })
                })
            }),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_text_input_on_block_list_while_composing() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            clear_blocklist_to_remove_bootstrapped_blocks()
        )
        .with_step(execute_command_for_single_terminal_in_tab(0, String::new(), ExpectedExitStatus::Success, ()))
        .with_step(
            new_step_with_default_assertions("Click on block to unfocus the input box")
                .with_click_on_saved_position("block_index:0")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                !input.editor().is_focused(ctx),
                                "Input box should not be focused after clicking on block."
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Mock keydown on 'option-e' + 'a' in composing state and ensure input box is not focused")
                .with_keystrokes_in_composing(&["alt-e", "a"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                !input.editor().is_focused(ctx),
                                "Input box should not be focused after option-e."
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Mock typedcharacters on 'á' and ensure input box is focused and its buffer is updated",
            )
            .with_typed_characters(&["á"])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let terminal_view: &ViewHandle<TerminalView> = views.first().unwrap();
                terminal_view.read(app, |view, _ctx| {
                    view.input().read(app, |input, ctx| {
                        async_assert!(
                            input.buffer_text(ctx) == *"á",
                            "Input box buffer should be updated."
                        )
                    })
                })
            }),
        )
}

pub fn test_unnecessary_resizes() -> Builder {
    struct SizeData {
        rows: usize,
        cols: usize,
    }

    let size_data = Rc::new(Mutex::new(SizeData { rows: 0, cols: 0 }));
    let size_data_store = size_data.clone();
    let size_data_lines = size_data.clone();
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Collect size information")
            .add_assertion(
                move |app, window_id| {
                    let views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).unwrap();
                    let terminal_view = views.first().unwrap();
                    terminal_view.read(app, |view, _| {
                        let model = view.model.lock();
                        let mut size_data = size_data_store.lock();

                        let alt_screen_grid = model.alt_screen().grid_handler();
                        size_data.rows = alt_screen_grid.visible_rows();
                        size_data.cols = alt_screen_grid.columns();

                        let active_block_grid =
                            model.block_list().active_block().output_grid().grid_handler();
                        assert!(
                            active_block_grid.visible_rows() == size_data.rows
                                && active_block_grid.columns() == size_data.cols,
                            "Block and alt screen grids should have the same size"
                        );
                        AssertionOutcome::Success
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Editor expanding to multiple lines doesn't cause resize")
                .with_keystrokes(&[
                    "shift-enter",
                    "shift-enter",
                ])
                .add_assertion(move |app, window_id| {
                    let views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).unwrap();
                    let terminal_view = views.first().unwrap();
                    terminal_view.read(app, |view, _| {
                        let model = view.model.lock();
                        let size_data = size_data_lines.lock();

                        let alt_screen_grid = model.alt_screen().grid_handler();
                        let active_block_grid =
                            model.block_list().active_block().output_grid().grid_handler();

                        assert!(
                            alt_screen_grid.visible_rows() == size_data.rows
                                && alt_screen_grid.columns() == size_data.cols
                                && active_block_grid.visible_rows() == size_data.rows
                                && active_block_grid.columns() == size_data.cols,
                            "Grids should not be resized when the editor expands"
                        );
                        AssertionOutcome::Success
                    })
                }),
        )
        .with_step(
            TestStep::new("Long-running command doesn't cause resize")
                .with_typed_characters(&["python3"])
                .with_keystrokes(&["enter"])
                .add_named_assertion("no pending model events", assert_no_pending_model_events())
                .add_assertion(move |app, window_id| {
                    let views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).unwrap();
                    let terminal_view = views.first().unwrap();
                    terminal_view.read(app, |view, _| {
                        let model = view.model.lock();
                        let size_data = size_data.lock();

                        let alt_screen_grid = model.alt_screen().grid_handler();
                        let active_block = model.block_list().active_block();
                        let active_block_grid = active_block.output_grid().grid_handler();

                        async_assert!(
                            active_block.is_active_and_long_running()
                                && alt_screen_grid.visible_rows() == size_data.rows
                                && alt_screen_grid.columns() == size_data.cols
                                && active_block_grid.visible_rows() == size_data.rows
                                && active_block_grid.columns() == size_data.cols,
                            "Grids should not be resized when the editor is hidden for long-running commands. is_active_and_long_running {}",
                            active_block.is_active_and_long_running()
                        )
                    })
                }),
        )
}

pub fn test_undo_redo() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Input command")
                .with_typed_characters(&["foo"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        view.buffer_text(ctx);
                        async_assert!(
                            view.buffer_text(ctx) == *"foo",
                            "Input box should have the typed characters"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Undo")
                .with_keystrokes(&["cmdorctrl-z"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(view.buffer_text(ctx) == *"", "Input box should be empty")
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Redo")
                .with_per_platform_keystroke(PerPlatformKeystroke {
                    mac: "shift-cmd-Z",
                    linux_and_windows: "shift-ctrl-Z",
                })
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            view.buffer_text(ctx) == *"foo",
                            "Input box should have the typed characters after redo"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Execute command to clear input box")
                .with_keystrokes(&["enter"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(view.buffer_text(ctx) == *"", "Input box should be empty")
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Undo should be no-op as undo stack should be reset")
                .with_keystrokes(&["cmdorctrl-z"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            view.buffer_text(ctx) == *"",
                            "Input box should remain empty because undo stack is empty"
                        )
                    })
                }),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_open_context_menu_and_execute_command() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Run ls and verify block exists")
                .with_keystrokes(&["l", "s", "enter"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        async_assert_eq!(
                            2,
                            model.block_list().blocks().len(),
                            "Block list should have two blocks but it has {}",
                            model.block_list().blocks().len()
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Hover over the recently created block")
                .with_hover_over_saved_position("block_index:0")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        assert_eq!(
                            Some(BlockIndex::zero()),
                            view.hovered_block_index(),
                            "Expected first block to be hovered over, but got block index {:?}",
                            view.hovered_block_index()
                        );
                    });
                    AssertionOutcome::Success
                }),
        )
        .with_step(
            new_step_with_default_assertions("Click on context menu button")
                .with_click_on_saved_position("context_menu_button_0")
                .add_assertion(assert_context_menu_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Select context menu action")
                .with_click_on_saved_position("Copy command")
                .add_assertion(assert_clipboard_contains_string("ls".into())),
        )
}

pub fn test_open_and_close_context_menu_with_keybinding() -> Builder {
    new_builder()
        // The ctrl-m keybinding to open a block context menu is only set on Mac. So that we can
        // test this behavior on all platforms, create a fake custom keybindings file that forces
        // this action to have a binding of `ctrl-m`.
        .with_setup(|_utils| {
            integration_testing::create_file_with_contents(
                r#""terminal:open_block_list_context_menu_via_keybinding": ctrl-m"#.as_bytes(),
                &integration_testing::keybindings::keybinding_file_path(),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls -a".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Select recently created block with cmd-up")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
        .with_step(
            new_step_with_default_assertions("Press keybinding to open context menu")
                .with_keystrokes(&["ctrl-m"])
                .add_assertion(assert_context_menu_is_open(true)),
        )
        .with_step(
            new_step_with_default_assertions("Press keybinding again to close context menu")
                .with_keystrokes(&["ctrl-m"])
                .add_assertion(|app, window_id| {
                    let views = app.views_of_type(window_id).unwrap();
                    let workspace: &ViewHandle<Workspace> = views.first().unwrap();
                    let is_overflow_menu_showing =
                        workspace.read(app, |workspace, _| workspace.is_overflow_menu_showing());
                    async_assert!(
                        !is_overflow_menu_showing,
                        "Expected overflow menu not to be showing",
                    )
                }),
        )
}

pub fn test_open_input_context_menu() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Right click to open input context menu")
                .with_typed_characters(&["pwd"])
                .with_action(move |app, _, _| {
                    let window_id =
                        app.read(|ctx| ctx.windows().active_window().expect("no active window"));
                    let terminal_view_id = single_terminal_view(app, window_id).id();

                    app.dispatch_typed_action(
                        window_id,
                        &[terminal_view_id],
                        &TerminalAction::OpenInputContextMenu {
                            position: Vector2F::new(8.5, 8.5),
                        },
                    );
                })
                .add_assertion(assert_context_menu_is_open(true)),
        )
}

pub fn test_copy_all_from_input_context_menu() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Type something into input")
                .with_typed_characters(&["all of this should be copied"]),
        )
        .with_step(open_input_context_menu())
        .with_step(
            new_step_with_default_assertions("Press Select all in context menu")
                .with_keystrokes(&["down", "enter"]),
        )
        .with_step(open_input_context_menu())
        .with_step(
            new_step_with_default_assertions("Press Copy in context menu")
                .with_keystrokes(&["down", "down", "enter"])
                .add_assertion(assert_clipboard_contains_string(
                    "all of this should be copied".into(),
                )),
        )
}

pub fn test_cut_paste_from_input_context_menu() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Type something into input")
                .with_typed_characters(&["this should be cut then pasted"])
                .add_assertion(input_contains_string(
                    0,
                    "this should be cut then pasted".to_owned(),
                )),
        )
        .with_step(
            new_step_with_default_assertions("Select all text then press Cut in context menu")
                .with_keystrokes(&[cmd_or_ctrl_shift("a")]),
        )
        .with_step(open_input_context_menu())
        .with_step(
            new_step_with_default_assertions("Cut text using context menu")
                .with_keystrokes(&["down", "enter"]) // Cut is the first menu item when text is selected
                .add_assertion(assert_clipboard_contains_string(
                    "this should be cut then pasted".into(),
                ))
                .add_assertion(input_is_empty(0)),
        )
        .with_step(open_input_context_menu())
        .with_step(
            new_step_with_default_assertions("Press Paste in context menu")
                .with_keystrokes(&["down", "enter"]) // Paste is the first menu item when input editor is empty
                .add_assertion(input_contains_string(
                    0,
                    "this should be cut then pasted".to_owned(),
                )),
        )
}

pub fn test_block_metadata_received() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("First block in blocklist has metadata")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.model
                                .lock()
                                .block_list()
                                .blocks()
                                .last()
                                .expect("block list cannot be empty")
                                .shell_host()
                                .expect("shell_host must be set")
                                .shell_type,
                            view.active_session_shell_type(ctx)
                                .expect("terminal must have an active shell"),
                            "block shell should be correct",
                        )
                    })
                }),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_scroll_to_hidden_block_and_open_context_menu_with_keybinding() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(create_long_block())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Shift the focus to block list")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let is_input_focused = view.input().as_ref(ctx).editor().as_ref(ctx).is_focused();
                        let scroll_position = view.scroll_position();
                        async_assert!(!is_input_focused && matches!(scroll_position, ScrollPosition::FollowsBottomOfMostRecentBlock), "input should not be focused and view shouldn't scroll up")
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Select first block without scrolling")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    app.update_view(&terminal_view, |view, ctx| {
                        view.integration_test_change_block_selection_to_single(BlockIndex::zero(), ctx)
                    });
                    terminal_view.read(app, |view, _ctx| {
                        async_assert_eq!(
                            view.selected_blocks_tail_index().unwrap(), BlockIndex::zero(),
                            "first block should be selected"
                        )
                    })
                })
        )
        .with_steps({
            let mut steps = open_context_menu_for_selected_block();
            let last = steps.pop().expect("steps should not be empty");
            steps.push(last.add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, _ctx| {
                    let scroll_position = view.scroll_position();
                    async_assert!(
                            matches!(scroll_position, ScrollPosition::FixedAtPosition{..}) && view.is_context_menu_open(),
                            "Expected to scroll up from keybinding press and the context menu should be open"
                        )
                })
            }));
            steps
        })
}

pub fn test_home_key_should_not_appear_in_input() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions(
                "Press home key and verify it doesn't show up in input",
            )
            .with_keystrokes(&["home"])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let input_view: &ViewHandle<Input> = views.first().unwrap();
                input_view.read(app, |view, ctx| {
                    assert_eq!(
                        view.buffer_text(ctx),
                        String::new(),
                        "Input should be empty"
                    );
                    AssertionOutcome::Success
                })
            }),
        )
}

pub fn test_change_font_size() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Press ctrl-shift-> and verify font size increases")
                .with_keystrokes(&["ctrl-shift->"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, _ctx| {
                        view.editor().read(app, |editor, ctx| {
                            let appearance = Appearance::as_ref(ctx);
                            // Since user defaults are empty to start in an integration test,
                            // we expect that increasing the font size will change
                            // the editor font size to one more than the default.
                            async_assert!(
                                ((editor.font_size(appearance) - 1.)
                                    - MonospaceFontSize::default_value())
                                .abs()
                                    < f32::EPSILON,
                                "Font size should be greater than default"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Press ctrl-shift-< and verify font size decreases")
                .with_keystrokes(&["ctrl-shift-<"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, _ctx| {
                        view.editor().read(app, |editor, ctx| {
                            let appearance = Appearance::as_ref(ctx);
                            async_assert!(
                                (editor.font_size(appearance) - MonospaceFontSize::default_value())
                                    .abs()
                                    < f32::EPSILON,
                                "Font size should be back to the original font size"
                            )
                        })
                    })
                }),
        )
}

pub fn test_removing_tabs_out_of_order() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Add a second tab with new tab button")
                .with_click_on_saved_position(NEW_TAB_BUTTON_POSITION_ID)
                .add_assertion(assert_tab_count(2)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Add a third tab with the new tab button")
                .with_click_on_saved_position(NEW_TAB_BUTTON_POSITION_ID)
                .add_assertion(assert_tab_count(3)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions("Switch to the first tab")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_assertion(|app, window_id| {
                    let views = app.views_of_type(window_id).unwrap();
                    let workspace: &ViewHandle<Workspace> = views.first().unwrap();
                    let (active_tab_id, first_tab_id) = workspace.read(app, |workspace, _| {
                        (
                            workspace.active_tab_pane_group().id(),
                            workspace.get_pane_group_view_unchecked(0).id(),
                        )
                    });
                    async_assert_eq!(
                        active_tab_id,
                        first_tab_id,
                        "Expected first tab (ID {}) to be active, but was (ID {})",
                        first_tab_id,
                        active_tab_id,
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Close the first tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_tab_count(2)),
        )
        .with_step(
            new_step_with_default_assertions("Close the new first tab (which was the 2nd tab)")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_tab_count(1)),
        )
}

pub fn test_add_and_close_session() -> Builder {
    let pid = Rc::new(std::cell::RefCell::new(0_u32));
    let pid_clone = pid.clone();
    new_builder()
        .with_user_defaults(HashMap::from([(
            "UndoCloseEnabled".to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions(
                "Add a second session using cmd-t and verify it bootstraps",
            )
            .with_keystrokes(&[cmd_or_ctrl_shift("t")])
            .set_timeout(Duration::from_secs(10))
            .add_assertion(|app, window_id| {
                assert_single_terminal_in_tab_bootstrapped(app, window_id, 1)
            })
            .add_assertion(assert_tab_count(2)),
        )
        .with_step(
            new_step_with_default_assertions("Check if session is created properly")
                .set_timeout(Duration::from_secs(10))
                .add_assertion(move |app, window_id| {
                    let views = app.views_of_type(window_id).unwrap();
                    let workspace: &ViewHandle<Workspace> = views.first().unwrap();
                    let shell_pid = workspace.read(app, |workspace, _| {
                        workspace
                            .get_pane_group_view_unchecked(1)
                            .read(app, |pane_group, ctx| {
                                pane_group
                                    .terminal_manager(0, ctx)
                                    .expect("pane at index 0 is a terminal pane")
                                    .as_ref(ctx)
                                    .as_any()
                                    .downcast_ref::<warp::terminal::local_tty::TerminalManager>()
                                    .expect("terminal pane at index 0 contains a local session")
                                    .pid()
                                    .expect("shell should be spawned")
                            })
                    });
                    *pid.borrow_mut() = shell_pid;
                    let mut system = System::new();
                    system.refresh_processes(
                        ProcessesToUpdate::Some(&[Pid::from_u32(shell_pid)]),
                        false, /* remove_dead_processes */
                    );

                    let process = system.process(Pid::from_u32(shell_pid));
                    async_assert!(
                        shell_pid != 0 && process.is_some(),
                        "The pid should be active"
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Remove second session")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            new_step_with_default_assertions("Check if session is terminated properly")
                .add_assertion(move |_, _| {
                    let mut system = System::new();
                    let shell_pid = *pid_clone.borrow();
                    system.refresh_processes(
                        ProcessesToUpdate::Some(&[Pid::from_u32(shell_pid)]),
                        false, /* remove_dead_processes */
                    );
                    let process = system.process(Pid::from_u32(shell_pid));
                    async_assert!(
                        process.is_none(),
                        "The pid {} should have been terminated",
                        shell_pid
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Add a tab with new session button")
                .with_click_on_saved_position(NEW_TAB_BUTTON_POSITION_ID)
                .add_assertion(assert_tab_count(2)),
        )
        .with_step(
            new_step_with_default_assertions("Close the first tab with close tab button")
                .with_hover_over_saved_position("close_tab_button:0")
                .with_click_on_saved_position("close_tab_button:0")
                .add_assertion(assert_tab_count(1)),
        )
}

pub fn test_open_and_close_resource_center() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Click the resource center button to show the menu")
                .with_hover_over_saved_position("resource_center_button")
                .with_click_on_saved_position("resource_center_button")
                .add_assertion(|app, window_id| {
                    let views = app.views_of_type(window_id).expect("No workspace found");
                    let workspace: &ViewHandle<Workspace> =
                        views.first().expect("No workspace in views");
                    let is_resource_center_showing =
                        workspace.read(app, |workspace, _| workspace.is_resource_center_showing());
                    async_assert!(
                        is_resource_center_showing,
                        "Expected resource center to be showing",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Click the resource center button to hide the menu")
                .with_hover_over_saved_position("resource_center_button")
                .with_click_on_saved_position("resource_center_button")
                .add_assertion(|app, window_id| {
                    let views = app.views_of_type(window_id).expect("No workspace found");
                    let workspace: &ViewHandle<Workspace> =
                        views.first().expect("No workspace in views");
                    let is_resource_center_showing =
                        workspace.read(app, |workspace, _| workspace.is_resource_center_showing());
                    async_assert!(
                        !is_resource_center_showing,
                        "Expected resource center not to be showing",
                    )
                }),
        )
}

pub fn test_add_many_sessions() -> Builder {
    let mut builder = new_builder().with_step(wait_until_bootstrapped_single_pane_for_tab(0));
    for i in 1..5 {
        let tab_idx = i;
        builder = builder
            .with_step(
                new_step_with_default_assertions(
                    format!("Add a session {i} using cmd-t and verify it bootstraps").as_str(),
                )
                .with_keystrokes(&[cmd_or_ctrl_shift("t")])
                .set_timeout(Duration::from_secs(10))
                .add_assertion(move |app, window_id| {
                    assert_single_terminal_in_tab_bootstrapped(app, window_id, tab_idx)
                })
                .add_assertion(assert_tab_count(tab_idx + 1)),
            )
            .with_step(execute_echo(tab_idx))
    }
    builder
}

pub fn test_ctrl_tab_session_switching() -> Builder {
    #[allow(unused_mut, unused_assignments)]
    let mut builder = new_builder();

    // If linux return early.  For reasons unknown and not worth the time to debug currently
    // this test fails on linux at the step where the command palette is expected to show.
    // The feature does work on linux though - there's some underlying issue with our integration
    // test here.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        return builder;
    }

    // if we are on linux, allow unreachable code
    #[allow(unreachable_code)]
    {
        builder = new_builder()
            .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
            .with_step(execute_echo(0))
            .with_step(
                toggle_setting(SettingsAction::FeaturesPageToggle(
                    FeaturesPageAction::SetCtrlTabBehavior(CtrlTabBehavior::CycleMostRecentSession),
                ))
                .add_assertion(|app, _| {
                    let ctrl_tab_behavior = KeysSettings::handle(app)
                        .read(app, |keys_settings, _| *keys_settings.ctrl_tab_behavior);
                    async_assert!(
                        matches!(ctrl_tab_behavior, CtrlTabBehavior::CycleMostRecentSession),
                        "Ctrl-Tab behavior should be set to CycleMostLeastRecentSession"
                    )
                })
                .add_assertion(save_active_window_id("first_window_id")),
            );

        for i in 1..5 {
            let tab_idx = i;
            builder = builder
                .with_step(
                    new_step_with_default_assertions(
                        format!("Add a session {i} using cmd-t and verify it bootstraps").as_str(),
                    )
                    .with_keystrokes(&[cmd_or_ctrl_shift("t")])
                    .set_timeout(Duration::from_secs(10))
                    .add_assertion(move |app, window_id| {
                        assert_single_terminal_in_tab_bootstrapped(app, window_id, tab_idx)
                    })
                    .add_assertion(assert_tab_count(tab_idx + 1)),
                )
                .with_step(execute_echo(tab_idx))
        }
        builder = builder
            .with_step(
                new_step_with_default_assertions("Switch to the most recently added tab")
                    .with_action(|app, _, data| {
                        let window_id = match data.get("first_window_id") {
                            Some(window_id) => *window_id,
                            None => {
                                panic!("Expected first_window_id to be defined");
                            }
                        };
                        app.dispatch_custom_action(CustomAction::CycleNextSession, window_id);
                    }),
            )
            .with_step(
                new_step_with_default_assertions("release ctrl key 1")
                    .with_event(Event::ModifierKeyChanged {
                        key_code: KeyCode::ControlLeft,
                        state: KeyState::Released,
                    })
                    .add_assertion(assert_focused_editor_in_tab(
                        3, /* second to last tab, which was most recently added */
                    )),
            )
            .with_step(
                new_step_with_default_assertions("Switch to the tab just switched away from")
                    .with_action(|app, _, data| {
                        let window_id = match data.get("first_window_id") {
                            Some(window_id) => *window_id,
                            None => {
                                panic!("Expected first_window_id to be defined");
                            }
                        };
                        app.dispatch_custom_action(CustomAction::CycleNextSession, window_id);
                    }),
            )
            .with_step(
                new_step_with_default_assertions("release ctrl key 2")
                    .with_event(Event::ModifierKeyChanged {
                        key_code: KeyCode::ControlLeft,
                        state: KeyState::Released,
                    })
                    .add_assertion(assert_focused_editor_in_tab(
                        4, /* last tab, which was most recently switched away from*/
                    )),
            )
            .with_step(
                new_step_with_default_assertions("Go backwards in the tab cycle").with_action(
                    |app, _, data| {
                        let window_id = match data.get("first_window_id") {
                            Some(window_id) => *window_id,
                            None => {
                                panic!("Expected first_window_id to be defined");
                            }
                        };
                        app.dispatch_custom_action(CustomAction::CyclePrevSession, window_id);
                    },
                ),
            )
            .with_step(
                new_step_with_default_assertions("release ctrl key 3")
                    .with_event(Event::ModifierKeyChanged {
                        key_code: KeyCode::ControlLeft,
                        state: KeyState::Released,
                    })
                    .add_assertion(assert_focused_editor_in_tab(
                        0, /* first tab, should wrap around */
                    )),
            );
        builder
    }
}

// This test verifies part of the behavior that we expect from 'ssh' command
pub fn test_shell_reinitializing() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Run bash")
                .with_typed_characters(&["bash --norc"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                )
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_terminal_bootstrapping(0, 0)),
        )
        .with_step(
            TestStep::new("Initialize shell")
                .add_named_assertion("Ensure input box is visible", move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let mut model = view.model.lock();
                        model.init_shell(InitShellValue {
                            session_id: 0.into(),
                            shell: "bash".to_string(),
                            user: "local:user".to_owned(),
                            hostname: "local:host".to_owned(),
                            ..Default::default()
                        });
                        let input_visible = view.is_input_box_visible(&model, ctx);

                        async_assert!(input_visible, "Input box should be visible")
                    })
                })
                .add_named_assertion("Confirm the prompt value", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let model_arc = terminal_view.read(app, |view, _| view.model.clone());
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, move |view, ctx| {
                        let model = model_arc.lock();

                        async_assert_eq!(
                            "Starting shell...".to_string(),
                            view.prompt_render_helper
                                .prompt_working_dir(&model, view.sessions(ctx)),
                            "Checking the prompt value"
                        )
                    })
                }),
        )
}

/// Verifies that ctrl-c correctly terminates long-running commands.
pub fn test_ctrl_c() -> Builder {
    new_builder()
        // TODO(CORE-2734): Unknown failure for Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Run read")
                .add_named_assertion("no pending model events", assert_no_pending_model_events())
                .with_input_string("sleep 999", Some(&["enter"]))
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Check ctrl-c terminates the command")
                .with_keystrokes(&["ctrl-c"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        async_assert!(
                            !model
                                .block_list()
                                .active_block()
                                .is_active_and_long_running(),
                            "Check if the command has terminated"
                        )
                    })
                }),
        )
}

// This is a regression test for a hang when hovering.
pub fn test_hover_over_menu() -> Builder {
    // Test every 15x15 square.
    let mut hover_every_15_pixels =
        new_step_with_default_assertions("Hover over every 15x15 square");
    for x in 0..1770 / 15 {
        for y in 0..1770 / 15 {
            hover_every_15_pixels = hover_every_15_pixels.with_event(Event::MouseMoved {
                position: Vector2F::new((x * 15) as f32, (y * 15) as f32),
                cmd: false,
                shift: false,
                is_synthetic: false,
            })
        }
    }

    new_builder()
        // TODO(REV-569): Fish flaking on linux
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            !matches!(starter.shell_type(), ShellType::Fish)
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            String::new(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(hover_every_15_pixels)
}

// This is a regression test for hanging due to a .zshrc requiring user input
// Starts zsh with a .zshrc that echoes, then issues a read call
// We send a newline which should allow bootstrapping to complete
pub fn test_zshrc_keypress() -> Builder {
    let keystrokes = vec!["\n"];

    new_builder()
        .set_should_run_test(|| {
            // Only run this one on zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Zsh)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(dir, "echo update\nread\n", [ShellRcType::Zsh]);
        })
        .with_step(
            TestStep::new("Wait until last block is considered active & long running")
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(false, 0),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Press Enter")
                .with_keystrokes(&keystrokes)
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
}

pub fn test_detect_powerlevel10k() -> Builder {
    fn check_banner_open(tab_index: usize, open: bool) -> TestStep {
        new_step_with_default_assertions("Verify incompatible configuration banner state")
            .add_assertion(move |app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, tab_index);
                terminal_view.read(app, |view, _ctx| {
                    assert_eq!(view.is_incompatible_configuration_banner_open(), open);
                });
                AssertionOutcome::Success
            })
    }

    new_builder()
        .set_should_run_test(|| {
            // Powerlevel10k only supports zsh
            let (starter, _) = current_shell_starter_and_version();
            starter.shell_type() == ShellType::Zsh
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
function _p9k_precmd () {
    echo "Pretending Powerlevel10k is installed"
}
precmd_functions+=(_p9k_precmd)
        "#,
                [ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Since honor_ps1 is not set, the banner should be closed.
        .with_step(check_banner_open(0, false))
        // Once the custom prompt is toggled on, the banner should reopen.
        .with_step(
            new_step_with_default_assertions("Enable honor_ps1").with_action(|app, _, _| {
                SessionSettings::handle(app).update(app, |session_settings, ctx| {
                    let _ = session_settings.honor_ps1.set_value(true, ctx);
                });
            }),
        )
        .with_step(check_banner_open(0, true))
        // Additionally, a new tab should show the banner from the start.
        .with_step(
            new_step_with_default_assertions("Add a new session")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(check_banner_open(1, true))
        // If the user then switches back to the Warp prompt, we should close the banner.
        .with_step(
            new_step_with_default_assertions("Disable honor_ps1").with_action(|app, _, _| {
                SessionSettings::handle(app).update(app, |session_settings, ctx| {
                    let _ = session_settings.honor_ps1.set_value(false, ctx);
                });
            }),
        )
        .with_step(check_banner_open(0, false))
}

pub fn test_exit_multiple_tabs() -> Builder {
    let (starter, _) = current_shell_starter_and_version();
    // PowerShell has an "exit" keyword. However, it will not exit immediately. If there is a
    // background job still running, it will wait for it to finish. This means in-band commands
    // will block it from exiting. To get around that, we use `[System.Environment]::Exit(0)`
    // instead.
    let exit_command = match starter.shell_type() {
        ShellType::PowerShell => "[System.Environment]::Exit(0)",
        _ => "exit",
    };

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Add a new session using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")])
                .set_timeout(Duration::from_secs(10))
                .add_assertion(|app, window_id| {
                    assert_single_terminal_in_tab_bootstrapped(app, window_id, 1)
                })
                .add_assertion(assert_tab_count(2))
                // Wait for a render so that the view isn't removed before it's shown
                .set_post_step_pause(Duration::from_millis(20)),
        )
        .with_step(
            new_step_with_default_assertions(
                "Close the session using 'exit' and verify it is closed",
            )
            .with_typed_characters(&[exit_command])
            .with_keystrokes(&["enter"])
            .set_timeout(Duration::from_secs(10))
            .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            TestStep::new("Close the remaining session using 'exit'")
                .with_typed_characters(&[exit_command])
                .with_keystrokes(&["enter"]),
        )
}

pub fn test_block_navigation() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Create two blocks.
        .with_step(execute_echo(0))
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions(
                "Hit cmd-up to select the block and verify it's selected",
            )
            .with_keystrokes(&["cmdorctrl-up"])
            .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
        .with_step(
            new_step_with_default_assertions("Hit up arrow to navigate up.")
                .with_keystrokes(&["up"])
                .add_assertion(assert_selected_block_index_is_first_renderable()),
        )
        .with_step(
            new_step_with_default_assertions(
                "Hit up arrow to navigate up. It should not have moved.",
            )
            .with_keystrokes(&["up"])
            .add_assertion(assert_selected_block_index_is_first_renderable()),
        )
        .with_step(
            new_step_with_default_assertions("Hit down arrow to navigate down.")
                .with_keystrokes(&["down"])
                .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
        .with_step(
            new_step_with_default_assertions(
                "Hit down arrow to navigate down. It should not have moved",
            )
            .with_keystrokes(&["down"])
            .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
}

pub fn test_long_running_block_height_updated() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Run python3 and verify long-running mode is activated")
                .with_typed_characters(&["python3"])
                .with_keystrokes(&["enter"])
                .add_assertion(|app, window_id| {
                    let views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).unwrap();
                    let terminal_view = views.first().unwrap();
                    terminal_view.read(app, |view, _| {
                        let model = view.model.lock();
                        async_assert!(
                            !model.block_list().is_empty(),
                            "The executing block should appear"
                        )
                    })
                })
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(false, 0),
                ),
        )
}

pub fn test_find_within_block() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo foo".to_string(),
            ExpectedExitStatus::Success,
            "foo",
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo bar".to_string(),
            ExpectedExitStatus::Success,
            "bar",
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo baz".to_string(),
            ExpectedExitStatus::Success,
            "baz",
        ))
        .with_step(
            new_step_with_default_assertions("Open Find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        is_find_bar_open && is_find_bar_focused,
                        "Expect the find bar to be open and focused",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Type into the find box")
                .with_typed_characters(&["e", "c"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let num_matches = terminal_view.read(app, |view, ctx| {
                        let find_model = view.find_model().as_ref(ctx);
                        find_model.visible_block_list_match_count()
                    });
                    async_assert_eq!(
                        num_matches,
                        3,
                        "Expected three matches but got {:?}",
                        num_matches
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Click the find in block button")
                .with_click_on_saved_position("find_in_block_button")
                .add_assertion(|app, window_id| {
                    let views: Vec<ViewHandle<Find<TerminalFindModel>>> =
                        app.views_of_type(window_id).expect("find bar should exist");
                    let find_bar = views.first().unwrap();
                    let find_in_block_enabled =
                        find_bar.read(app, |view, _ctx| view.display_find_within_block);
                    async_assert!(
                        find_in_block_enabled == FindWithinBlockState::Enabled,
                        "Expect find in block to be turned on"
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Check number of matches after enabling find in block",
            )
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                let num_matches = terminal_view.read(app, |view, ctx| {
                    let find_model = view.find_model().as_ref(ctx);
                    find_model.visible_block_list_match_count()
                });
                async_assert_eq!(
                    num_matches,
                    1,
                    "Expected one match but got {:?}",
                    num_matches
                )
            }),
        )
        .with_step(
            new_step_with_default_assertions("Expand selection up")
                .with_keystrokes(&["shift-up"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let num_matches = terminal_view.read(app, |view, ctx| {
                        let find_model = view.find_model().as_ref(ctx);
                        find_model.visible_block_list_match_count()
                    });
                    async_assert_eq!(
                        num_matches,
                        2,
                        "Expected two matches but got {:?}",
                        num_matches
                    )
                }),
        )
}

pub fn test_case_sensitive_find() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo foo".to_string(),
            ExpectedExitStatus::Success,
            "foo",
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo FOO".to_string(),
            ExpectedExitStatus::Success,
            "FOO",
        ))
        .with_step(
            new_step_with_default_assertions("Open Find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        is_find_bar_open && is_find_bar_focused,
                        "Expect the find bar to be open and focused",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Type into the find box")
                .with_typed_characters(&["foo"])
                .add_assertion(|app, window_id| {
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
        .with_step(
            new_step_with_default_assertions("Click the case sensitivity button")
                .with_click_on_saved_position("case_sensitive_button")
                .add_assertion(|app, window_id| {
                    let views: Vec<ViewHandle<Find<TerminalFindModel>>> =
                        app.views_of_type(window_id).expect("find bar should exist");
                    let find_bar = views.first().unwrap();
                    let case_sensitivity_enabled =
                        find_bar.read(app, |view, _ctx| view.case_sensitivity_enabled);
                    async_assert!(
                        case_sensitivity_enabled,
                        "Expect case sensitivity to be turned on"
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Check number of matches after enabling case sensitivity",
            )
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                let num_matches = terminal_view.read(app, |view, ctx| {
                    let find_model = view.find_model().as_ref(ctx);
                    find_model.visible_block_list_match_count()
                });
                async_assert_eq!(
                    num_matches,
                    2,
                    "Expected two matches but got {:?}",
                    num_matches
                )
            }),
        )
}

/// Regression test for WAR-4240
pub fn test_find_bar_autoselects_text() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo foo".to_string(),
            ExpectedExitStatus::Success,
            "foo",
        ))
        .with_step(
            new_step_with_default_assertions("Open Find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        is_find_bar_open && is_find_bar_focused,
                        "Expect the find bar to be open and focused",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Type into the find box")
                .with_typed_characters(&["e", "c"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let num_matches = terminal_view.read(app, |view, ctx| {
                        let find_model = view.find_model().as_ref(ctx);
                        find_model.visible_block_list_match_count()
                    });
                    async_assert_eq!(
                        num_matches,
                        1,
                        "Expected one match but got {:?}",
                        num_matches
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Close the find bar")
                .with_keystrokes(&["escape"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        !is_find_bar_open && !is_find_bar_focused,
                        "Expect the find bar to be closed",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Re-open the find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));

                    terminal_view.read(app, |terminal_view, ctx| {
                        terminal_view.find_bar().as_ref(ctx).editor().read(
                            app,
                            |editor_view, ctx| {
                                let selected_text = editor_view.selected_text(ctx);
                                async_assert!(
                            is_find_bar_open && is_find_bar_focused && selected_text == "ec",
                            "Expect the find bar to be open and focused with text autoselected",
                        )
                            },
                        )
                    })
                }),
        )
}

pub fn test_execute_multiple_cursor_command() -> Builder {
    // Check that terminal does not crash upon input after multi-cursor command is run
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Set up multiple cursor command")
                .with_typed_characters(&["a", " ", "a"])
                .with_keystrokes(&[
                    ADD_NEXT_OCCURRENCE_KEYBINDING,
                    ADD_NEXT_OCCURRENCE_KEYBINDING,
                ])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.editor().as_ref(ctx).num_selections(ctx),
                            2,
                            "Check that there are 2 cursors"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Run multiple cursor command and type first character",
            )
            .with_keystrokes(&["enter"])
            .with_typed_characters(&["a"]),
        )
}

pub fn test_disabling_action_dispatching() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_command_palette())
        .with_steps(
            open_command_palette_and_run_action("Open Keybindings Editor").add_assertion(
                move |app, window_id| {
                    let settings_views: Vec<ViewHandle<SettingsView>> = app
                        .views_of_type(window_id)
                        .expect("Settings view must exist");
                    assert_eq!(settings_views.len(), 1);

                    let settings_view = settings_views.first().expect("Settings view must exist");
                    settings_view.read(app, |view, _| {
                        async_assert_eq!(
                            view.current_settings_section(),
                            SettingsSection::Keybindings
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Click the first element in the list")
                .with_click_on_saved_position("first_keybinding_setting")
                .set_timeout(Duration::from_secs(5))
                .add_assertion(move |app, window_id| {
                    async_assert!(
                        !app.key_bindings_dispatching_enabled(window_id),
                        "Action dispatching is disabled"
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Don't dispatch command palette action")
                .with_keystrokes(&["cmd-p"])
                .set_timeout(Duration::from_secs(5))
                .add_assertion(move |app, window_id| {
                    let views: Vec<ViewHandle<KeybindingsView>> =
                        app.views_of_type(window_id).unwrap();
                    let keybindings_view = views.first().unwrap();
                    keybindings_view.read(app, |view, _| {
                        async_assert_eq!(
                            &Keystroke::parse("cmd-p").unwrap(),
                            view.rows
                                .as_ref()
                                .unwrap()
                                .first()
                                .unwrap()
                                .binding
                                .trigger
                                .as_ref()
                                .unwrap(),
                            "Keystroke for the first binding set"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Click the cancel button in the first keybinding editor",
            )
            .with_click_on_saved_position("first_keybinding_cancel")
            .set_timeout(Duration::from_secs(5))
            .add_assertion(move |app, window_id| {
                async_assert!(
                    app.key_bindings_dispatching_enabled(window_id),
                    "Action dispatching is reenabled"
                )
            }),
        )
}

fn create_long_block() -> TestStep {
    let long_echo = long_block_command();

    execute_command_for_single_terminal_in_tab(0, long_echo, ExpectedExitStatus::Success, ())
}

/// Returns a command that would produce a long block when executed.
fn long_block_command() -> String {
    let lots_of_lines = (1..100).fold(String::new(), |mut last, _i| {
        last.push_str("a\\n");
        last
    });
    let long_echo = format!("printf \"{lots_of_lines}\"");
    long_echo
}

pub fn test_block_based_snackbar_scroll_to_top() -> Builder {
    // Test that clicking on the block based header scrolls to the top of the block
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(create_long_block())
        .with_step(
            new_step_with_default_assertions("Assert scrolled down").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let scroll_position = view.scroll_position();
                        async_assert!(
                            matches!(
                                scroll_position,
                                ScrollPosition::FollowsBottomOfMostRecentBlock
                            ),
                            "Expected to be scrolled to bottom"
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Click on block header")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let scroll_position = view.scroll_position();
                        async_assert!(
                            matches!(scroll_position, ScrollPosition::FixedAtPosition { .. }),
                            "Expected to scroll up from header click"
                        )
                    })
                }),
        )
}

/// Ensure that the block-based-snackbar appears when a command is running when input at bottom mode
/// is enabled.
pub fn test_block_based_snackbar_appears_for_running_command_input_at_bottom() -> Builder {
    new_builder()
        .with_user_defaults(user_defaults::input_mode(InputMode::PinnedToBottom))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_long_running_command(
            0,
            format!("{} && python3", long_block_command()),
        ))
        .with_step(
            TestStep::new("Ensure scroll position is at bottom of most recent block")
                .add_assertion(assert_scroll_position(
                    0,
                    ScrollPosition::FollowsBottomOfMostRecentBlock,
                ))
                .add_assertion(assert_no_pending_model_events())
                .add_assertion(assert_snackbar_is_visible(0)),
        )
}

/// Ensure that the block-based-snackbar does not appear when a pager command (such as `less -X` or
/// `git log`) is running when input at bottom mode is enabled.
pub fn test_block_based_snackbar_not_visible_for_pager_command_input_at_bottom() -> Builder {
    new_builder()
        .with_user_defaults(user_defaults::input_mode(InputMode::PinnedToBottom))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("touch foo && echo {} >> foo", long_block_command()),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_long_running_command(0, "less -X foo".into()))
        .with_step(
            TestStep::new("Ensure scroll position is at bottom of most recent block")
                .add_assertion(assert_scroll_position(
                    0,
                    ScrollPosition::FollowsBottomOfMostRecentBlock,
                ))
                .add_assertion(assert_no_pending_model_events())
                .add_assertion(assert_snackbar_is_not_visible(0)),
        )
}

/// Ensure that the block-based-snackbar appears when a command is running when input pinned to top
/// mode is enabled.
pub fn test_block_based_snackbar_appears_for_running_command_pinned_to_top() -> Builder {
    new_builder()
        .with_user_defaults(user_defaults::input_mode(InputMode::PinnedToTop))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_long_running_command(
            0,
            format!("{} && python3", long_block_command()),
        ))
        .with_step(
            TestStep::new("Ensure scroll position is at bottom of most recent block")
                .add_assertion(assert_scroll_position(
                    0,
                    ScrollPosition::FollowsBottomOfMostRecentBlock,
                ))
                .add_assertion(assert_no_pending_model_events())
                .add_assertion(assert_snackbar_is_visible(0)),
        )
}

/// Ensure that the block-based-snackbar does not appear when a pager command (such as `less -X` or
/// `git log`) is running when input pinned to top mode is enabled.
pub fn test_block_based_snackbar_not_visible_for_pager_command_pinned_to_top() -> Builder {
    new_builder()
        .with_user_defaults(user_defaults::input_mode(InputMode::PinnedToTop))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("touch foo && echo {} >> foo", long_block_command()),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_long_running_command(0, "less -X foo".into()))
        .with_step(
            TestStep::new("Ensure scroll position is at bottom of most recent block")
                .add_assertion(assert_scroll_position(
                    0,
                    ScrollPosition::FollowsBottomOfMostRecentBlock,
                ))
                .add_assertion(assert_no_pending_model_events())
                .add_assertion(assert_snackbar_is_not_visible(0)),
        )
}

/// Ensure that the block-based-snackbar appears when a command is running when input waterfall
/// mode is enabled.
pub fn test_block_based_snackbar_appears_for_running_command_waterfall_mode() -> Builder {
    new_builder()
        .with_user_defaults(user_defaults::input_mode(InputMode::Waterfall))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_long_running_command(
            0,
            format!("{} && python3", long_block_command()),
        ))
        .with_step(
            TestStep::new("Ensure scroll position is at bottom of most recent block")
                .add_assertion(assert_scroll_position(
                    0,
                    ScrollPosition::FollowsBottomOfMostRecentBlock,
                ))
                .add_assertion(assert_no_pending_model_events())
                .add_assertion(assert_snackbar_is_visible(0)),
        )
}

/// Ensure that the block-based-snackbar does not appear when a pager command (such as `less -X` or
/// `git log`) is running when input waterfall mode is enabled.
pub fn test_block_based_snackbar_not_visible_pager_command_waterfall_mode() -> Builder {
    new_builder()
        // TODO(CORE-2857) There is some flakiness with long-running commands exiting.
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_user_defaults(user_defaults::input_mode(InputMode::Waterfall))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("touch foo && echo {} >> foo", long_block_command()),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_long_running_command(0, "less -X foo".into()))
        .with_step(
            TestStep::new("Ensure scroll position is at bottom of most recent block")
                .add_assertion(assert_scroll_position(
                    0,
                    ScrollPosition::FollowsBottomOfMostRecentBlock,
                ))
                .add_assertion(assert_no_pending_model_events())
                .add_assertion(assert_snackbar_is_not_visible(0)),
        )
}

pub fn test_block_based_snackbar_small_window() -> Builder {
    // Test that clicking on the block based header scrolls to the top of the block
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "small_window.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(create_long_block())
        .with_step(
            new_step_with_default_assertions("Assert scrolled down").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let scroll_position = view.scroll_position();
                        async_assert!(
                            matches!(
                                scroll_position,
                                ScrollPosition::FollowsBottomOfMostRecentBlock
                            ),
                            "Expected to be scrolled to bottom"
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Click on block header")
                .with_click_on_saved_position("block_index:last")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let scroll_position = view.scroll_position();
                        async_assert!(
                            matches!(
                                scroll_position,
                                ScrollPosition::FollowsBottomOfMostRecentBlock
                            ),
                            "Expected no scroll from clicking on header"
                        )
                    })
                }),
        )
}

pub fn test_multi_block_selections() -> Builder {
    // Check that multi block selections work as expected
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_echo(0))
        .with_step(execute_echo(0))
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Expand selection up")
                .with_keystrokes(&["cmdorctrl-up", "shift-up"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let last_index = model
                            .block_list()
                            .last_non_hidden_block_by_index()
                            .expect("block should exist");
                        let second_last_index = model
                            .block_list()
                            .prev_non_hidden_block_from_index(last_index)
                            .expect("block_should_exist");
                        let pivot_index = view.selected_blocks_pivot_index();
                        let tail_index = view.selected_blocks_tail_index();
                        assert_eq!(pivot_index.unwrap(), last_index);
                        assert_eq!(tail_index.unwrap(), second_last_index);
                        AssertionOutcome::Success
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Expand selection back down")
                .with_keystrokes(&["shift-down"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let last_index = model
                            .block_list()
                            .last_non_hidden_block_by_index()
                            .expect("block should exist");
                        let pivot_index = view.selected_blocks_pivot_index();
                        let tail_index = view.selected_blocks_tail_index();
                        assert_eq!(pivot_index.unwrap(), last_index);
                        assert_eq!(tail_index.unwrap(), last_index);
                        AssertionOutcome::Success
                    })
                }),
        )
}

/// Bash commonly comes with .bashrc files that will no-op if PS1 is not set.
/// This test includes an alias for `l` that's set if PS1 is set, and will
/// fail if `l` is not set.
pub fn test_alias_guards_on_ps1_set() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on bash and zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(
                starter.shell_type(),
                shell::ShellType::Zsh | shell::ShellType::Bash
            )
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
# If not running interactively, don't do anything
[ -z "$PS1" ] && return
alias l='ls -CF'
"#,
                [ShellRcType::Bash, ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0, /*tab_idx*/
            "l".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
}

/// Regression test that ensures that if we don't set any PS1 value, the value
/// that comes from the binary is preserved.
/// The exit value is equivalent to empty string for older versions of bash. This
/// value comes from running the subshell and parsing the output, if the PS1 is empty.
pub fn test_ps1_value_not_null_or_exit() -> Builder {
    let (starter, _) = current_shell_starter_and_version();

    let check_exit = matches!(starter.shell_type(), ShellType::Bash);
    new_builder()
        .with_user_defaults(HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            // Check the PS1 value of the hidden block after we've finished bootstrapping.
            // This will be the block created at the `PostBootstrapPrecmd` stage.
            new_step_with_default_assertions("Check PS1 value").add_assertion(
                move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        let model = view.model.lock();
                        let block = model
                            .block_list()
                            .blocks()
                            .last()
                            .expect("After bootstrapping, we should have a block");
                        let prompt = block.prompt_to_string();
                        if check_exit {
                            async_assert!(
                                !prompt.is_empty() && !prompt.contains("exit"),
                                "Prompt should not contain the string \"exit\""
                            )
                        } else {
                            async_assert!(!prompt.is_empty(), "Prompt should not be empty")
                        }
                    })
                },
            ),
        )
}

/// Regression test that ensures we support custom prompts with bash, whether
/// it's a recent version or the older version that ships with macOS.
/// The system bash includes a hardcoded deprecation warning, which caused a
/// prompt expansion bug.
pub fn test_custom_ps1_expansion_bash() -> Builder {
    new_builder()
        .with_user_defaults(HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            starter.shell_type() == ShellType::Bash
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                // Use a prompt special character, \$, to make sure PS1 is expanded
                r#"export PS1="prompt-for-test: \$""#,
                [ShellRcType::Bash],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Check PS1 value").add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, _ctx| {
                    let model = view.model.lock();
                    let prompt = model
                        .block_list()
                        .active_block()
                        .prompt_contents_to_string(false);

                    assert_eq!(&prompt, "prompt-for-test: $", "Unexpected PS1: {prompt:?}");
                    AssertionOutcome::Success
                })
            }),
        )
}

/// Default auto title. We test that Warp's auto title is used and verify that that
/// DISABLE_AUTO_TITLE is set correctly.
pub fn test_auto_title() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Zsh)
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(tab_title_step(
            "Assert the default tab title used",
            "~".to_string(),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo $DISABLE_AUTO_TITLE".to_string(),
            ExpectedExitStatus::Success,
            util::per_shell_output(vec![(shell::ShellType::Zsh, "true")]),
        ))
}

/// Validate that disabling Warp's auto title feature will not mess with oh-my-zsh settings.
pub fn test_warp_auto_title_disabled() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on bash and zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(
                starter.shell_type(),
                shell::ShellType::Zsh | shell::ShellType::Bash
            )
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                &dir,
                r#"
WARP_DISABLE_AUTO_TITLE="1"
"#,
                [ShellRcType::Bash],
            );
            write_rc_files_for_test(
                &dir,
                r#"
WARP_DISABLE_AUTO_TITLE="true"
"#,
                [ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // If Warp title is disabled, we don't set the DISABLE_AUTO_TITLE env variable
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo $DISABLE_AUTO_TITLE".to_string(),
            ExpectedExitStatus::Success,
            util::per_shell_output(vec![
                (shell::ShellType::Zsh, ""),
                (shell::ShellType::Bash, ""),
            ]),
        ))
}

/// Checks that the tab title set by the user takes precedence over the Warp's default title and
/// doesn't require any additional setting from the user's POV. This is bash-specific test.
pub fn test_warp_honors_user_title_bash() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on bash
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Bash)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
PROMPT_COMMAND='echo -en "\033]0;TEST_TAB_TITLE\a"'
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

/// Checks that the tab title set by the user takes precedence over the Warp's default title and
/// doesn't require any additional setting from the user's POV. This is zsh-specific test.
pub fn test_warp_honors_user_title_zsh() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on bash
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Zsh)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
function set_title () {
  window_title="\033]0;TEST_TAB_TITLE\007"
  echo -ne "$window_title"
}

precmd_functions+=(set_title)
"#,
                [ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(tab_title_step(
            "Assert the user's tab title used",
            "TEST_TAB_TITLE".to_string(),
        ))
}

/// Checks that we focus the prompt after executing a command, regardless
/// of if the find bar is open or not.
pub fn test_input_focused_after_executing_command() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open Find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        is_find_bar_open && is_find_bar_focused,
                        "Expect the find bar to be open and focused",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Click input editor and verify input box is focused")
                .with_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("input_editor_{}", input.id())
                })
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box should be focused"
                            )
                        })
                    })
                }),
        )
        .with_step(execute_echo(0))
        .with_step(
            new_step_with_default_assertions("Make sure correct view is focused").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_input_focused = terminal_view.read(app, |view, ctx| {
                        view.input().as_ref(ctx).editor().as_ref(ctx).is_focused()
                    });
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        is_input_focused && !is_find_bar_focused,
                        "Expect the input editor to be focused"
                    )
                },
            ),
        )
}

/// Regression test to ensure that a new session results in the input box
/// being focused immediately.
pub fn test_new_session_focuses_input() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Input box should be focused").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let is_input_focused =
                            view.input().as_ref(ctx).editor().as_ref(ctx).is_focused();
                        assert!(is_input_focused);
                    });
                    AssertionOutcome::Success
                },
            ),
        )
}

/// TODO: make this test work for fish as well
pub fn test_executable_completions() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            let (starter, _version) = current_shell_starter_and_version();
            // TODO(CORE-2734): Unknown failure for Powershell
            !matches!(starter.shell_type(), ShellType::Fish) && skip_if_powershell_core_2303()
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            let dir_string = dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            let filepath = format!("{dir_string}/my_exec");
            let rc_contents = format!(
                r#"
                touch {filepath} && chmod a+x {filepath};
                PATH=$PATH:{dir_string}
            "#
            );
            write_rc_files_for_test(&dir, rc_contents, [ShellRcType::Bash, ShellRcType::Zsh]);
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Wait until executables are loaded")
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion("Assert executables are loaded", move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let active_session_id = view.active_block_session_id().unwrap();
                        let session = view.sessions(ctx).get(active_session_id).unwrap();
                        async_assert!(
                            session.external_commands().get().is_some(),
                            "External commands not loaded yet"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Enter 'm' and hit tab")
                .with_typed_characters(&["m"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion("Assert executable completions", move |app, window_id| {
                    let input_suggestions =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions.read(app, |view, _ctx| {
                        async_assert!(
                            view.items().iter().any(|item| item.text() == "my_exec"),
                            "my_exec should be suggested"
                        )
                    })
                }),
        )
}

// TODO: we should test this for fish someday too.
pub fn test_function_completions() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            let (starter, _version) = current_shell_starter_and_version();
            // TODO(CORE-2734): Unknown failure for Powershell
            !matches!(starter.shell_type(), ShellType::Fish) && skip_if_powershell_core_2303()
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            let content = "my_func () { true ; }";
            write_rc_files_for_test(dir, content, [ShellRcType::Bash, ShellRcType::Zsh]);
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Enter 'm' and hit tab")
                .with_typed_characters(&["m"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion("Assert function completions", move |app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        let function_name = "my_func";
                        let buffer_text = view.buffer_text(ctx);
                        let suggestion_inserted_into_buffer = buffer_text.trim() == function_name;
                        let suggestion_in_tab_menu =
                            match view.suggestions_mode_model().as_ref(ctx).mode() {
                                InputSuggestionsMode::CompletionSuggestions {
                                    completion_results,
                                    ..
                                } => completion_results
                                    .suggestions
                                    .iter()
                                    .any(|item| item.display() == function_name),
                                _ => false,
                            };

                        // If "my_func" was the only result, then it would be inserted directly, otherwise
                        // it should be in the tab completions menu.
                        async_assert!(
                            suggestion_inserted_into_buffer || suggestion_in_tab_menu,
                            "my_func is not a tab completion result"
                        )
                    })
                }),
        )
}

pub fn test_builtin_completions() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Enter 's' and hit tab")
                .with_typed_characters(&["s"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion("Assert builtin completions work", move |app, window_id| {
                    let input_suggestions =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions.read(app, |view, _ctx| {
                        async_assert!(
                            view.items().iter().any(|item| item.text() == "set"),
                            "set should be suggested"
                        )
                    })
                }),
        )
}

pub fn test_keyword_completions() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Enter 'w' and hit tab")
                .with_typed_characters(&["w"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion("Assert keyword completions work", move |app, window_id| {
                    let input_suggestions =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions.read(app, |view, _ctx| {
                        async_assert!(
                            view.items().iter().any(|item| item.text() == "while"),
                            "while should be suggested"
                        )
                    })
                }),
        )
}

pub fn test_add_windows_correct_position_and_cascade() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we have only 1 window open at start")
                .add_assertion(move |app, _| async_assert_eq!(app.window_ids().len(), 1))
                .add_assertion(save_active_window_id("first_window_id")),
        )
        // Shrink the first window so there is room on screen for the cascade
        // offset. Without this, screens that are close to the default window
        // size leave no room and cascadeTopLeftFromPoint: is a no-op.
        .with_step(
            new_step_with_default_assertions("Resize first window to leave room for cascade")
                .with_action(|app, window_id, _| {
                    app.read(|ctx| {
                        let window_manager = WindowManager::as_ref(ctx);
                        let bounds = ctx.window_bounds(&window_id).expect("window should have bounds");
                        let bounds = RectF::new(
                            bounds.origin(),
                            Vector2F::new(640., 480.),
                        );
                        window_manager.set_window_bounds(window_id, bounds);
                    });
                }),
        )
        .with_step(add_window(2)
                .add_assertion(save_active_window_id("second_window_id")))
        .with_step(new_step_with_default_assertions("Assert cascading windows")
                .add_named_assertion_with_data_from_prior_step("Check that second window is positioned with cascade",
                    move |app, window_id, data| {
                    let first_window_id = match data.get("first_window_id") {
                        Some(window_id) => *window_id,
                        None => {
                            return AssertionOutcome::failure("Expected first_window_id to be defined".into());
                        }
                    };
                    let first_window_bounds = app.window_bounds(&first_window_id).expect("first window bounds defined");
                    // Save the first window bounds for a later step
                    data.insert("first_window_bounds", first_window_bounds);
                    let second_window_bounds = app.window_bounds(&window_id).expect("second window bounds defined");
                    async_assert!(first_window_id != window_id &&
                        second_window_bounds.origin_x() > first_window_bounds.origin_x() &&
                        second_window_bounds.origin_y() > first_window_bounds.origin_y(),
                        "Second window should below and right of first window but second window was {:?} and first window was {:?}", second_window_bounds, first_window_bounds)
                    }
            ))
        .with_step(close_window("first_window_id", 1))
        .with_step(add_window_and_check_bounds(2, "first_window_bounds"))
}

pub fn test_open_new_tab_with_specific_shell_from_new_session_menu() -> Builder {
    FeatureFlag::ShellSelector.set_enabled(true);

    // Consults the AvailableShells model to find a shell based on the shell type,
    // gets the display name, and then clicks on that entry in the new session menu.
    fn new_tab_with_click_on_shell(shell: ShellType) -> TestStep {
        let shell_name = shell.name();
        new_step_with_default_assertions(format!("Open New tab with {shell_name} shell").as_str())
            .with_click_on_saved_position_fn(move |app, _| {
                AvailableShells::handle(app).read(app, |shells, _| {
                    let shell = shells.find_known_shell_by_type(shell).unwrap_or_else(|| {
                        panic!("Shell {shell_name} should be loaded into AvailableShells")
                    });
                    shells.display_name_for_shell(&shell).to_string()
                })
            })
    }

    let test_cases = vec![(ShellType::PowerShell, "(Get-Process -Id $PID).Path")];

    let mut builder = new_builder()
        .set_should_run_test(|| cfg!(windows))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0));
    let mut tab_index = 1;

    for (shell, test_command) in test_cases {
        let expected =
            regex::Regex::new(format!("{}$", shell.name()).as_str()).expect("regex should compile");
        builder = builder
            .with_step(
                new_step_with_default_assertions("Click on new tab menu button")
                    .with_click_on_saved_position(NEW_SESSION_MENU_BUTTON_POSITION_ID),
            )
            .with_step(new_tab_with_click_on_shell(shell))
            .with_step(
                wait_until_bootstrapped_single_pane_for_tab(tab_index)
                    .set_timeout(Duration::from_secs(20)),
            )
            .with_step(execute_command_for_single_terminal_in_tab(
                tab_index,
                test_command.to_string(),
                ExpectedExitStatus::Success,
                expected,
            ));
        tab_index += 1;
    }
    builder
}

pub fn test_command_xray_hover() -> Builder {
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Type in command")
                .with_typed_characters(&["git status"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);

                    input_view.read(app, |view, ctx| {
                        view.editor().read(app, |editor, _ctx| {
                            assert!(editor.get_command_x_ray().is_none())
                        });
                        async_assert!(
                            view.buffer_text(ctx) == "git status",
                            "Expect the buffer text to be correct"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Hover over status and make sure tooltip shows")
                .with_event_fn(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.update(app, |view, ctx| {
                        let mut position = ctx
                            .element_position_by_id(format!("editor:cursor_{}", view.editor().id()))
                            .expect("editor cursor should have a position")
                            .origin();
                        // Move the position slightly left and down so it's clearly over the "status" token
                        position.set_x(position.x() - 10.);
                        position.set_y(position.y() + 5.);
                        Event::MouseMoved {
                            position,
                            cmd: false,
                            shift: false,
                            is_synthetic: false,
                        }
                    })
                })
                .add_assertion(move |app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, _ctx| {
                        view.editor().read(app, |editor, _ctx| {
                            async_assert!(
                                editor.get_command_x_ray().is_some(),
                                "Command XRay state should be set"
                            )
                        })
                    })
                })
                .set_timeout(Duration::from_secs(5)),
        )
        .with_step(
            new_step_with_default_assertions("Hover past buffer text")
                // Add post step pause so that the the async assert in the next
                // step doesn't succeed right away just because we didn't give enough
                // time for the xray to trigger.
                .set_post_step_pause(Duration::from_secs(1))
                .with_event_fn(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.update(app, |view, ctx| {
                        let mut position = ctx
                            .element_position_by_id(format!("editor:cursor_{}", view.editor().id()))
                            .expect("editor cursor should have a position")
                            .origin();
                        // Move the position slightly right so it's clearly past the buffer text
                        position.set_x(position.x() + 10.);
                        Event::MouseMoved {
                            position,
                            cmd: false,
                            shift: false,
                            is_synthetic: false,
                        }
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Make sure tooltip doesn't show")
                .add_assertion(move |app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, _ctx| {
                        view.editor().read(app, |editor, _ctx| {
                            async_assert!(
                                editor.get_command_x_ray().is_none(),
                                "Command XRay state should not be set"
                            )
                        })
                    })
                })
                .set_timeout(Duration::from_secs(5)),
        )
}

/// Regression test for WAR-4951
pub fn test_command_xray_for_partial_command() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Type in partial command")
                .with_typed_characters(&["git st"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);

                    input_view.read(app, |view, ctx| {
                        view.editor().read(app, |editor, _ctx| {
                            assert!(editor.get_command_x_ray().is_none())
                        });
                        async_assert!(
                            view.buffer_text(ctx) == "git st",
                            "Expect the buffer text to be correct"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Hover over st")
                // Add post step pause so that the the async assert in the next
                // step doesn't succeed right away just because we didn't give enough
                // time for the xray to trigger.
                .set_post_step_pause(Duration::from_secs(1))
                .with_event_fn(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.update(app, |view, ctx| {
                        let mut position = ctx
                            .element_position_by_id(format!("editor:cursor_{}", view.editor().id()))
                            .expect("editor cursor should have a position")
                            .origin();
                        // Move the position slightly left and down so it's clearly over the "status" token
                        position.set_x(position.x() - 10.);
                        position.set_y(position.y() + 5.);
                        Event::MouseMoved {
                            position,
                            cmd: false,
                            shift: false,
                            is_synthetic: false,
                        }
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Make sure tooltip doesn't show")
                .add_assertion(move |app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, _ctx| {
                        view.editor().read(app, |editor, _ctx| {
                            async_assert!(
                                editor.get_command_x_ray().is_none(),
                                "Command XRay state should not be set"
                            )
                        })
                    })
                })
                .set_timeout(Duration::from_secs(5)),
        )
}

/// Regression test for WAR-4288
pub fn test_ctrl_r_multi_cursor() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Set up multiple cursors")
                .with_keystrokes(&["l", "\n", "s", "ctrl-shift-up"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.editor().as_ref(ctx).num_selections(ctx),
                            2,
                            "Check that there are 2 cursors"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Run ctrl-r and make sure it succeeds")
                .with_keystrokes(&["ctrl-r"]),
        )
}

/// This test ensures that the HISTCONTROL env var is not clobbered by our bootstrap process for bash.
/// See https://linear.app/warpdotdev/issue/WAR-2592 for more details
pub fn test_histcontrol_env_var() -> Builder {
    let histcontrol_val = "ignorespace";
    new_builder()
        .set_should_run_test(|| {
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), ShellType::Bash)
        })
        .with_setup(move |utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                format!("export HISTCONTROL={histcontrol_val}"),
                [ShellRcType::Bash],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo $HISTCONTROL".to_string(),
            ExpectedExitStatus::Success,
            histcontrol_val,
        ))
}

pub fn test_session_navigation_recency_change_tab() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Add a second session using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that second tab is most recent.")
                .add_assertion(move |app, window_id| {
                    let first_tab_session = single_terminal_pane_view_for_tab(app, window_id, 0);
                    let second_tab_session = single_terminal_pane_view_for_tab(app, window_id, 1);
                    check_recency(
                        first_tab_session,
                        second_tab_session,
                        RecentSession::Second,
                        app,
                        window_id,
                    )
                }),
        )
        .with_step(close_command_palette())
        .with_step(
            new_step_with_default_assertions("Navigate to previous tab.")
                .with_per_platform_keystroke(PerPlatformKeystroke {
                    mac: "shift-cmd-{",
                    linux_and_windows: "ctrl-pageup",
                })
                .add_assertion(move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box of original tab should be focused"
                            )
                        })
                    })
                }),
        )
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that first tab is most recent.").add_assertion(
                move |app, window_id| {
                    let first_tab_session = single_terminal_pane_view_for_tab(app, window_id, 0);
                    let second_tab_session = single_terminal_pane_view_for_tab(app, window_id, 1);
                    check_recency(
                        first_tab_session,
                        second_tab_session,
                        RecentSession::First,
                        app,
                        window_id,
                    )
                },
            ),
        )
}

pub fn test_session_navigation_recency_navigate_to_tab() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open a new tab using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(open_navigation_palette_step())
        .with_step(navigate_to_other_session_step())
        .with_step(
            new_step_with_default_assertions("Check that first tab's input box is focused.")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box of original tab should be focused"
                            )
                        })
                    })
                }),
        )
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that first tab is most recent.").add_assertion(
                move |app, window_id| {
                    let first_tab_session = single_terminal_pane_view_for_tab(app, window_id, 0);
                    let second_tab_session = single_terminal_pane_view_for_tab(app, window_id, 1);
                    check_recency(
                        first_tab_session,
                        second_tab_session,
                        RecentSession::First,
                        app,
                        window_id,
                    )
                },
            ),
        )
}

pub fn test_session_navigation_recency_click_on_window() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Opening a new window")
                .with_action(move |app, _, _| {
                    app.dispatch_global_action("root_view:open_new", ());
                })
                .add_assertion(move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box of new window should be focused"
                            )
                        })
                    })
                })
                .add_assertion(move |app, _| {
                    let window_id = app.read(|ctx| ctx.windows().active_window());
                    AssertionOutcome::SuccessWithData(StepData::new(
                        "first_window_id",
                        window_id.expect("window id present"),
                    ))
                })
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that second window session is most recent.")
                .add_named_assertion_with_data_from_prior_step(
                    "Check that second window session is most recent",
                    move |app, window_id, data| {
                        assert_eq!(app.window_ids().len(), 2, "Should have 2 windows open");
                        let Some(first_window_id) = data.get("first_window_id") else {
                            return AssertionOutcome::failure(
                                "Expected window id to be passed from prior step".to_owned(),
                            );
                        };
                        let first_window_session =
                            single_terminal_pane_view_for_tab(app, *first_window_id, 0);
                        let second_window_session =
                            single_terminal_pane_view_for_tab(app, window_id, 0);
                        check_recency(
                            first_window_session,
                            second_window_session,
                            RecentSession::Second,
                            app,
                            window_id,
                        )
                    },
                ),
        )
        .with_step(close_command_palette())
        .with_step(
            new_step_with_default_assertions("Click on the first window to focus it.")
                .set_timeout(Duration::from_secs(10))
                .with_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("prompt_area_{}", input.id())
                })
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box should be focused."
                            )
                        })
                    })
                }),
        )
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that first window session is most recent.")
                .add_named_assertion_with_data_from_prior_step(
                    "first window is most recent",
                    move |app, window_id, data| {
                        let Some(first_window_id) = data.get("first_window_id") else {
                            return AssertionOutcome::failure(
                                "Expected window id to be passed from prior step".to_owned(),
                            );
                        };
                        let first_window_session =
                            single_terminal_pane_view_for_tab(app, *first_window_id, 0);
                        let second_window_session =
                            single_terminal_pane_view_for_tab(app, window_id, 0);
                        check_recency(
                            first_window_session,
                            second_window_session,
                            RecentSession::First,
                            app,
                            window_id,
                        )
                    },
                ),
        )
}

pub fn test_session_navigation_recency_navigate_to_window() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Opening a new window")
                .with_action(move |app, _, _| {
                    app.dispatch_global_action("root_view:open_new", ());
                })
                .add_assertion(move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        view.input().read(app, |input, ctx| {
                            async_assert!(
                                input.editor().is_focused(ctx),
                                "Input box of new window should be focused"
                            )
                        })
                    })
                })
                .add_assertion(save_active_window_id("first_window_id"))
                .set_timeout(Duration::from_secs(10)),
        )
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that second window session is most recent.")
                .add_named_assertion_with_data_from_prior_step(
                    "first window is most recent",
                    move |app, window_id, data| {
                        let Some(first_window_id) = data.get("first_window_id") else {
                            return AssertionOutcome::failure(
                                "Expected window id to be passed from prior step".to_owned(),
                            );
                        };
                        let first_window_session =
                            single_terminal_pane_view_for_tab(app, *first_window_id, 0);
                        let second_window_session =
                            single_terminal_pane_view_for_tab(app, window_id, 0);
                        check_recency(
                            first_window_session,
                            second_window_session,
                            RecentSession::Second,
                            app,
                            window_id,
                        )
                    },
                ),
        )
        .with_step(
            navigate_to_other_session_step().add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, _ctx| {
                    view.input().read(app, |input, ctx| {
                        async_assert!(
                            input.editor().is_focused(ctx),
                            "Input box of original window should be focused"
                        )
                    })
                })
            }),
        )
        .with_step(open_navigation_palette_step())
        .with_step(
            new_step_with_default_assertions("Check that first window session is most recent.")
                .add_named_assertion_with_data_from_prior_step(
                    "first window is most recent",
                    move |app, window_id, data| {
                        let Some(first_window_id) = data.get("first_window_id") else {
                            return AssertionOutcome::failure(
                                "Expected window id to be passed from prior step".to_owned(),
                            );
                        };
                        let first_window_session =
                            single_terminal_pane_view_for_tab(app, *first_window_id, 0);
                        let second_window_session =
                            single_terminal_pane_view_for_tab(app, window_id, 0);
                        check_recency(
                            first_window_session,
                            second_window_session,
                            RecentSession::First,
                            app,
                            window_id,
                        )
                    },
                ),
        )
}

pub fn test_accepting_completion_inserts_space() -> Builder {
    new_builder()
        .with_setup(|utils| {
            // Change dir to cargo test dir for auto cleanup.
            let dir = utils.test_dir();
            let dir_string = dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_rc_files_for_test(
                &dir,
                format!("cd {dir_string}"),
                [ShellRcType::Bash, ShellRcType::Zsh, ShellRcType::Fish],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir test test2 && touch test/abc test/abd".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(run_completer(0, "mv test"))
        .with_step(
            new_step_with_default_assertions(
                "Hit tab to select the first item and enter to accept directory suggestion",
            )
            .with_keystrokes(&["tab", "enter"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                // Ensure we did NOT append a space at the end
                input_view.read(app, |input, ctx| {
                    let buffer_text = input.buffer_text(ctx);
                    // Ensure we did NOT append a space at the end
                    assert_eq!(buffer_text, "mv test/".to_owned());
                    assert!(matches!(
                        input.suggestions_mode_model().as_ref(ctx).mode(),
                        InputSuggestionsMode::Closed
                    ));
                });
                AssertionOutcome::Success
            }),
        )
        .with_step(
            new_step_with_default_assertions("Open tab completions menu and fill up to prefix")
                .with_typed_characters(&["a"])
                .with_keystrokes(&["tab"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |input, ctx| {
                        let buffer_text = input.buffer_text(ctx);
                        // Ensure we did NOT append a space at the end
                        assert_eq!(buffer_text, "mv test/ab".to_owned());
                        assert!(matches!(
                            input.suggestions_mode_model().as_ref(ctx).mode(),
                            InputSuggestionsMode::CompletionSuggestions { .. }
                        ));
                    });
                    AssertionOutcome::Success
                }),
        )
        .with_step(
            new_step_with_default_assertions("Accepting suggestion should add space at end")
                .with_keystrokes(&["tab", "enter"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |input, ctx| {
                        let buffer_text = input.buffer_text(ctx);
                        assert_eq!(buffer_text, "mv test/abc ".to_owned());
                        assert!(matches!(
                            input.suggestions_mode_model().as_ref(ctx).mode(),
                            InputSuggestionsMode::Closed
                        ));
                    });
                    AssertionOutcome::Success
                }),
        )
        .with_step(
            new_step_with_default_assertions("Tab right away to see completions menu again")
                .with_keystrokes(&["tab"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |input, ctx| {
                        assert!(matches!(
                            input.suggestions_mode_model().as_ref(ctx).mode(),
                            InputSuggestionsMode::CompletionSuggestions { .. }
                        ));
                    });
                    AssertionOutcome::Success
                }),
        )
        .with_step(
            new_step_with_default_assertions("Move cursor away from end of buffer and hit tab")
                .with_keystrokes(&["backspace", "left", "tab"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |input, ctx| {
                        let buffer_text = input.buffer_text(ctx);
                        assert_eq!(buffer_text, "mv test/abc".to_owned());
                        assert!(matches!(
                            input.suggestions_mode_model().as_ref(ctx).mode(),
                            InputSuggestionsMode::CompletionSuggestions { .. }
                        ));
                    });
                    AssertionOutcome::Success
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Accepting tab completion in middle of buffer doesn't append space",
            )
            .with_keystrokes(&["tab", "enter"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |input, ctx| {
                    let buffer_text = input.buffer_text(ctx);
                    assert_eq!(buffer_text, "mv test/abcc".to_owned());
                    assert!(matches!(
                        input.suggestions_mode_model().as_ref(ctx).mode(),
                        InputSuggestionsMode::Closed
                    ));
                });
                AssertionOutcome::Success
            }),
        )
}

pub fn test_create_session_with_split_pane_while_bootstrapping() -> Builder {
    // cd to the test's tmp directory so we can test pwd preservation from parent to child
    // session while the parent is still bootstrapping -- we just need to cd to any non-home
    // directory.
    let test_dir = PathBuf::from(cargo_target_tmpdir::get());
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "cd {}",
                test_dir
                    .to_str()
                    .expect("Test temp directory path should be UTF-8 compatible.")
            ),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Create two new sessions in two split panes, with the second being created before \
                the first is done bootstrapping.",
            )
            .with_keystrokes(&[cmd_or_ctrl_shift("d")])
            .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(1)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(|app, window_id| {
                    assert_single_terminal_in_tab_bootstrapped(app, window_id, 1)
                }),
        )
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(2)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(|app, window_id| {
                    assert_single_terminal_in_tab_bootstrapped(app, window_id, 2)
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Check working directories of newly created sessions.",
            )
            .set_timeout(Duration::from_secs(10))
            .add_assertion(move |app, window_id| {
                let views = app
                    .views_of_type(window_id)
                    .expect("App has no open window.");
                let workspace: &ViewHandle<Workspace> =
                    views.first().expect("Window is missing Workspace view.");
                let test_dir_ref: &PathBuf = &test_dir;
                workspace.read(app, |workspace, ctx| {
                    workspace
                        .get_pane_group_view(0)
                        .expect("Workspace has no tab view.")
                        .read(ctx, |pane_group, ctx| {
                            for pane_index in 0..=2 {
                                let actual_path = pane_group
                                    .terminal_view_at_pane_index(pane_index, ctx)
                                    .unwrap_or_else(|| {
                                        panic!(
                                            "Pane group is missing pane view at index {pane_index}"
                                        )
                                    })
                                    .read(ctx, |view, ctx| view.active_session_path_if_local(ctx));

                                if !matches!(actual_path, Some(path) if &path == test_dir_ref) {
                                    return AssertionOutcome::failure(format!(
                                        "Pane {pane_index} was not in expected path {}",
                                        test_dir_ref.display()
                                    ));
                                }
                            }
                            AssertionOutcome::Success
                        })
                })
            }),
        )
}

pub fn test_create_session_with_new_tab_while_bootstrapping() -> Builder {
    // cd to the test's tmp directory so we can test pwd preservation from parent to child
    // session while the parent is still bootstrapping -- we just need to cd to any non-home
    // directory.
    let test_dir = PathBuf::from(cargo_target_tmpdir::get());
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "cd {}",
                test_dir
                    .as_os_str()
                    .to_str()
                    .expect("Test temp directory path should be UTF-8 compatible.")
            ),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Create two new sessions in two new tabs, with the second being created before \
                the first is done bootstrapping.",
            )
            .with_keystrokes(&[cmd_or_ctrl_shift("t")])
            .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(1)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(|app, window_id| {
                    assert_single_terminal_in_tab_bootstrapped(app, window_id, 1)
                }),
        )
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(2)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(|app, window_id| {
                    assert_single_terminal_in_tab_bootstrapped(app, window_id, 2)
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Check working directories of newly created sessions.",
            )
            .set_timeout(Duration::from_secs(10))
            .add_assertion(move |app, window_id| {
                let views = app
                    .views_of_type(window_id)
                    .expect("App has no open window.");
                let workspace: &ViewHandle<Workspace> =
                    views.first().expect("Window is missing Workspace view.");

                let test_dir_ref: &PathBuf = &test_dir;
                workspace.read(app, |workspace, ctx| {
                    for tab_view in workspace.tab_views() {
                        let tab_path = tab_view.read(ctx, |pane_group, ctx| {
                            pane_group
                                .terminal_view_at_pane_index(0, ctx)
                                .unwrap_or_else(|| panic!("Pane group is missing terminal view."))
                                .read(ctx, |view, ctx| view.active_session_path_if_local(ctx))
                        });

                        if !matches!(tab_path, Some(path) if &path == test_dir_ref) {
                            return AssertionOutcome::failure(format!(
                                "Tab was not in expected path {}",
                                test_dir_ref.display()
                            ));
                        }
                    }
                    AssertionOutcome::Success
                })
            }),
        )
}

pub fn test_cmd_enter() -> Builder {
    new_builder()
        .set_should_run_test(|| cfg!(target_os = "macos"))
        .with_setup(|utils| {
            // Change dir to cargo test dir for auto cleanup.
            let dir = utils.test_dir();
            let dir_string = dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&dir, format!("cd {dir_string}"));
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "touch test1".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "touch test2".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Cmd+enter on closed suggestions menu, no action - current input not executed",
            )
            .with_typed_characters(&["cd"])
            .with_keystrokes(&[cmd_or_ctrl_shift("enter")])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    async_assert!(!view.suggestions_mode_model().as_ref(ctx).mode().is_visible(), "Suggestions menu should be closed")
                })
            })
            .add_assertion(assert_command_executed_for_single_terminal_in_tab(0, "touch test2".to_string())),
        )
        .with_step(
            new_step_with_default_assertions(
                "Clear buffer",
            )
            .with_keystrokes(&[
                // ATODO: change this to ctrl-a
                cmd_or_ctrl_shift("a").as_str(), "backspace" // delete existing buffer text
            ])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    async_assert!(
                        view.buffer_text(ctx).is_empty(),
                        "Expected buffer text to be empty, got {}",
                        view.buffer_text(ctx)
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Enter up key and verify previous command is in buffer",
            )
            .with_keystrokes(&[
                "up", "up",    // second last item in history menu
            ])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    let history_menu_open = matches!(
                        view.suggestions_mode_model().as_ref(ctx).mode(),
                        InputSuggestionsMode::HistoryUp { .. }
                    );
                    let buffer_text_is_last_executed_command = view.buffer_text(ctx) == *"touch test1";

                    async_assert!(
                        history_menu_open && buffer_text_is_last_executed_command,
                        "Expected history menu to be open, got {}, and last executed command {} to be in buffer, instead got {}",
                        history_menu_open,
                        "touch test1",
                        view.buffer_text(ctx)
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Cmd+enter on history item, command is executed")
                .with_keystrokes(&[cmd_or_ctrl_shift("enter")])
                .add_assertion(assert_command_executed_for_single_terminal_in_tab(0, "touch test1".to_string()))
                .add_assertion(assert_active_block_received_precmd(0, 0))
        )
        .with_step(
            new_step_with_default_assertions("Enter `ls t` and hit tab")
                .with_typed_characters(&["ls t"])
                // Tab once to complete up to the matching prefix and open completions menu
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "Assert tab completions menu opens",
                    move |app, window_id| {
                        let input_view = single_input_view_for_tab(app, window_id, 0);
                        input_view.read(app, |view, ctx| {
                            let tab_completions_menu_open = matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                InputSuggestionsMode::CompletionSuggestions { .. }
                            );
                            async_assert!(
                                tab_completions_menu_open,
                                "Expected tab completions menu to be open, got {:?}",
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                            )
                        })
                    },
                )
        )
        .with_step(
            new_step_with_default_assertions("Tab again to select the first item")
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "Assert test1 command is selected",
                    move |app, window_id| {
                        let input_suggestions = single_input_suggestions_view_for_tab(app, window_id, 0);
                        input_suggestions.read(app, |view, _ctx| {
                            async_assert!(
                                view.items().iter().any(|item| item.text() == "test1"),
                                "test1 should be selected"
                            )
                        })
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions(
                "Cmd+enter on tab completions item, command is executed",
            )
            .with_keystrokes(&[cmd_or_ctrl_shift("enter")])
            .add_assertion(assert_command_executed_for_single_terminal_in_tab(0, "ls test1".to_string())),
        )
}

pub fn test_completions_as_you_type() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir.clone(),
                // We need two entries that match the 'gitte' prefix to test tab behavior
                "alias gittest='git'\nalias gittext='git'",
                [ShellRcType::Bash, ShellRcType::Zsh, ShellRcType::Fish],
            );
            write_rc_files_for_test(dir, "New-Alias gittest git", [ShellRcType::PowerShell]);
        })
        .with_user_defaults(HashMap::from([(
            CompletionsOpenWhileTyping::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("One letter is too short to open the menu")
                .with_typed_characters(&["g"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::Closed,
                            "InputSuggestions should be closed after inserting 'g'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Two letters is long enough to open the menu")
                .with_typed_characters(&["i"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                &InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "InputSuggestions should be open after inserting 'i'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Type tte to narrow down to two results")
                .with_typed_characters(&["tte"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "Tab completions are not open after typing 'tte'"
                        )
                    })
                })
                .add_assertion(|app, window_id| {
                    let input_suggestions_view =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions_view.read(app, |view, _| {
                        let first_item = view.items().first().expect("should be Some()").text();
                        async_assert_eq!(
                            first_item,
                            "gittest",
                            "Expected first completion result to be 'gittest' but got '{first_item}'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Hitting tab should select the first item in the menu and insert result into buffer",
            )
            .with_keystrokes(&["tab"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    assert_eq!(view.buffer_text(ctx), "gittest".to_owned());
                    async_assert!(
                        matches!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::CompletionSuggestions { .. }
                        ),
                        "InputSuggestions should remain open after selecting 'gittest'"
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Hitting enter should close the menu and add a space",
            )
            .with_keystrokes(&["enter"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    assert_eq!(view.buffer_text(ctx), "gittest ".to_owned());
                    async_assert!(
                        matches!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::Closed
                        ),
                        "InputSuggestions should be closed after accepting 'gittest'"
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Space should reopen the menu")
                .with_typed_characters(&[" "])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                &InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "InputSuggestions should be open after adding space"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Get close to an exact match between suggestion and buffer text",
            )
            .with_typed_characters(&["commi"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    async_assert!(
                        matches!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::CompletionSuggestions { .. }
                        ),
                        "InputSuggestions should be typing 'commi'"
                    )
                })
            })
            .add_assertion(|app, window_id| {
                let input_suggestions_view =
                    single_input_suggestions_view_for_tab(app, window_id, 0);
                input_suggestions_view.read(app, |view, _| {
                    async_assert_eq!(
                        view.items().first().expect("should be Some()").text(),
                        "commit",
                        "First completion result is not 'commit'"
                    )
                })
            }),
        )
}

pub fn test_completions_as_you_type_one_matching_entry_tab() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir.clone(),
                "alias gittest='git'",
                [ShellRcType::Bash, ShellRcType::Zsh, ShellRcType::Fish],
            );
            write_rc_files_for_test(dir, "New-Alias gittest git", [ShellRcType::PowerShell]);
        })
        .with_user_defaults(HashMap::from([(
            CompletionsOpenWhileTyping::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("One letter is too short to open the menu")
                .with_typed_characters(&["g"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::Closed,
                            "InputSuggestions should be closed after inserting 'g'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Two letters is long enough to open the menu")
                .with_typed_characters(&["i"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                &InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "InputSuggestions should be open after inserting 'i'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Type tte to narrow down to one result")
                .with_typed_characters(&["tte"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "Tab completions are not open after typing 'tte'"
                        )
                    })
                })
                .add_assertion(|app, window_id| {
                    let input_suggestions_view =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions_view.read(app, |view, _| {
                        let first_item = view.items().first().expect("should be Some()").text();
                        async_assert_eq!(
                            first_item,
                            "gittest",
                            "Expected first completion result to be 'gittest' but got '{first_item}'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Hitting tab should complete gittest and close the menu since it's the only entry with matching prefix",
            )
            .with_keystrokes(&["tab"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    assert_eq!(view.buffer_text(ctx), "gittest".to_owned());
                    async_assert!(
                        matches!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::Closed
                        ),
                        "InputSuggestions should be closed after completing 'gittest'"
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Space should reopen the menu")
                .with_typed_characters(&[" "])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                &InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "InputSuggestions should be open after adding space"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Get close to an exact match between suggestion and buffer text",
            )
            .with_typed_characters(&["commi"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    async_assert!(
                        matches!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::CompletionSuggestions { .. }
                        ),
                        "InputSuggestions should be typing 'commi'"
                    )
                })
            })
            .add_assertion(|app, window_id| {
                let input_suggestions_view =
                    single_input_suggestions_view_for_tab(app, window_id, 0);
                input_suggestions_view.read(app, |view, _| {
                    async_assert_eq!(
                        view.items().first().expect("should be Some()").text(),
                        "commit",
                        "First completion result is not 'commit'"
                    )
                })
            }),
        )
}

pub fn test_completions_as_you_type_execute_on_enter() -> Builder {
    new_builder()
        .with_setup(|utils| {
            // Change dir to cargo test dir for auto cleanup.
            let dir = utils.test_dir();
            let dir_string = dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            write_all_rc_files_for_test(&dir, format!("cd {dir_string}"));
        })
        .with_user_defaults(HashMap::from([(
            CompletionsOpenWhileTyping::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir app apples && touch test_file test_file_2".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Setup exact match between suggestion and buffer text with multiple suggestions",
            )
            .with_typed_characters(&["cat test_file"])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view_for_tab(app, window_id, 0);
                input_view.read(app, |view, ctx| {
                    async_assert!(
                        matches!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::CompletionSuggestions { .. }
                        ),
                        "Tab completions are not open after typing 'cat test_file'"
                    )
                })
            })
            .add_assertion(|app, window_id| {
                let input_suggestions_view =
                    single_input_suggestions_view_for_tab(app, window_id, 0);
                input_suggestions_view.read(app, |view, _| {
                    async_assert_eq!(
                        view.items().first().expect("should be Some()").text(),
                        "test_file",
                        "First completion result is not test_file"
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Enter should execute on exact match when there are multiple suggestions",
            )
            .with_keystrokes(&["enter"])
            .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                0,
                "cat test_file".to_owned(),
            ))
            // need the active block to have received precmd before we can
            // get tab completions because we need a session id
            .add_assertion(assert_active_block_received_precmd(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions("Setup buffer to have dir with missing slash")
                .with_typed_characters(&["cd app"])
                .add_assertion(|app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        println!("buffer text is {}", view.buffer_text(ctx));
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                &InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "completions are not open after typing 'cd app'"
                        )
                    })
                })
                .add_assertion(|app, window_id| {
                    let input_suggestions_view =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions_view.read(app, |view, _| {
                        async_assert_eq!(
                            view.items().first().expect("should be Some()").text(),
                            "app/",
                            "First completion result is not 'app/'"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Enter should execute if suggestion only differs by buffer text by a slash",
            )
            .with_keystrokes(&["enter"])
            .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                0,
                "cd app".to_owned(),
            )),
        )
}

pub fn test_alias_expansion_has_limit() -> Builder {
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"
                alias a=b;
                alias b=c;
                alias c=d;
                alias d=e;
                alias e=f;
                alias f=git
                "#,
                [ShellRcType::Bash, ShellRcType::Zsh, ShellRcType::Fish],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Make sure there are multiple items in the working directory,
        // otherwise the tab completion menu will not appear (as there is
        // nothing for the user to choose between).
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir some_dir && touch some_file".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(run_completer(0, "a "))
        .with_step(
            new_step_with_default_assertions("ensure the suggestions are not for git")
                .add_assertion(|app, window_id| {
                    let input_suggestions_view =
                        single_input_suggestions_view_for_tab(app, window_id, 0);
                    input_suggestions_view.read(app, |view, _| {
                        async_assert!(
                            !view.items().iter().any(|item| item.text() == "status"),
                            "max alias expansion violated"
                        )
                    })
                }),
        )
}

pub fn test_command_corrections() -> Builder {
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir -p foo/bar".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cd fo/br".to_owned(),
            ExpectedExitStatus::Failure,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Ensure the autosuggestion is present").add_assertion(
                |app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |input_view, ctx| {
                        let editor_view = input_view.editor().as_ref(ctx);
                        async_assert_eq!(
                            editor_view.current_autosuggestion_text(),
                            Some("cd foo/bar"),
                            "autosuggestion for command correction was not populated"
                        )
                    })
                },
            ),
        )
}

/// Tests that we can successfully start a shell from a directory that has been
/// deleted.
pub fn test_start_shell_in_deleted_directory() -> Builder {
    let initial_dir =
        PathBuf::from(cargo_target_tmpdir::get()).join("test_start_shell_in_deleted_directory");
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell (Linux)
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // Start the test in a fresh directory within the test temporary files
        // directory, to make the test more hermetic.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("mkdir -p {}", initial_dir.display()),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("cd {}", initial_dir.display()),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir to-delete".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "cd to-delete".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Open a new tab using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        // Make sure the new tab opened in the to-delete subdirectory.
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            format!(
                r#"test "$PWD" = "{}""#,
                initial_dir.join("to-delete").as_path().display()
            ),
            ExpectedExitStatus::Success,
            (),
        ))
        // Delete our current directory (the one the session started in).
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "rmdir ../to-delete".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Open a new tab using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        // Ensure a new tab opened from a session where the current and initial
        // directories don't exist can bootstrap successfully, and uses the
        // user's home directory as the initial directory.
        .with_step(execute_command_for_single_terminal_in_tab(
            2,
            r#"test "$PWD" = "$HOME""#.to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
}

/// Tests that a new window will, by default, inherit the working directory from
/// the active session in the previous window.
pub fn test_new_window_inherits_previous_session_directory() -> Builder {
    let subdir_name = "subdir";
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("mkdir {subdir_name}"),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("cd {subdir_name}"),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions(
                "Assert working directories match expectations (round 1)",
            )
            .add_named_assertion("Check working directory for active session in each window", move |app, _| {
                app.window_ids()
                    .into_iter()
                    .fold(AssertionOutcome::Success, |prev, window_id| {
                        if !matches!(prev, AssertionOutcome::Success) {
                            return prev;
                        }

                        let views = app
                            .views_of_type(window_id)
                            .expect("Active window lacks a Workspace.");
                        let workspace: &ViewHandle<Workspace> =
                            views.first().expect("Window is missing Workspace view.");
                        workspace.read(app, |workspace, ctx| {
                            workspace
                                .get_pane_group_view(0)
                                .expect("Workspace has no tab view.")
                                .read(ctx, |pane_group, ctx| {
                                    let session_path = pane_group.active_session_path(ctx);
                                    async_assert!(
                                        matches!(session_path.as_ref(), Some(path) if path.ends_with(subdir_name)),
                                        "Active session working directory should end with {subdir_name}; actually got {session_path:?}"
                                    )
                                })
                        })
                    })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Open a new window")
            .with_action(|app, _, data| {
                let window_id = app.read(|ctx| {
                    ctx.windows()
                        .active_window()
                });
                app.dispatch_global_action("root_view:open_new", ());
                data.insert("first_window_id", window_id.expect("window id present"));
            }),
        )
        .with_step(
            TestStep::new("Wait for window to open").add_assertion(move |app, _| {
                async_assert_eq!(app.window_ids().len(), 2, "Should have two windows open")
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Assert working directories match expectations (round 2)",
            )
            .add_named_assertion_with_data_from_prior_step("Check active window is not initial window", move |app, _, data| {
                assert_eq!(app.window_ids().len(), 2, "Should have 2 windows open");
                let Some(first_window_id) = data.get("first_window_id") else {
                    return AssertionOutcome::failure("Expected window id to be passed from prior step".to_owned());
                };
                let active_window_id = app.read(|ctx| {
                    ctx.windows()
                        .active_window()
                });
                assert!(active_window_id.is_some());
                assert_ne!(Some(*first_window_id), active_window_id);
                AssertionOutcome::Success
            })
            .add_named_assertion("Check working directory for session in active window", move |app, _| {
                let active_window_id = app.read(|ctx| {
                    ctx.windows()
                        .active_window()
                        .expect("no active window")
                });
                let views = app
                    .views_of_type(active_window_id)
                    .expect("Active window lacks a Workspace.");
                let workspace: &ViewHandle<Workspace> =
                    views.first().expect("Window is missing Workspace view.");
                workspace.read(app, |workspace, ctx| {
                    workspace
                        .get_pane_group_view(0)
                        .expect("Workspace has no tab view.")
                        .read(ctx, |pane_group, ctx| {
                            let session_path = pane_group.active_session_path(ctx);
                            async_assert!(
                                matches!(session_path.as_ref(), Some(path) if path.ends_with(subdir_name)),
                                "Active session working directory should end with {subdir_name}; actually got {session_path:?}"
                            )
                        })
                })
            }),
        )
}

/// Checks that if the user configured a valid shell, we use it, and if not,
/// we fall back to the default system shell.
pub fn test_preferred_shell() -> Builder {
    let (starter, _) = current_shell_starter_and_version();
    let temp_dir = PathBuf::from(cargo_target_tmpdir::get()).join("test_preferred_shell");
    let shell_type = starter.shell_type();
    let custom_shell = temp_dir.join(shell_type.name());

    // Env vars in powershell need to be prefixed with `env:`
    // (ex: $env:SHELL), while sh-shells can simply access
    // env variables directly (ex: $SHELL)
    let var_prefix = match shell_type {
        ShellType::PowerShell => "env:",
        _ => "",
    };
    new_builder()
        .with_user_defaults(HashMap::from([(
            StartupShellOverride::storage_key().to_owned(),
            serde_json::to_string(&custom_shell).expect("should serialize to JSON"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        // At first, the custom shell doesn't exist, so we fall back to the default shell.
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo ${var_prefix}SHELL"),
            ExpectedExitStatus::Success,
            starter.logical_shell_path().to_path_buf(),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "mkdir -p {} && ln -s ${var_prefix}SHELL {}",
                temp_dir.display(),
                custom_shell.display()
            ),
            ExpectedExitStatus::Success,
            (),
        ))
        // Now that the shell exists, a new tab should use it.
        .with_step(
            new_step_with_default_assertions("Open a new tab using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            format!("echo ${var_prefix}SHELL"),
            ExpectedExitStatus::Success,
            custom_shell.display().to_string(),
        ))
        // After the shell is removed, new sessions should revert to the default shell.
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            format!("rm -f {}", custom_shell.display()),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Open a new tab using cmd-t")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(execute_command_for_single_terminal_in_tab(
            2,
            format!("echo ${var_prefix}SHELL"),
            ExpectedExitStatus::Success,
            starter.logical_shell_path().display().to_string(),
        ))
}

/// Checks that the git branch is correct when outside of and inside of a git
/// repo.
pub fn test_git_prompt() -> Builder {
    let (starter, _) = current_shell_starter_and_version();
    // Note that we can't use the OUT_DIR for the temp directory
    // here because that would put us in the warp repo. We need to
    // be in a place in the filesystem that's not already a git repo.
    new_builder()
        // TODO(CORE-2734): Unknown failure for Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .use_tmp_filesystem_for_test_root_directory()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Git branch should be None").add_assertion(
                |app, window_id| validate_git_branch(None, 0, window_id, app),
            ),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "git init -b main; git config user.email \"test@test.com\"; git config user.name \"Git TestUser\"".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "touch file".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "git add file; git commit -am \"commit\"".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Git branch should be main").add_assertion(
                |app, window_id| {
                    validate_git_branch(Some("main".to_string()), 0, window_id, app)
                },
            ),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            if starter.shell_type() == ShellType::Fish {
                "git checkout -q (git rev-parse HEAD) && echo (git rev-parse --short HEAD)"
            } else {
                "git checkout -q \"$(git rev-parse HEAD)\" && echo $(git rev-parse --short HEAD)"
            }.into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Git branch should be commit hash").add_assertion(
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let branch_name = terminal_view.read(app, |view, _| {
                        let model = view.model.lock();
                        let block = &model.block_list().last_non_hidden_block().expect("Must have visible blocks.");
                        block.output_to_string()
                    });
                    assert_ne!(branch_name, "HEAD");
                    validate_git_branch(Some(branch_name), 0, window_id, app)
                },
            ),
        )
}

pub fn test_terminal_announces_capabilities_to_shell() -> Builder {
    let (starter, _) = current_shell_starter_and_version();
    // Env vars in powershell need to be prefixed with `env:`
    // (ex: $env:TERM), while sh-shells can simply access
    // env variables directly (ex: $TERM)
    let var_prefix = match starter.shell_type() {
        ShellType::PowerShell => "env:",
        _ => "",
    };

    // Note that we can't use the OUT_DIR for the temp directory
    // here because that would put us in the warp repo. We need to
    // be in a place in the filesystem that's not already a git repo.
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo ${var_prefix}TERM"),
            ExpectedExitStatus::Success,
            "xterm-256color",
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo ${var_prefix}TERM_PROGRAM"),
            ExpectedExitStatus::Success,
            "WarpTerminal",
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!("echo ${var_prefix}COLORTERM"),
            ExpectedExitStatus::Success,
            "truecolor",
        ))
}

pub fn test_find_query_not_evaluated_on_terminal_mode_change() -> Builder {
    new_builder()
        // TODO(CORE-2732): Flakey on Powershell
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "seq 1000".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Open Find bar")
                .with_keystrokes(&[cmd_or_ctrl_shift("f")])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    let is_find_bar_focused =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_focused(ctx));
                    async_assert!(
                        is_find_bar_open && is_find_bar_focused,
                        "Expect the find bar to be open and focused",
                    )
                }),
        )
        .with_step(
            new_step_with_default_assertions("Type into the find box")
                .with_typed_characters(&["42"]),
        )
        .with_step(
            new_step_with_default_assertions("Dismiss the find box")
                .with_keystrokes(&["escape"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let is_find_bar_open =
                        terminal_view.read(app, |view, ctx| view.is_find_bar_open(ctx));
                    async_assert!(!is_find_bar_open, "Expect the find bar to be closed",)
                }),
        )
        .with_step(
            // Don't perform default assertions here, as we expect to be in the
            // middle of an executing command.
            TestStep::new("Run git diff")
                .with_typed_characters(&["git diff"])
                .with_keystrokes(&["enter"])
                .set_post_step_pause(Duration::from_millis(100))
        )
        .with_step(
            new_step_with_default_assertions("Quit git diff")
                .with_typed_characters(&["q"])
        )
        .with_step(
            new_step_with_default_assertions("Assert scroll bar is at the bottom")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        async_assert_eq!(view.scroll_position(), ScrollPosition::FollowsBottomOfMostRecentBlock, "Expect the scroll position to be fixed to the bottom of the most recent block")
                    })
                })
        )
}

pub fn test_custom_open_completions_menu_binding() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Set custom keybinding for completions")
                .add_assertion(move |app, window_id| {
                    // Set a custom keybinding for opening the completions menu to <ctrl>-1
                    app.update(|ctx| {
                        ctx.set_custom_trigger(
                            "input:open_completion_suggestions".to_owned(),
                            Trigger::Keystrokes(vec![Keystroke {
                                ctrl: true,
                                key: "1".to_owned(),
                                ..Default::default()
                            }]),
                        );
                    });

                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert_eq!(
                            view.suggestions_mode_model().as_ref(ctx).mode(),
                            &InputSuggestionsMode::Closed,
                            "InputSuggestions should still be closed"
                        )
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Attempt to trigger completions for ls")
                .with_typed_characters(&["ls", " "])
                .with_keystrokes(&["ctrl-1"])
                .set_timeout(Duration::from_secs(30))
                .add_assertion(move |app, window_id| {
                    let input_view = single_input_view_for_tab(app, window_id, 0);
                    input_view.read(app, |view, ctx| {
                        async_assert!(
                            matches!(
                                view.suggestions_mode_model().as_ref(ctx).mode(),
                                InputSuggestionsMode::CompletionSuggestions { .. }
                            ),
                            "Tab completions are open after pressing ctrl-1"
                        )
                    })
                }),
        )
}

/// This is a regression test for:
/// https://linear.app/warpdotdev/issue/WAR-6095/panic-internal-error-entered-unreachable-code-handled-at-model-layer
pub fn test_color_overrides_in_prompt_dont_crash() -> Builder {
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on zsh
            let (starter, _) = current_shell_starter_and_version();
            matches!(starter.shell_type(), shell::ShellType::Zsh)
        })
        .with_setup(|utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                dir,
                r#"PS1=$(printf "hello> \x1b]4;1;rgb:0/0/0\x1b")"#,
                [ShellRcType::Zsh],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0, /*tab_idx*/
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
}

pub fn test_copy_prompt_from_block_honor_ps1_disabled() -> Builder {
    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            // cding into home gives us a predictable prompt
            "cd $HOME".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Select last block")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
        .with_steps(open_context_menu_for_selected_block())
        .with_step(
            new_step_with_default_assertions("Copy prompt copies to clipboard properly")
                .with_click_on_saved_position("Copy prompt")
                .add_assertion(assert_clipboard_contains_string("~".into())),
        )
}

pub fn test_copy_prompt_from_block_honor_ps1_enabled() -> Builder {
    let prompt_text = "this is my custom prompt";
    new_builder()
        // TODO(CORE-2732): Flakey on linux
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_user_defaults(HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .with_setup(move |utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                &dir,
                format!(r#"export PS1="{prompt_text}""#),
                [ShellRcType::Bash, ShellRcType::Zsh],
            );
            write_rc_files_for_test(
                &dir,
                format!(
                    r#"
function fish_prompt
  echo -n "{prompt_text}"
end
"#
                ),
                [ShellRcType::Fish],
            );
            write_rc_files_for_test(
                &dir,
                format!(
                    r#"
function prompt {{
    "{prompt_text}"
}}
"#
                ),
                [ShellRcType::PowerShell],
            )
        })
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_assertion(move |app, _window_id| {
                let input = single_input_view_for_tab(app, _window_id, 0);
                let input_text = input.read(app, |input, ctx| input.prompt_and_rprompt_text(ctx).0);

                async_assert_eq!(input_text, prompt_text)
            }),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Select last block")
                .with_keystrokes(&["cmdorctrl-up"])
                .add_assertion(assert_selected_block_index_is_last_renderable()),
        )
        .with_steps(open_context_menu_for_selected_block())
        .with_step(
            new_step_with_default_assertions("Copy prompt copies to clipboard properly")
                .with_right_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("prompt_area_{}", input.id())
                })
                .with_click_on_saved_position("Copy prompt")
                .add_assertion(assert_clipboard_contains_string(String::from(prompt_text))),
        )
}

pub fn test_copy_prompt_from_input_honor_ps1_disabled() -> Builder {
    new_builder()
        .use_tmp_filesystem_for_test_root_directory()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            // cding into home gives us a predictable prompt
            "cd $HOME".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Copy prompt copies to clipboard properly")
                .with_right_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("prompt_area_{}", input.id())
                })
                .with_click_on_saved_position("Copy prompt")
                .add_assertion(assert_clipboard_contains_string("~".into())),
        )
}

pub fn test_copy_prompt_from_input_honor_ps1_enabled() -> Builder {
    let prompt_text = "this is my custom prompt";
    new_builder()
        .with_user_defaults(HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .with_setup(move |utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                &dir,
                format!(r#"export PS1="{prompt_text}""#),
                [ShellRcType::Bash, ShellRcType::Zsh],
            );
            write_rc_files_for_test(
                &dir,
                format!(
                    r#"
function fish_prompt
  echo -n "{prompt_text}"
end
"#
                ),
                [ShellRcType::Fish],
            );

            write_rc_files_for_test(
                &dir,
                format!(
                    r#"
function prompt {{
    "{prompt_text}"
}}
"#
                ),
                [ShellRcType::PowerShell],
            )
        })
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_assertion(move |app, _window_id| {
                let input = single_input_view_for_tab(app, _window_id, 0);
                let input_text = input.read(app, |input, ctx| input.prompt_and_rprompt_text(ctx).0);

                async_assert_eq!(input_text, prompt_text)
            }),
        )
        .with_step(
            new_step_with_default_assertions("Copy prompt copies to clipboard properly")
                .with_right_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("prompt_area_{}", input.id())
                })
                .with_click_on_saved_position("Copy prompt")
                .add_assertion(assert_clipboard_contains_string(String::from(prompt_text))),
        )
}

pub fn test_copy_rprompt_from_input_honor_ps1_enabled() -> Builder {
    let rprompt_text = "right prompt";
    new_builder()
        .set_should_run_test(|| {
            // Only run this one on zsh and fish
            let (starter, _) = current_shell_starter_and_version();
            matches!(
                starter.shell_type(),
                shell::ShellType::Zsh | shell::ShellType::Fish
            )
        })
        .with_setup(move |utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                &dir,
                format!(r#"export RPROMPT="{rprompt_text}""#),
                [ShellRcType::Zsh],
            );
            write_rc_files_for_test(
                &dir,
                format!(
                    r#"
function fish_prompt
    echo -n "left prompt"
end
function fish_right_prompt
  echo -n "{rprompt_text}"
end
"#
                ),
                [ShellRcType::Fish],
            );
        })
        .with_user_defaults(HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Ensure toggle_ps1 is on, rprompt shown in input")
                .add_assertion(move |app, _window_id| {
                    app.read(|ctx| async_assert!(*SessionSettings::as_ref(ctx).honor_ps1.value()))
                })
                .add_assertion(move |app, _window_id| {
                    let input = single_input_view_for_tab(app, _window_id, 0);
                    let input_text = input.read(app, |input, ctx| {
                        input
                            .prompt_and_rprompt_text(ctx)
                            .1
                            .expect("rprompt should exist!")
                    });
                    async_assert_eq!(input_text, rprompt_text)
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy rprompt copies to clipboard properly")
                .with_right_click_on_saved_position_fn(|app, window_id| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    format!("rprompt_area_{}", input.id())
                })
                .with_keystrokes(&["down", "down", "enter"]) // Copy Right Prompt should be second option in context menu
                .add_assertion(assert_clipboard_contains_string(String::from(rprompt_text))),
        )
}

pub fn test_rprompt_doesnt_show_when_not_enough_space() -> Builder {
    let prompt_text = "this is my custom prompt which is very very very very very very very very very very very long";
    let rprompt_text = "this is my custom right prompt which is very very very very very very very very very very very long";
    new_builder()
        .with_user_defaults(HashMap::from([(
            HonorPS1::storage_key().to_owned(),
            true.to_string(),
        )]))
        .set_should_run_test(|| {
            // Only run this one on zsh and fish
            let (starter, _) = current_shell_starter_and_version();
            matches!(
                starter.shell_type(),
                shell::ShellType::Zsh | shell::ShellType::Fish
            )
        })
        .with_setup(move |utils| {
            let dir = utils.test_dir();
            write_rc_files_for_test(
                &dir,
                format!(r#"export PS1="{prompt_text}" \n export RPROMPT="{rprompt_text}""#),
                [ShellRcType::Zsh],
            );
            write_rc_files_for_test(
                &dir,
                format!(
                    r#"
function fish_prompt
    echo -n "{prompt_text}"
end
function fish_right_prompt
  echo -n "{rprompt_text}"
end
"#
                ),
                [ShellRcType::Fish],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions(
                "Switch to honor ps1 mode, prompt should exist, rprompt should not",
            )
            .add_assertion(move |app, _window_id| {
                let input = single_input_view_for_tab(app, _window_id, 0);
                let input_prompt_text =
                    input.read(app, |input, ctx| input.prompt_and_rprompt_text(ctx).0);

                async_assert_eq!(input_prompt_text, prompt_text)
            })
            .add_assertion(move |app, _window_id| {
                let input = single_input_view_for_tab(app, _window_id, 0);
                let input_rprompt_text =
                    input.read(app, |input, ctx| input.prompt_and_rprompt_text(ctx).1);

                async_assert_eq!(input_rprompt_text, None)
            }),
        )
}

/// When in REPLs like irb or ipython, we want to make sure well-known word navigation keybindings
/// get converted to the right control characters to move the cursor. This also applies to
/// subshells which are not bootstrapped.
pub fn test_block_cursor_navigation_using_escape_codes() -> Builder {
    let (starter, _) = current_shell_starter_and_version();
    // On Linux, bash will overwrite an inherited PS1 variable with its choice
    // of default value.  To work around this, we also set PROMPT_COMMAND
    // (which doesn't get clobbered) to set the PS1 variable, ensuring it has
    // the expected value after shell startup.
    let bash_command = match starter.shell_type() {
        ShellType::PowerShell => {
            r#"$env:BASH_SILENCE_DEPRECATION_WARNING=1; $env:PS1='> '; $env:PROMPT_COMMAND='export PS1="> "'; /bin/bash"#
        }
        _ => {
            r#"BASH_SILENCE_DEPRECATION_WARNING=1 PS1='> ' PROMPT_COMMAND='export PS1="> "' /bin/bash"#
        }
    };
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute a REPL, but without bootstrapping a subshell")
                // The default bash prompt shows the version number, which we don't want polluting
                // the test so we overwrite PS1. Additionally, set the
                // `BASH_SILENCE_DEPRECATION_WARNING` env var so we don't a message on MacOS telling
                // us that this version of bash is deprecated.
                .with_typed_characters(&[bash_command])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                )
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> ", 0,
                )),
        )
        .with_step(
            TestStep::new("Verify left word navigation")
                .with_typed_characters(&["echo hello world"])
                .with_keystrokes(&["alt-left", "alt-left", "backspace"])
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> echohello world",
                    0,
                )),
        )
        .with_step(
            TestStep::new("Verify right word navigation")
                .with_keystrokes(&["alt-right", "backspace"])
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> echohell world",
                    0,
                )),
        )
        .with_step(
            TestStep::new("Verify beginning of line navigation")
                .with_keystrokes(&["right", "right", "right", "home", "delete"])
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> chohell world",
                    0,
                )),
        )
        .with_step(
            TestStep::new("Verify end of line navigation")
                .with_keystrokes(&["end", "backspace"])
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    // We use a regex here to ignore trailing whitespace, which
                    // can be present due to how backspace is typically implemented
                    // (write a space over the cell).  There is differing behavior
                    // here between Linux and macOS, hence the regex.
                    regex::Regex::new("> chohell worl *").expect("regex should compile"),
                    0,
                )),
        )
}

/// Similar to above, we want to make sure deleting words and lines works in REPLs and shells
pub fn test_block_bulk_deletion_using_escape_codes() -> Builder {
    let (starter, _) = current_shell_starter_and_version();
    // On Linux, bash will overwrite an inherited PS1 variable with its choice
    // of default value.  To work around this, we also set PROMPT_COMMAND
    // (which doesn't get clobbered) to set the PS1 variable, ensuring it has
    // the expected value after shell startup.
    let bash_command = match starter.shell_type() {
        ShellType::PowerShell => {
            r#"$env:BASH_SILENCE_DEPRECATION_WARNING=1; $env:PS1='> '; $env:PROMPT_COMMAND='export PS1="> "'; /bin/bash"#
        }
        _ => {
            r#"BASH_SILENCE_DEPRECATION_WARNING=1 PS1='> ' PROMPT_COMMAND='export PS1="> "' /bin/bash"#
        }
    };
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Execute a REPL, but without bootstrapping a subshell")
                // The default bash prompt shows the version number, which we don't want polluting
                // the test so we overwrite PS1. Additionally, set the
                // `BASH_SILENCE_DEPRECATION_WARNING` env var so we don't a message on MacOS telling
                // us that this version of bash is deprecated.
                .with_typed_characters(&[bash_command])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 0),
                )
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> ", 0,
                )),
        )
        .with_step(
            TestStep::new("Verify delete word")
                .with_typed_characters(&["echo hello world"])
                .with_per_platform_keystroke(PerPlatformKeystroke {
                    mac: "alt-backspace",
                    linux_and_windows: "ctrl-backspace",
                })
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> echo hello ",
                    0,
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Delete to line start within an executing command")
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> ", 0,
                )),
        )
        .with_step(
            TestStep::new("Verify delete to end")
                .with_typed_characters(&["echo hello world"])
                .with_keystrokes(&["home", "right"]),
        )
        .with_steps(
            open_command_palette_and_run_action("Delete to line end within an executing command")
                .add_assertion(assert_active_block_output_for_single_terminal_in_tab(
                    "> e", 0,
                )),
        )
}

/// Tests that any keydowns that would trigger an escape sequence to be written to a running program
/// are only sent if the terminal is the focused terminal.
pub fn test_escape_sequences_sent_to_focused_terminal() -> Builder {
    new_builder()
        // TODO(CORE-2857) There is some flakiness with long-running commands exiting.
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(TestStep::new("Create a new session").with_keystrokes(&[cmd_or_ctrl_shift("d")]))
        .with_step(
            wait_until_bootstrapped_pane(0 /* tab_index */, 1 /* pane_index */)
                .set_timeout(Duration::from_secs(10))
                .add_assertion(assert_terminal_bootstrapped(
                    0, /* tab_index */ 1, /* pane_index */
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Previous Pane").add_named_assertion(
                "Assert first pane is focused",
                assert_focused_pane_index(0 /* tab_index */, 0 /* pane_index */),
            ),
        )
        .with_step(
            TestStep::new("Execute sleep")
                .with_typed_characters(&["sleep 999"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_long_running_block_executing(
                    true, 0, /* tab_index */
                    0, /* pane_index */
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Next Pane").add_named_assertion(
                "Assert second pane is focused",
                assert_focused_pane_index(0 /* tab_index */, 1 /* pane_index */),
            ),
        )
        .with_step(
            execute_long_running_command_for_pane(
                0, /* tab_index */
                1, /* pane_index */
                "vim",
            )
            .add_assertion(assert_alt_grid_active(
                0, /* tab_index */
                1, /* pane_index */
                true,
            )),
        )
        .with_step(
            TestStep::new("Ensure the cursor moves to the left in vim by pressing `shift-left`")
                .with_typed_characters(&["a"])
                .with_keystrokes(&["shift-left"])
                .with_typed_characters(&["bar"])
                // The running command in the first pane should be unchanged.
                .add_assertion(assert_active_block_output(
                    "", 0, /* tab_index */
                    0, /* pane_index */
                ))
                .add_assertion(assert_alt_screen_output(
                    ExactLine::from("bar"),
                    0, /* tab_index */
                    1, /* pane_index */
                )),
        )
}

// When SGR_MOUSE is set, that means we'll only intercept the mouse event
// if mouse reporting is disabled. Since it's enabled in this case, that means
// we shouldn't intercept (vim should handle the right-click) so we don't want to
// open the context-menu on right-click in this case.
pub fn test_alt_screen_context_menu_with_sgr_with_mouse_reporting() -> Builder {
    let mut builder = new_builder()
        .with_user_defaults(HashMap::from([(
            MouseReportingEnabled::storage_key().to_owned(),
            serde_json::to_string(&true).expect("bool should convert to string"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0));

    let exit_step = TestStep::new("Close vim")
        .with_keystrokes(&["escape"])
        .with_typed_characters(&[":q!"])
        .with_keystrokes(&["enter"]);
    let steps = run_alt_grid_program(
        "vim",
        0,
        0,
        exit_step,
        vec![
            TestStep::new("Turn SGR_MOUSE on for vim")
                .with_typed_characters(&[":set mouse=a"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_model_term_mode(TermMode::SGR_MOUSE, true)),
            TestStep::new("Right click to try to open context menu but it shouldn't open")
                .with_event(Event::RightMouseDown {
                    position: Vector2F::new(500., 100.),
                    cmd: false,
                    shift: false,
                    click_count: 1,
                })
                .add_assertion(assert_context_menu_is_open(false)),
        ],
    );

    builder = builder.with_steps(steps);
    builder
}

// When SGR_MOUSE is set, that means we'll only intercept the mouse event
// if mouse reporting is disabled. Thus, we want to open the context-menu
// on right-click in this case.
pub fn test_alt_screen_context_menu_with_sgr_without_mouse_reporting() -> Builder {
    let mut builder = new_builder()
        .with_user_defaults(HashMap::from([(
            MouseReportingEnabled::storage_key().to_owned(),
            serde_json::to_string(&false).expect("bool should convert to string"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0));

    let exit_step = TestStep::new("Close vim")
        .with_keystrokes(&["escape"])
        .with_typed_characters(&[":q!"])
        .with_keystrokes(&["enter"]);
    let steps = run_alt_grid_program(
        "vim",
        0,
        0,
        exit_step,
        vec![
            TestStep::new("Turn SGR_MOUSE on for vim")
                .with_typed_characters(&[":set mouse=a"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_model_term_mode(TermMode::SGR_MOUSE, true)),
            TestStep::new("Right click to open context menu")
                .with_event(Event::RightMouseDown {
                    position: Vector2F::new(400., 100.),
                    cmd: false,
                    shift: false,
                    click_count: 1,
                })
                .add_assertion(assert_context_menu_is_open(true)),
            TestStep::new("Close the menu with escape")
                .with_keystrokes(&["escape"])
                .add_assertion(assert_context_menu_is_open(false)),
        ],
    );

    builder = builder.with_steps(steps);
    builder
}

// When SGR_MOUSE is not set, we always intercept the mouse events
// so we definitely want to open the context-menu on right-click.
pub fn test_alt_screen_context_menu_without_sgr_with_mouse_reporting() -> Builder {
    let mut builder = new_builder()
        .with_user_defaults(HashMap::from([(
            MouseReportingEnabled::storage_key().to_owned(),
            serde_json::to_string(&true).expect("bool should convert to string"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0));

    let exit_step = TestStep::new("Close vim")
        .with_keystrokes(&["escape"])
        .with_typed_characters(&[":q!"])
        .with_keystrokes(&["enter"]);
    let steps = run_alt_grid_program(
        "vim",
        0,
        0,
        exit_step,
        vec![
            TestStep::new("Turn SGR_MOUSE off for vim")
                .with_typed_characters(&[":set mouse="])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_model_term_mode(TermMode::SGR_MOUSE, false)),
            TestStep::new("Right click to open context menu")
                .with_event(Event::RightMouseDown {
                    position: Vector2F::new(400., 100.),
                    cmd: false,
                    shift: false,
                    click_count: 1,
                })
                .add_assertion(assert_context_menu_is_open(true)),
            TestStep::new("Close the menu with escape")
                .with_keystrokes(&["escape"])
                .add_assertion(assert_context_menu_is_open(false)),
        ],
    );

    builder = builder.with_steps(steps);
    builder
}

// When SGR_MOUSE is not set, we always intercept the mouse events
// so we definitely want to open the context-menu on right-click.
pub fn test_alt_screen_context_menu_without_sgr_without_mouse_reporting() -> Builder {
    let mut builder = new_builder()
        .with_user_defaults(HashMap::from([(
            MouseReportingEnabled::storage_key().to_owned(),
            serde_json::to_string(&false).expect("bool should convert to string"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0));

    let exit_step = TestStep::new("Close vim")
        .with_keystrokes(&["escape"])
        .with_typed_characters(&[":q!"])
        .with_keystrokes(&["enter"]);
    let steps = run_alt_grid_program(
        "vim",
        0,
        0,
        exit_step,
        vec![
            TestStep::new("Turn SGR_MOUSE off for vim")
                .with_typed_characters(&[":set mouse="])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_model_term_mode(TermMode::SGR_MOUSE, false)),
            TestStep::new("Right click to open context menu")
                .with_event(Event::RightMouseDown {
                    position: Vector2F::new(400., 100.),
                    cmd: false,
                    shift: false,
                    click_count: 1,
                })
                .add_assertion(assert_context_menu_is_open(true)),
            TestStep::new("Close the menu with escape")
                .with_keystrokes(&["escape"])
                .add_assertion(assert_context_menu_is_open(false)),
        ],
    );

    builder = builder.with_steps(steps);
    builder
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_pane_group_state_single_pane() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_python_interpreter_in_tab(0))
        .with_step(
            new_step_with_default_assertions("exit the interpreter without errors")
                .with_keystrokes(&["ctrl-d"])
                .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Normal)),
        )
        .with_step(
            execute_command_for_single_terminal_in_tab(
                0,
                "false".to_owned(),
                ExpectedExitStatus::Failure,
                (),
            )
            .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Errored)),
        )
        .with_step(
            execute_command_for_single_terminal_in_tab(
                0,
                "ls".to_owned(),
                ExpectedExitStatus::Success,
                (),
            )
            .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Normal)),
        )
        .with_step(execute_python_interpreter_in_tab(0))
        .with_step(
            new_step_with_default_assertions("exit the interpreter with an error status")
                .with_typed_characters(&["exit(1)"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Errored)),
        )
}

// TODO(CORE-2721): Block count / index Failed b/c of in-band generators
pub fn test_pane_group_state_multi_pane() -> Builder {
    new_builder()
        .with_step(
            new_step_with_default_assertions("create 2 additional panes")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 0))
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(wait_until_bootstrapped_pane(0, 2))
        .with_steps(
            open_command_palette_and_run_action("Activate Previous Pane")
                .add_named_assertion("Assert pane 1 is focused", assert_focused_pane_index(0, 1)),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Previous Pane")
                .add_named_assertion("Assert pane 0 is focused", assert_focused_pane_index(0, 0)),
        )
        .with_step(
            TestStep::new("(pane 0) run long-running command")
                .with_typed_characters(&["python3"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_pane_group_has_state(
                    0,
                    TerminalViewState::LongRunning,
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Next Pane")
                .add_named_assertion("Assert pane 1 is focused", assert_focused_pane_index(0, 1)),
        )
        .with_step(
            TestStep::new("(pane 1) run another long-running command")
                .with_typed_characters(&["python3"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_pane_group_has_state(
                    0,
                    TerminalViewState::LongRunning,
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Next Pane")
                .add_named_assertion("Assert pane 2 is focused", assert_focused_pane_index(0, 2)),
        )
        .with_step(
            TestStep::new("(pane 2) run command that errors")
                .with_typed_characters(&["false"])
                .with_keystrokes(&["enter"])
                // the most recent overall state change is now that we're in an error state
                .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Errored)),
        )
        .with_step(
            TestStep::new("(pane 2) clear the error by running normal command")
                .with_typed_characters(&["true"])
                .with_keystrokes(&["enter"])
                // pane 2 is normal, so we return to our next most-recent state, which is long running
                .add_assertion(assert_pane_group_has_state(
                    0,
                    TerminalViewState::LongRunning,
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Previous Pane")
                .add_named_assertion("Assert pane 1 is focused", assert_focused_pane_index(0, 1)),
        )
        .with_step(
            TestStep::new(
                "(pane 1) terminate command with non-zero status, and verify error state",
            )
            .with_typed_characters(&["exit(1)"])
            .with_keystrokes(&["enter"])
            // once pane 1 has finished & errored, our most recent state is errored
            .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Errored)),
        )
        .with_step(
            TestStep::new("(pane 1) clear the error by running normal command")
                .with_typed_characters(&["ls"])
                .with_keystrokes(&["enter"])
                // pane 1 is normal, so we return to our next most-recent state, which is long running
                .add_assertion(assert_pane_group_has_state(
                    0,
                    TerminalViewState::LongRunning,
                )),
        )
        .with_steps(
            open_command_palette_and_run_action("Activate Previous Pane")
                .add_named_assertion("Assert pane 0 is focused", assert_focused_pane_index(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions_for_pane(
                "(pane 0) terminate command with no errors, returning back to normal state",
                0,
                2,
            )
            .with_keystrokes(&["ctrl-d"])
            .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Normal)),
        )
}

pub fn test_pane_group_state_close_pane() -> Builder {
    new_builder()
        .with_step(
            new_step_with_default_assertions("create 1 additional pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 0))
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            TestStep::new("(pane 2) run long-running command")
                .with_typed_characters(&["python3"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_pane_group_has_state(
                    0,
                    TerminalViewState::LongRunning,
                )),
        )
        .with_step(
            new_step_with_default_assertions("(pane 2) close pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("w")])
                .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Normal)),
        )
}

pub fn test_pane_group_state_clear_blocks() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("run a non existent command")
                .with_typed_characters(&["dsfsdjfpdsojfpdsofj"])
                .with_keystrokes(&["enter"])
                .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Errored)),
        )
        .with_step(
            new_step_with_default_assertions("clear the pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("k")])
                .add_assertion(assert_pane_group_has_state(0, TerminalViewState::Normal)),
        )
}

/// Create a small window and enough terminal panes inside it so that a new pane would normally have
/// a narrow width. Check that the Agent Mode pane is wide enough regardless.
pub fn test_agent_mode_pane_minimum_size() -> Builder {
    const WINDOW_ID_KEY: &str = "small_window_id";

    new_builder()
        .with_step(set_window_custom_size(40, 120))
        .with_step(add_and_save_window(WINDOW_ID_KEY))
        .with_step(
            new_step_with_default_assertions("Check the new window size")
                .add_named_assertion_with_data_from_prior_step(
                    "Validate window size",
                    move |app, _, step_data_map| {
                        let window_id = step_data_map
                            .get(WINDOW_ID_KEY)
                            .expect("Window ID for new window should exist");

                        let size = app
                            .window_bounds(window_id)
                            .expect("Window should exist")
                            .size();
                        // This doesn't correspond clearly to the given rows and columns due to line
                        // height and padding. There's also some platform-specific variance and room
                        // for floating-point error.
                        assert_approx_eq!(f32, size.x(), 992., epsilon = 2.);
                        assert_approx_eq!(f32, size.y(), 644., epsilon = 2.);
                        AssertionOutcome::Success
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions("Create a new empty pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(
            new_step_with_default_assertions("Create an Agent Mode pane and check its width")
                .with_action(move |app, _, step_data_map| {
                    let window_id = step_data_map
                        .get(WINDOW_ID_KEY)
                        .expect("Window ID for new window should exist");

                    let workspace_view_id = workspace_view(app, *window_id).id();

                    app.dispatch_typed_action(
                        *window_id,
                        &[workspace_view_id],
                        &WorkspaceAction::NewPaneInAgentMode {
                            entrypoint: AgentModeEntrypoint::TabBar,
                            zero_state_prompt_suggestion_type: None,
                        },
                    );
                })
                .add_named_assertion_with_data_from_prior_step(
                    "Check Agent Mode pane width",
                    |app, _, step_data_map| {
                        let window_id = step_data_map
                            .get(WINDOW_ID_KEY)
                            .expect("Window ID for new window should exist");

                        let pane_group = pane_group_view(app, *window_id, 0);
                        pane_group.read(app, |view, app| {
                            let Some(agent_mode_pane) = view.terminal_view_at_pane_index(2, app)
                            else {
                                return AssertionOutcome::failure(
                                    "no terminal pane at pane_index 2".to_owned(),
                                );
                            };

                            let pane_width =
                                agent_mode_pane.as_ref(app).size_info().pane_size_px().x();

                            // Approx equality to handle pane borders, etc.
                            assert_approx_eq!(
                                f32,
                                pane_width - AGENT_MODE_PANE_DEFAULT_MINIMUM_WIDTH,
                                0.,
                                epsilon = 4.
                            );

                            AssertionOutcome::Success
                        })
                    },
                ),
        )
}

// cheating a little bit in this test; it's hard to tell if the create folder dialog is open from
// the workspace view, but we DO force warp drive open to show the dialog, so we can look for that
pub fn test_create_folder_from_command_palette() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(join_a_workspace())
        .with_step(go_offline())
        .with_steps(
            open_command_palette_and_run_action("Create a New Team Folder")
                .add_assertion(assert_warp_drive_is_closed()),
        )
        .with_steps(
            open_command_palette_and_run_action("Create a New Personal Folder")
                .add_assertion(assert_warp_drive_is_closed()),
        )
        .with_step(go_online())
        .with_steps(
            open_command_palette_and_run_action("Create a New Team Folder")
                .add_assertion(assert_warp_drive_is_open()),
        )
        .with_steps(
            open_command_palette_and_run_action("Create a New Personal Folder")
                .add_assertion(assert_warp_drive_is_open()),
        )
}

pub fn test_tab_behavior_setting() -> Builder {
    let completions_binding_name = "input:open_completion_suggestions";
    let autosuggestions_binding_name = "editor_view:insert_autosuggestion";

    let expected_completion_binding_name = if OperatingSystem::get().is_mac() {
        "⌃Space"
    } else {
        "Ctrl Space"
    };

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(toggle_setting(SettingsAction::FeaturesPageToggle(
            FeaturesPageAction::SetTabBehavior(TabBehavior::Autosuggestions),
        )))
        .with_step(assert_binding_display_string(
            autosuggestions_binding_name,
            Some("Tab"),
        ))
        .with_step(assert_binding_display_string(
            completions_binding_name,
            Some(expected_completion_binding_name),
        ))
        .with_step(toggle_setting(SettingsAction::FeaturesPageToggle(
            FeaturesPageAction::SetTabBehavior(TabBehavior::Completions),
        )))
        .with_step(assert_binding_display_string(
            autosuggestions_binding_name,
            None,
        ))
        .with_step(assert_binding_display_string(
            completions_binding_name,
            Some("Tab"),
        ))
}

pub fn test_context_chips_prompt_at_bootstrap() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_user_defaults(HashMap::from([
            (HonorPS1::storage_key().to_owned(), false.to_string()),
            (String::from("SavedPrompt"), String::from("Default")),
        ]))
        .with_step(
            new_step_with_default_assertions("Check Warp prompt")
                .add_assertion(assert_working_dir_is_present(0)),
        )
}

pub fn test_pass_control_sequences_to_long_running_block() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_long_running_command_for_pane(0, 0, "cat -v"))
        .with_step(
            TestStep::new("Type F2 into cat -v")
                .with_keystrokes(&["f2"])
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _| {
                        // `cat -v` writes the control sequences as visible characters.
                        let output = view
                            .model
                            .lock()
                            .block_list()
                            .active_block()
                            .output_to_string();
                        async_assert_eq!(
                            output,
                            String::from("^[OQ"),
                            "Control sequences should be passed through to the long-running command"
                        )
                    })
                }),
        )
}

/// Test that undo close stack cleanup works safely when a window is closed
/// before the grace period expires. This reproduces the specific bug scenario:
/// 1. Open a new window
/// 2. In that window open a new tab
/// 3. Close that tab (adds it to undo close stack)
/// 4. Close the window before grace period expires
/// 5. Verify cleanup handles missing window gracefully
pub fn test_undo_close_stack_timeout_cleanup() -> Builder {
    FeatureFlag::UndoClosedPanes.set_enabled(true);
    new_builder()
        // This test is Mac-only due to differences in window management on Linux
        .set_should_run_test(|| cfg!(target_os = "macos"))
        // Set a 5-second grace period to give time to close the window before it expires
        .with_user_defaults(HashMap::from([(
            "UndoCloseGracePeriod".to_owned(),
            serde_json::to_string(&Duration::from_secs(5))
                .expect("Duration should serialize to JSON"),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert we have only 1 window open at start")
                .add_assertion(move |app, _| async_assert_eq!(app.window_ids().len(), 1)),
        )
        .with_step(add_and_save_window("new_window_id"))
        .with_step(
            new_step_with_default_assertions("Assert we now have 2 windows")
                .add_assertion(move |app, _| async_assert_eq!(app.window_ids().len(), 2)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Add a new tab in the new window")
                .with_click_on_saved_position(NEW_TAB_BUTTON_POSITION_ID)
                .add_assertion(assert_tab_count(2)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Close the tab to trigger undo close stack")
                .with_hover_over_saved_position("close_tab_button:1")
                .with_click_on_saved_position("close_tab_button:1")
                .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            TestStep::new("Close the new window")
                .with_action(|app, _, data_map| {
                    let window_id = data_map
                        .get("new_window_id")
                        .expect("Expected window id to be in data map");
                    app.update(|ctx| {
                        WindowManager::as_ref(ctx)
                            .close_window(*window_id, TerminationMode::ForceTerminate);
                    });

                    // Switch back to the original window (WindowId(0))
                    let original_window = app.window_ids()[0];
                    app.update(|ctx| {
                        WindowManager::as_ref(ctx).show_window_and_focus_app(original_window);
                    });
                })
                .add_assertion(|app, _| {
                    // Verify we now have only 1 window remaining
                    async_assert_eq!(app.window_ids().len(), 1)
                }),
        )
        .with_step(
            TestStep::new("Wait for undo close grace period to expire and trigger cleanup")
                .set_timeout(Duration::from_secs(8))
                .with_action(|_app, _, _data| {
                    // Wait longer than the grace period to ensure cleanup is triggered
                    std::thread::sleep(Duration::from_secs(6));
                })
                // After waiting, verify the application is still stable
                .add_assertion(|app, _| {
                    // Simple stability check - ensure we still have 1 window
                    async_assert_eq!(app.window_ids().len(), 1)
                }),
        )
}
