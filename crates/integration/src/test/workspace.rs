//! Integration tests for workspace-level behavior.

use std::{fs, time::Duration};

use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use settings::Setting as _;
use warp::integration_testing::terminal::{
    assert_command_executed_for_single_terminal_in_tab, assert_focused_editor_in_tab,
    assert_long_running_block_executing_for_single_terminal_in_tab,
};
use warp::integration_testing::view_getters::{terminal_view, workspace_view};
use warp::integration_testing::window::{
    add_and_save_window, assert_num_windows_open, save_active_window_id,
};
use warp::integration_testing::workspace::{
    assert_focused_tab_index, assert_tab_count, press_native_modal_button,
};
use warp::{
    cmd_or_ctrl_shift,
    features::FeatureFlag,
    integration_testing::{
        clipboard::assert_clipboard_contains_string,
        pane_group::assert_focused_pane_index,
        step::new_step_with_default_assertions,
        terminal::{
            assert_active_session_local_path, execute_command,
            execute_command_for_single_terminal_in_tab,
            util::{current_shell_starter_and_version, ExpectedExitStatus},
            wait_until_bootstrapped_pane, wait_until_bootstrapped_single_pane_for_tab,
        },
    },
    settings::PaneSettings,
    terminal::shell::ShellType,
    workspace::tab_settings::{TabSettings, VerticalTabsDisplayGranularity},
    workspace::{WorkspaceAction, NEW_TAB_BUTTON_POSITION_ID},
};
use warpui::{
    async_assert, async_assert_eq,
    event::{Event, ModifiersState},
    integration::{AssertionCallback, AssertionOutcome, TestStep},
    windowing::WindowManager,
    SingletonEntity, TypedActionView, WindowId,
};

use crate::{util::skip_if_powershell_core_2303, Builder};

use super::new_builder;

const SOURCE_WINDOW_KEY: &str = "source window";
const TARGET_WINDOW_KEY: &str = "target window";
const DETACHED_WINDOW_KEY: &str = "detached window";
const METADATA_TAB_TITLE: &str = "Integration Metadata Tab";
const METADATA_BRANCH: &str = "main";
const METADATA_CLICKED_PANE_TITLE: &str = "Integration Metadata Clicked Pane";
const METADATA_PANE_TITLE: &str = "Integration Metadata Pane";
const METADATA_PANE_BRANCH: &str = "pane-branch";
const METADATA_PANE_DIRECTORY: &str = "active-pane";

fn tab_position_id(tab_index: usize) -> String {
    format!("tab_position_{tab_index}")
}

fn vertical_tab_pane_row_position_id(app: &mut warpui::App, window_id: WindowId) -> String {
    let workspace = workspace_view(app, window_id);
    let pane_group = workspace.read(app, |workspace, _ctx| {
        workspace
            .get_pane_group_view(0)
            .expect("pane group should exist")
            .clone()
    });
    let pane_group_id = pane_group.id();
    pane_group.read(app, |pane_group, ctx| {
        let pane_id = pane_group.focused_pane_id(ctx);
        format!("vertical_tabs:pane_row:{pane_group_id:?}:{pane_id}")
    })
}

fn vertical_tab_pane_row_position_id_for_pane_index(
    app: &mut warpui::App,
    window_id: WindowId,
    pane_index: usize,
) -> String {
    let workspace = workspace_view(app, window_id);
    let pane_group = workspace.read(app, |workspace, _ctx| {
        workspace
            .get_pane_group_view(0)
            .expect("pane group should exist")
            .clone()
    });
    let pane_group_id = pane_group.id();
    pane_group.read(app, |pane_group, _ctx| {
        let pane_id = pane_group
            .pane_id_from_index(pane_index)
            .expect("pane should exist at index");
        format!("vertical_tabs:pane_row:{pane_group_id:?}:{pane_id}")
    })
}

fn first_vertical_tab_pane_row_position_id(app: &mut warpui::App, window_id: WindowId) -> String {
    vertical_tab_pane_row_position_id_for_pane_index(app, window_id, 0)
}

fn should_run_tab_context_menu_metadata_test() -> bool {
    let (starter, _) = current_shell_starter_and_version();
    starter.shell_type() != ShellType::PowerShell
}

fn set_active_tab_name(name: &'static str) -> TestStep {
    TestStep::new("Set active tab name").with_action(move |app, window_id, _| {
        let workspace = workspace_view(app, window_id);
        workspace.update(app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::SetActiveTabName(name.to_string()), ctx);
        });
    })
}

fn set_active_pane_name(name: &'static str) -> TestStep {
    TestStep::new("Set active pane name").with_action(move |app, window_id, _| {
        let workspace = workspace_view(app, window_id);
        let pane_group = workspace.read(app, |workspace, _ctx| {
            workspace
                .get_pane_group_view(0)
                .expect("pane group should exist")
                .clone()
        });
        pane_group.update(app, |pane_group, ctx| {
            let pane_id = pane_group.focused_pane_id(ctx);
            let pane = pane_group
                .pane_by_id(pane_id)
                .expect("focused pane should exist");
            pane.pane_configuration().update(ctx, |configuration, ctx| {
                configuration.set_custom_vertical_tabs_title(name, ctx);
            });
        });
    })
}

fn enable_vertical_tabs(display_granularity: VerticalTabsDisplayGranularity) -> TestStep {
    FeatureFlag::VerticalTabs.set_enabled(true);
    new_step_with_default_assertions("Enable vertical tabs").add_assertion(
        move |app, _window_id| {
            TabSettings::handle(app).update(app, |settings, ctx| {
                settings
                    .use_vertical_tabs
                    .set_value(true, ctx)
                    .expect("vertical tabs setting should update");
                settings
                    .vertical_tabs_display_granularity
                    .set_value(display_granularity, ctx)
                    .expect("vertical tabs display granularity should update");
                async_assert!(
                    *settings.use_vertical_tabs
                        && *settings.vertical_tabs_display_granularity.value()
                            == display_granularity
                )
            })
        },
    )
}

fn open_horizontal_tab_context_menu(step_name: &'static str) -> TestStep {
    new_step_with_default_assertions(step_name)
        .with_right_click_on_saved_position(tab_position_id(0))
}

fn open_vertical_tab_context_menu(step_name: &'static str) -> TestStep {
    new_step_with_default_assertions(step_name)
        .with_right_click_on_saved_position_fn(vertical_tab_pane_row_position_id)
}
fn open_first_vertical_tab_pane_context_menu(step_name: &'static str) -> TestStep {
    new_step_with_default_assertions(step_name)
        .with_right_click_on_saved_position_fn(first_vertical_tab_pane_row_position_id)
}

fn add_tab_context_metadata_setup_steps(builder: Builder) -> Builder {
    builder
        .set_should_run_test(should_run_tab_context_menu_metadata_test)
        .use_tmp_filesystem_for_test_root_directory()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(set_active_tab_name(METADATA_TAB_TITLE))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            format!(
                "git init -b {METADATA_BRANCH}; git config user.email \"test@test.com\"; git config user.name \"Git TestUser\""
            ),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "touch file".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Git branch metadata should be populated")
                .set_timeout(Duration::from_secs(15))
                .add_assertion(assert_current_git_branch(0, METADATA_BRANCH)),
        )
}

fn add_active_pane_context_metadata_setup_steps(builder: Builder) -> Builder {
    builder
        .with_step(set_active_pane_name(METADATA_CLICKED_PANE_TITLE))
        .with_step(
            new_step_with_default_assertions("Create active split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(set_active_pane_name(METADATA_PANE_TITLE))
        .with_step(execute_command(
            0,
            1,
            format!(
                "mkdir {METADATA_PANE_DIRECTORY}; cd {METADATA_PANE_DIRECTORY}; git init -b {METADATA_PANE_BRANCH}; git config user.email \"test@test.com\"; git config user.name \"Git TestUser\"; touch file"
            ),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Active pane git branch metadata should be populated")
                .set_timeout(Duration::from_secs(15))
                .add_assertion(assert_current_git_branch(1, METADATA_PANE_BRANCH)),
        )
}

fn add_horizontal_tab_context_metadata_copy_steps(
    builder: Builder,
    open_tab_context_menu: fn(&'static str) -> TestStep,
) -> Builder {
    builder
        .with_step(
            open_tab_context_menu("Open tab context menu for title copy").add_assertion(
                assert_saved_positions_absent(&[
                    "Copy branch",
                    "Copy pane title",
                    "Copy working directory",
                    "Copy pull request link",
                ]),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Copy tab title from tab context menu")
                .with_click_on_saved_position("Copy tab title")
                .add_assertion(assert_clipboard_contains_string(
                    METADATA_TAB_TITLE.to_string(),
                )),
        )
}

fn add_vertical_tab_context_metadata_copy_steps(
    builder: Builder,
    open_tab_context_menu: fn(&'static str) -> TestStep,
) -> Builder {
    builder
        .with_step(open_tab_context_menu(
            "Open tab context menu for branch copy",
        ))
        .with_step(
            new_step_with_default_assertions("Copy branch from tab context menu")
                .with_click_on_saved_position("Copy branch")
                .add_assertion(assert_clipboard_contains_string(
                    METADATA_BRANCH.to_string(),
                )),
        )
        .with_step(open_tab_context_menu(
            "Open tab context menu for title copy",
        ))
        .with_step(
            new_step_with_default_assertions("Copy tab title from tab context menu")
                .with_click_on_saved_position("Copy tab title")
                .add_assertion(assert_clipboard_contains_string(
                    METADATA_TAB_TITLE.to_string(),
                )),
        )
        .with_step(open_tab_context_menu(
            "Open tab context menu for working directory copy",
        ))
        .with_step(
            new_step_with_default_assertions("Copy working directory from tab context menu")
                .with_click_on_saved_position("Copy working directory")
                .add_assertion(assert_clipboard_contains_home()),
        )
}

fn add_vertical_pane_context_metadata_copy_steps(
    builder: Builder,
    open_tab_context_menu: fn(&'static str) -> TestStep,
) -> Builder {
    builder
        .with_step(open_tab_context_menu(
            "Open pane context menu for branch copy",
        ))
        .with_step(
            new_step_with_default_assertions("Copy branch from pane context menu")
                .with_click_on_saved_position("Copy branch")
                .add_assertion(assert_clipboard_contains_string(
                    METADATA_BRANCH.to_string(),
                )),
        )
        .with_step(open_tab_context_menu(
            "Open pane context menu for title copy",
        ))
        .with_step(
            new_step_with_default_assertions("Copy pane title from pane context menu")
                .with_click_on_saved_position("Copy pane title")
                .add_assertion(assert_clipboard_contains_string(
                    METADATA_CLICKED_PANE_TITLE.to_string(),
                )),
        )
        .with_step(open_tab_context_menu(
            "Open pane context menu for working directory copy",
        ))
        .with_step(
            new_step_with_default_assertions("Copy working directory from pane context menu")
                .with_click_on_saved_position("Copy working directory")
                .add_assertion(assert_clipboard_contains_home()),
        )
}

fn assert_current_git_branch(
    pane_index: usize,
    expected_branch: &'static str,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = terminal_view(app, window_id, 0, pane_index);
        terminal_view.read(app, |terminal_view, ctx| {
            async_assert_eq!(
                terminal_view.current_git_branch(ctx),
                Some(expected_branch.to_string())
            )
        })
    })
}

fn assert_saved_positions_absent(labels: &'static [&'static str]) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let presenter = app.presenter(window_id).expect("presenter should exist");
        let presenter = presenter.borrow();
        let position_cache = presenter.position_cache();
        async_assert!(labels
            .iter()
            .all(|label| position_cache.get_position(label).is_none()))
    })
}

fn assert_clipboard_contains_home() -> AssertionCallback {
    Box::new(|app, _window_id| {
        let clipboard = app.update(|ctx| ctx.clipboard().read());
        let content = match clipboard.paths {
            Some(paths) => paths.join(" "),
            None => clipboard.plain_text,
        };
        let home = std::env::var("HOME").expect("HOME should be set for integration tests");

        async_assert_eq!(content, home)
    })
}

fn focus_other_window(other_window_key: &'static str, known_window_key: &'static str) -> TestStep {
    TestStep::new("Focus other window").with_action(move |app, _, data| {
        let known_window_id = *data
            .get::<_, WindowId>(known_window_key)
            .expect("saved window id should exist");
        let other_window_id = app
            .window_ids()
            .into_iter()
            .find(|window_id| *window_id != known_window_id)
            .expect("other window should exist");
        data.insert(other_window_key, other_window_id);
        app.update(|ctx| {
            WindowManager::as_ref(ctx).show_window_and_focus_app(other_window_id);
        });
    })
}

fn dispatch_mouse_event(app: &mut warpui::App, window_id: WindowId, event: Event) {
    let window = app.read(|ctx| {
        ctx.windows()
            .platform_window(window_id)
            .expect("platform window should exist")
    });
    app.update(|ctx| {
        (window.callbacks().event_callback)(event, ctx);
    });
}

fn tab_bounds(app: &mut warpui::App, window_id: WindowId, tab_index: usize) -> RectF {
    let presenter = app.presenter(window_id).expect("presenter should exist");
    let bounds = presenter
        .borrow()
        .position_cache()
        .get_position(tab_position_id(tab_index))
        .unwrap_or_else(|| panic!("tab_position_{tab_index} should exist for {window_id:?}"));
    bounds
}

fn tab_center(app: &mut warpui::App, window_id: WindowId, tab_index: usize) -> Vector2F {
    tab_bounds(app, window_id, tab_index).center()
}

fn source_local_point_for_screen_point(
    app: &mut warpui::App,
    source_window_id: WindowId,
    screen_point: Vector2F,
) -> Vector2F {
    let source_bounds = app
        .window_bounds(&source_window_id)
        .expect("source window bounds should exist");
    screen_point - source_bounds.origin()
}

fn tab_screen_point(
    app: &mut warpui::App,
    window_id: WindowId,
    tab_index: usize,
    x_offset: f32,
    y_offset: f32,
) -> Vector2F {
    let bounds = tab_bounds(app, window_id, tab_index);
    let window_bounds = app
        .window_bounds(&window_id)
        .expect("window bounds should exist");
    window_bounds.origin() + vec2f(bounds.min_x() + x_offset, bounds.min_y() + y_offset)
}

fn focus_saved_window(window_key: &'static str) -> TestStep {
    TestStep::new("Focus saved window").with_action(move |app, _, data| {
        let window_id = *data
            .get::<_, WindowId>(window_key)
            .expect("saved window id should exist");
        app.update(|ctx| {
            WindowManager::as_ref(ctx).show_window_and_focus_app(window_id);
        });
    })
}

fn set_saved_window_origin(window_key: &'static str, origin: Vector2F) -> TestStep {
    TestStep::new("Move saved window").with_action(move |app, _, data| {
        let window_id = *data
            .get::<_, WindowId>(window_key)
            .expect("saved window id should exist");
        let size = app
            .window_bounds(&window_id)
            .expect("window bounds should exist")
            .size();
        app.update(|ctx| {
            ctx.set_and_cache_window_bounds(window_id, RectF::new(origin, size));
        });
    })
}

fn assert_total_tab_count(
    expected_total_tab_count: usize,
) -> impl FnMut(&mut warpui::App, WindowId) -> AssertionOutcome {
    move |app, _| {
        let total_tab_count = app
            .window_ids()
            .into_iter()
            .map(|window_id| {
                let workspace = workspace_view(app, window_id);
                workspace.read(app, |view, _ctx| view.tab_count())
            })
            .sum::<usize>();
        async_assert_eq!(total_tab_count, expected_total_tab_count)
    }
}

fn drag_tabs_feature_enabled() -> bool {
    cfg!(feature = "drag_tabs_to_windows")
}

pub fn test_active_session_follows_focus() -> Builder {
    new_builder()
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_setup(|utils| {
            fs::create_dir(utils.test_dir().join("dir1")).expect("Couldn't create subdirectory");
            fs::create_dir(utils.test_dir().join("dir2")).expect("Couldn't create subdirectory");
        })
        .with_step(
            new_step_with_default_assertions("Ensure initial active session is set")
                .add_assertion(assert_active_session_local_path("~")),
        )
        .with_step(
            new_step_with_default_assertions("Create another session in the same tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            execute_command(0, 1, "cd dir1".to_string(), ExpectedExitStatus::Success, ())
                .add_assertion(assert_active_session_local_path("~/dir1")),
        )
        .with_step(
            new_step_with_default_assertions("Switch to the first session")
                .with_keystrokes(&["cmdorctrl-meta-left"])
                .add_assertion(assert_active_session_local_path("~")),
        )
        .with_step(
            new_step_with_default_assertions("Open a new tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_pane(1, 0))
        .with_step(
            execute_command(1, 0, "cd dir2".to_string(), ExpectedExitStatus::Success, ())
                .add_assertion(assert_active_session_local_path("~/dir2")),
        )
        .with_step(
            new_step_with_default_assertions("Switch to the first tab")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_assertion(assert_active_session_local_path("~")),
        )
        .with_step(
            new_step_with_default_assertions("Close the tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("w"), cmd_or_ctrl_shift("w")])
                .add_assertion(assert_active_session_local_path("~/dir2")),
        )
}

pub fn test_tab_context_menu_copies_metadata() -> Builder {
    let builder = add_tab_context_metadata_setup_steps(new_builder());
    add_horizontal_tab_context_metadata_copy_steps(builder, open_horizontal_tab_context_menu)
}

pub fn test_vertical_tab_context_menu_copies_metadata() -> Builder {
    let builder = add_tab_context_metadata_setup_steps(new_builder())
        .with_step(enable_vertical_tabs(VerticalTabsDisplayGranularity::Tabs));
    add_vertical_tab_context_metadata_copy_steps(builder, open_vertical_tab_context_menu)
}

pub fn test_vertical_pane_context_menu_copies_metadata() -> Builder {
    let builder = add_active_pane_context_metadata_setup_steps(
        add_tab_context_metadata_setup_steps(new_builder())
            .with_step(enable_vertical_tabs(VerticalTabsDisplayGranularity::Panes)),
    );
    add_vertical_pane_context_metadata_copy_steps(
        builder,
        open_first_vertical_tab_pane_context_menu,
    )
}

pub fn test_focus_panes_on_hover() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Create a new session in a split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")])
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            new_step_with_default_assertions("Enable focus pane on hover").add_assertion(
                |app, _| {
                    PaneSettings::handle(app).update(app, |settings, ctx| {
                        settings
                            .focus_panes_on_hover
                            .set_value(true, ctx)
                            .expect("error updating setting");
                        async_assert!(*settings.focus_panes_on_hover)
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Hover over the initial pane's terminal")
                .with_hover_on_saved_position_fn(|app, window_id| {
                    let terminal_view = terminal_view(app, window_id, 0, 0);
                    terminal_view.read(app, |terminal, _| terminal.terminal_position_id())
                })
                .add_assertion(assert_focused_pane_index(0, 0)),
        )
        .with_step(
            new_step_with_default_assertions("Hover back over the second pane's terminal")
                .with_hover_on_saved_position_fn(|app, window_id| {
                    let terminal_view = terminal_view(app, window_id, 0, 1);
                    terminal_view.read(app, |terminal, _| terminal.terminal_position_id())
                })
                .add_assertion(assert_focused_pane_index(0, 1)),
        )
        .with_step(
            new_step_with_default_assertions("Create another new session in a split pane")
                .with_keystrokes(&[cmd_or_ctrl_shift("d")]),
        )
        .with_step(wait_until_bootstrapped_pane(0, 2))
        .with_step(
            new_step_with_default_assertions(
                "Make sure the pane is focused even though the mouse is over the first pane",
            )
            .add_assertion(assert_focused_pane_index(0, 2)),
        )
        .with_step(
            new_step_with_default_assertions("Disable focus pane on hover").add_assertion(
                |app, _| {
                    PaneSettings::handle(app).update(app, |settings, ctx| {
                        settings
                            .focus_panes_on_hover
                            .set_value(false, ctx)
                            .expect("error updating setting");
                        async_assert!(!*settings.focus_panes_on_hover)
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions(
                "Hover over the initial pane's terminal and make sure it's not focused",
            )
            .with_hover_on_saved_position_fn(|app, window_id| {
                let terminal_view = terminal_view(app, window_id, 0, 0);
                terminal_view.read(app, |terminal, _| terminal.terminal_position_id())
            })
            .add_assertion(assert_focused_pane_index(0, 2)),
        )
}

pub fn test_close_tab_with_long_running_process() -> Builder {
    new_builder()
        .set_should_run_test(|| cfg!(any(target_os = "linux", target_os = "freebsd")))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open a new tab")
                .with_click_on_saved_position(NEW_TAB_BUTTON_POSITION_ID),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            TestStep::new("Execute long-running command")
                .with_typed_characters(&["python3"])
                .with_keystrokes(&["enter"])
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 1),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Close the tab with a long-running command")
                .with_hover_over_saved_position("close_tab_button:1")
                .with_click_on_saved_position("close_tab_button:1")
                .add_assertion(assert_tab_count(2))
                .add_assertion(
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 1),
                ),
        )
        .with_step(press_native_modal_button(0))
        .with_step(TestStep::new("Wait for tab to close").add_assertion(assert_tab_count(1)))
}

pub fn test_reorder_tabs_with_drag() -> Builder {
    new_builder()
        .set_should_run_test(drag_tabs_feature_enabled)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo source-zero".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("Open a new tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "echo source-one".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Drag the second tab to the first position")
                .with_action(|app, window_id, _| {
                    let start = tab_center(app, window_id, 1);
                    dispatch_mouse_event(
                        app,
                        window_id,
                        Event::LeftMouseDown {
                            position: start,
                            modifiers: ModifiersState::default(),
                            click_count: 1,
                            is_first_mouse: false,
                        },
                    );
                })
                .with_action(|app, window_id, _| {
                    let start = tab_center(app, window_id, 1);
                    dispatch_mouse_event(
                        app,
                        window_id,
                        Event::LeftMouseDragged {
                            position: start + vec2f(12.0, 0.0),
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, window_id, _| {
                    let target_bounds = tab_bounds(app, window_id, 0);
                    let target = vec2f(target_bounds.min_x() + 5.0, target_bounds.center().y());
                    dispatch_mouse_event(
                        app,
                        window_id,
                        Event::LeftMouseDragged {
                            position: target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, window_id, _| {
                    let target_bounds = tab_bounds(app, window_id, 0);
                    let target = vec2f(target_bounds.min_x() + 5.0, target_bounds.center().y());
                    dispatch_mouse_event(
                        app,
                        window_id,
                        Event::LeftMouseUp {
                            position: target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .add_assertion(assert_focused_tab_index(0))
                .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                    0,
                    "echo source-one".to_string(),
                ))
                .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                    1,
                    "echo source-zero".to_string(),
                )),
        )
}

pub fn test_detach_tab_to_new_window_with_drag() -> Builder {
    new_builder()
        .set_should_run_test(drag_tabs_feature_enabled)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            execute_command_for_single_terminal_in_tab(
                0,
                "echo source-zero".to_string(),
                ExpectedExitStatus::Success,
                (),
            )
            .add_assertion(save_active_window_id(SOURCE_WINDOW_KEY)),
        )
        .with_step(
            new_step_with_default_assertions("Open a new tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "echo source-one".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Detach the second tab into a standalone window")
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDown {
                            position: start,
                            modifiers: ModifiersState::default(),
                            click_count: 1,
                            is_first_mouse: false,
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: start + vec2f(12.0, 0.0),
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: start + vec2f(0.0, 140.0),
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    let drop_position = start + vec2f(220.0, 220.0);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: drop_position,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    let drop_position = start + vec2f(220.0, 220.0);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseUp {
                            position: drop_position,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .add_assertion(assert_num_windows_open(2))
                .add_assertion(assert_total_tab_count(2))
                .add_assertion(assert_tab_count(1)),
        )
        .with_step(
            focus_other_window(DETACHED_WINDOW_KEY, SOURCE_WINDOW_KEY)
                .add_assertion(assert_tab_count(1))
                .add_assertion(assert_focused_editor_in_tab(0))
                .add_assertion(assert_command_executed_for_single_terminal_in_tab(
                    0,
                    "echo source-one".to_string(),
                )),
        )
        .with_step(focus_saved_window(SOURCE_WINDOW_KEY).add_assertion(assert_tab_count(1)))
}

pub fn test_attach_tab_to_other_window_and_continue_drag() -> Builder {
    new_builder()
        .set_should_run_test(drag_tabs_feature_enabled)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            execute_command_for_single_terminal_in_tab(
                0,
                "echo source-zero".to_string(),
                ExpectedExitStatus::Success,
                (),
            )
            .add_assertion(save_active_window_id(SOURCE_WINDOW_KEY)),
        )
        .with_step(
            new_step_with_default_assertions("Open a new tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "echo source-one".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(add_and_save_window(TARGET_WINDOW_KEY))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo target-only".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(set_saved_window_origin(
            SOURCE_WINDOW_KEY,
            vec2f(100.0, 100.0),
        ))
        .with_step(set_saved_window_origin(
            TARGET_WINDOW_KEY,
            vec2f(900.0, 100.0),
        ))
        .with_step(focus_saved_window(SOURCE_WINDOW_KEY))
        .with_step(
            TestStep::new("Attach the dragged tab into another window and keep dragging")
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDown {
                            position: start,
                            modifiers: ModifiersState::default(),
                            click_count: 1,
                            is_first_mouse: false,
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: start + vec2f(12.0, 0.0),
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 1);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: start + vec2f(0.0, 140.0),
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let target_window_id = *data
                        .get::<_, WindowId>(TARGET_WINDOW_KEY)
                        .expect("saved target window id should exist");
                    let target_tab_bounds = tab_bounds(app, target_window_id, 0);
                    let attach_before = tab_screen_point(
                        app,
                        target_window_id,
                        0,
                        8.0,
                        target_tab_bounds.height() / 2.0,
                    );
                    let source_local_target =
                        source_local_point_for_screen_point(app, source_window_id, attach_before);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let target_window_id = *data
                        .get::<_, WindowId>(TARGET_WINDOW_KEY)
                        .expect("saved target window id should exist");
                    let target_tab_bounds = tab_bounds(app, target_window_id, 0);
                    let reorder_after = tab_screen_point(
                        app,
                        target_window_id,
                        0,
                        target_tab_bounds.width() + 40.0,
                        target_tab_bounds.height() / 2.0,
                    );
                    let source_local_target =
                        source_local_point_for_screen_point(app, source_window_id, reorder_after);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseUp {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .add_assertion(assert_num_windows_open(2))
                .add_assertion(assert_total_tab_count(3)),
        )
        .with_step(
            focus_saved_window(TARGET_WINDOW_KEY)
                .add_assertion(assert_tab_count(2))
                .add_assertion(assert_focused_tab_index(1))
                .add_assertion(assert_focused_editor_in_tab(1)),
        )
        .with_step(focus_saved_window(SOURCE_WINDOW_KEY).add_assertion(assert_tab_count(1)))
}

pub fn test_single_tab_handoff_continues_drag() -> Builder {
    new_builder()
        .set_should_run_test(drag_tabs_feature_enabled)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            execute_command_for_single_terminal_in_tab(
                0,
                "echo single-source".to_string(),
                ExpectedExitStatus::Success,
                (),
            )
            .add_assertion(save_active_window_id(SOURCE_WINDOW_KEY)),
        )
        .with_step(add_and_save_window(TARGET_WINDOW_KEY))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo target-only".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(set_saved_window_origin(
            SOURCE_WINDOW_KEY,
            vec2f(100.0, 100.0),
        ))
        .with_step(set_saved_window_origin(
            TARGET_WINDOW_KEY,
            vec2f(900.0, 100.0),
        ))
        .with_step(focus_saved_window(SOURCE_WINDOW_KEY))
        .with_step(
            TestStep::new("Attach a single-tab window and then drag it back out")
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 0);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDown {
                            position: start,
                            modifiers: ModifiersState::default(),
                            click_count: 1,
                            is_first_mouse: false,
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let start = tab_center(app, source_window_id, 0);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: start + vec2f(12.0, 0.0),
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let target_window_id = *data
                        .get::<_, WindowId>(TARGET_WINDOW_KEY)
                        .expect("saved target window id should exist");
                    let target_tab_bounds = tab_bounds(app, target_window_id, 0);
                    let attach_before = tab_screen_point(
                        app,
                        target_window_id,
                        0,
                        8.0,
                        target_tab_bounds.height() / 2.0,
                    );
                    let source_local_target =
                        source_local_point_for_screen_point(app, source_window_id, attach_before);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let target_window_id = *data
                        .get::<_, WindowId>(TARGET_WINDOW_KEY)
                        .expect("saved target window id should exist");
                    let target_tab_bounds = tab_bounds(app, target_window_id, 0);
                    let below_target = tab_screen_point(
                        app,
                        target_window_id,
                        0,
                        target_tab_bounds.width() / 2.0,
                        target_tab_bounds.height() + 160.0,
                    );
                    let source_local_target =
                        source_local_point_for_screen_point(app, source_window_id, below_target);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .with_action(|app, _, data| {
                    let source_window_id = *data
                        .get::<_, WindowId>(SOURCE_WINDOW_KEY)
                        .expect("saved source window id should exist");
                    let target_window_id = *data
                        .get::<_, WindowId>(TARGET_WINDOW_KEY)
                        .expect("saved target window id should exist");
                    let target_tab_bounds = tab_bounds(app, target_window_id, 0);
                    let drop_point = tab_screen_point(
                        app,
                        target_window_id,
                        0,
                        target_tab_bounds.width() / 2.0 + 120.0,
                        target_tab_bounds.height() + 220.0,
                    );
                    let source_local_target =
                        source_local_point_for_screen_point(app, source_window_id, drop_point);
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseDragged {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                    dispatch_mouse_event(
                        app,
                        source_window_id,
                        Event::LeftMouseUp {
                            position: source_local_target,
                            modifiers: ModifiersState::default(),
                        },
                    );
                })
                .add_assertion(assert_num_windows_open(2))
                .add_assertion(assert_total_tab_count(2)),
        )
        .with_step(
            focus_saved_window(SOURCE_WINDOW_KEY)
                .add_assertion(assert_tab_count(1))
                .add_assertion(assert_focused_editor_in_tab(0)),
        )
        .with_step(focus_saved_window(TARGET_WINDOW_KEY).add_assertion(assert_tab_count(1)))
}
