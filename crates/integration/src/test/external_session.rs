use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use warp::integration_testing::external_session::{
    TestExternalSessionOptions, open_external_session_for_test,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    assert_active_block_received_precmd,
    assert_long_running_block_executing_for_single_terminal_in_tab, assert_terminal_bootstrapped,
    execute_command_for_single_terminal_in_tab, validate_block_output,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::workspace_view;
use warp::integration_testing::window::assert_num_windows_open;
use warp::integration_testing::workspace::{assert_focused_tab_index, assert_tab_count};
use warp_cli::ExternalCommandSpec;
use warpui_core::async_assert_eq;
use warpui_core::integration::TestStep;

use super::new_builder;
use crate::Builder;

pub fn test_external_session_opens_tab_and_runs_command_with_env() -> Builder {
    let proj_dir: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let proj_dir_setup = Arc::clone(&proj_dir);
    let proj_dir_action = Arc::clone(&proj_dir);
    let proj_dir_assert = Arc::clone(&proj_dir);
    let proj_dir_cleanup = proj_dir;

    new_builder()
        .with_setup(move |utils| {
            let dir = utils.test_dir().join("warp_ext_tab");
            std::fs::create_dir_all(&dir).expect("Should create temporary project directory");
            *proj_dir_setup.lock() = Some(dir);
        })
        .with_cleanup(move |_| {
            if let Some(dir) = proj_dir_cleanup.lock().take() {
                let _ = std::fs::remove_dir_all(dir);
            }
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open external session in a new tab")
                .with_action(move |app, _, _| {
                    let working_directory = proj_dir_action
                        .lock()
                        .clone()
                        .expect("project directory should be initialized in setup");
                    open_external_session_for_test(
                        TestExternalSessionOptions {
                            working_directory: Some(working_directory),
                            command: Some(ExternalCommandSpec::Argv(vec![
                                "sh".to_owned(),
                                "-c".to_owned(),
                                "pwd && printf ':%s' \"$VSCODE_EXT_DEBUG_VAR\"".to_owned(),
                            ])),
                            env_vars: vec![(
                                OsString::from("VSCODE_EXT_DEBUG_VAR"),
                                OsString::from("vscode_ok"),
                            )],
                            title: Some("VS Code Debug".to_owned()),
                            new_window: false,
                            wait_after_command: Some(true),
                        },
                        app,
                    );
                })
                .add_assertion(assert_num_windows_open(1))
                .add_assertion(assert_tab_count(2))
                .add_assertion(assert_focused_tab_index(1)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions(
                "Verify external session working directory, env var, and title",
            )
            .add_named_assertion(
                "Active block received precmd",
                assert_active_block_received_precmd(1, 0),
            )
            .add_named_assertion(
                "Block prints working dir and env var",
                move |app, window_id| {
                    let dir = proj_dir_assert
                        .lock()
                        .clone()
                        .expect("project directory should be initialized in setup");
                    let canonical = std::fs::canonicalize(&dir).unwrap_or(dir);
                    let expected = format!("{}\n:vscode_ok", canonical.display());
                    validate_block_output(&expected, 1, 0, window_id, app)
                },
            )
            .add_named_assertion("Tab has custom external title", |app, window_id| {
                let workspace = workspace_view(app, window_id);
                let pane_group = workspace.read(app, |ws, _| {
                    ws.get_pane_group_view(1)
                        .expect("second tab should exist")
                        .clone()
                });
                pane_group.read(app, |pg, ctx| {
                    async_assert_eq!(pg.display_title(ctx), "VS Code Debug")
                })
            }),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "echo still_alive".to_owned(),
            ExpectedExitStatus::Success,
            "still_alive".to_owned(),
        ))
}

pub fn test_external_session_new_window_runs_command() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open external session in a new window")
                .with_action(|app, _, _| {
                    open_external_session_for_test(
                        TestExternalSessionOptions {
                            working_directory: None,
                            command: Some(ExternalCommandSpec::Argv(vec![
                                "sh".to_owned(),
                                "-c".to_owned(),
                                "printf '%s' \"$VSCODE_EXT_WIN_VAR\"".to_owned(),
                            ])),
                            env_vars: vec![(
                                OsString::from("VSCODE_EXT_WIN_VAR"),
                                OsString::from("window_ok"),
                            )],
                            title: Some("Debug Win".to_owned()),
                            new_window: true,
                            wait_after_command: Some(true),
                        },
                        app,
                    );
                })
                .add_assertion(assert_num_windows_open(2)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Verify external session output in new window")
                .add_named_assertion(
                    "Active block received precmd",
                    assert_active_block_received_precmd(0, 0),
                )
                .add_named_assertion("Block prints injected env var", |app, window_id| {
                    validate_block_output("window_ok", 0, 0, window_id, app)
                })
                .add_named_assertion("Tab has custom external title", |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    let pane_group = workspace.read(app, |ws, _| {
                        ws.get_pane_group_view(0)
                            .expect("first tab in new window should exist")
                            .clone()
                    });
                    pane_group.read(app, |pg, ctx| {
                        async_assert_eq!(pg.display_title(ctx), "Debug Win")
                    })
                }),
        )
}

pub fn test_external_session_reuses_idle_debug_tab() -> Builder {
    let dirs: Arc<Mutex<Option<(PathBuf, PathBuf)>>> = Arc::new(Mutex::new(None));
    let dirs_setup = Arc::clone(&dirs);
    let dirs_action_1 = Arc::clone(&dirs);
    let dirs_assert_1 = Arc::clone(&dirs);
    let dirs_action_2 = Arc::clone(&dirs);
    let dirs_assert_2 = Arc::clone(&dirs);
    let dirs_cleanup = dirs;

    new_builder()
        .with_setup(move |utils| {
            let dir_one = utils.test_dir().join("warp_ext_reuse_1");
            let dir_two = utils.test_dir().join("warp_ext_reuse_2");
            std::fs::create_dir_all(&dir_one).expect("Should create first debug directory");
            std::fs::create_dir_all(&dir_two).expect("Should create second debug directory");
            *dirs_setup.lock() = Some((dir_one, dir_two));
        })
        .with_cleanup(move |_| {
            if let Some((dir_one, dir_two)) = dirs_cleanup.lock().take() {
                let _ = std::fs::remove_dir_all(dir_one);
                let _ = std::fs::remove_dir_all(dir_two);
            }
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open first external session tab")
                .with_action(move |app, _, _| {
                    let (dir_one, _) = dirs_action_1
                        .lock()
                        .clone()
                        .expect("directories should be initialized in setup");
                    open_external_session_for_test(
                        TestExternalSessionOptions {
                            working_directory: Some(dir_one),
                            command: Some(ExternalCommandSpec::Argv(vec![
                                "sh".to_owned(),
                                "-c".to_owned(),
                                "pwd && printf ':%s' \"$VSCODE_EXT_RUN\"".to_owned(),
                            ])),
                            env_vars: vec![(
                                OsString::from("VSCODE_EXT_RUN"),
                                OsString::from("first"),
                            )],
                            title: Some("Debug One".to_owned()),
                            new_window: false,
                            wait_after_command: Some(true),
                        },
                        app,
                    );
                })
                .add_assertion(assert_num_windows_open(1))
                .add_assertion(assert_tab_count(2))
                .add_assertion(assert_focused_tab_index(1)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Wait for first debug run to finish cleanly")
                .add_named_assertion(
                    "First block received precmd",
                    assert_active_block_received_precmd(1, 0),
                )
                .add_named_assertion("First run output matches dir_one", move |app, window_id| {
                    let (dir_one, _) = dirs_assert_1
                        .lock()
                        .clone()
                        .expect("directories should be initialized in setup");
                    let canonical = std::fs::canonicalize(&dir_one).unwrap_or(dir_one);
                    let expected = format!("{}\n:first", canonical.display());
                    validate_block_output(&expected, 1, 0, window_id, app)
                }),
        )
        .with_step(
            TestStep::new("Open second external session and reuse idle debug tab")
                .with_action(move |app, _, _| {
                    let (_, dir_two) = dirs_action_2
                        .lock()
                        .clone()
                        .expect("directories should be initialized in setup");
                    open_external_session_for_test(
                        TestExternalSessionOptions {
                            working_directory: Some(dir_two),
                            command: Some(ExternalCommandSpec::Argv(vec![
                                "sh".to_owned(),
                                "-c".to_owned(),
                                "pwd && printf ':%s' \"$VSCODE_EXT_RUN\"".to_owned(),
                            ])),
                            env_vars: vec![(
                                OsString::from("VSCODE_EXT_RUN"),
                                OsString::from("second"),
                            )],
                            title: Some("Debug Two".to_owned()),
                            new_window: false,
                            wait_after_command: Some(true),
                        },
                        app,
                    );
                })
                .add_assertion(assert_num_windows_open(1))
                .add_assertion(assert_tab_count(2))
                .add_assertion(assert_focused_tab_index(1)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Verify reused tab runs second command and title")
                .add_named_assertion(
                    "Second block received precmd",
                    assert_active_block_received_precmd(1, 0),
                )
                .add_named_assertion(
                    "Second run output matches dir_two",
                    move |app, window_id| {
                        let (_, dir_two) = dirs_assert_2
                            .lock()
                            .clone()
                            .expect("directories should be initialized in setup");
                        let canonical = std::fs::canonicalize(&dir_two).unwrap_or(dir_two);
                        let expected = format!("{}\n:second", canonical.display());
                        validate_block_output(&expected, 1, 0, window_id, app)
                    },
                )
                .add_named_assertion("Reused tab updated title", |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    let pane_group = workspace.read(app, |ws, _| {
                        ws.get_pane_group_view(1)
                            .expect("reused tab should exist")
                            .clone()
                    });
                    pane_group.read(app, |pg, ctx| {
                        async_assert_eq!(pg.display_title(ctx), "Debug Two")
                    })
                }),
        )
}

pub fn test_external_session_opens_new_tab_when_debug_tab_is_busy() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open first external session running a long-lived command")
                .with_action(|app, _, _| {
                    open_external_session_for_test(
                        TestExternalSessionOptions {
                            working_directory: None,
                            command: Some(ExternalCommandSpec::Argv(vec![
                                "sh".to_owned(),
                                "-c".to_owned(),
                                "printf 'started\\n'; sleep 30".to_owned(),
                            ])),
                            env_vars: vec![],
                            title: Some("Debug Busy".to_owned()),
                            new_window: false,
                            wait_after_command: Some(true),
                        },
                        app,
                    );
                })
                .add_assertion(assert_tab_count(2))
                .add_assertion(assert_focused_tab_index(1)),
        )
        .with_step(
            TestStep::new("Wait until first debug tab is actively executing")
                .add_named_assertion(
                    "First debug tab is bootstrapped",
                    assert_terminal_bootstrapped(1, 0),
                )
                .add_named_assertion(
                    "First debug tab is running long command",
                    assert_long_running_block_executing_for_single_terminal_in_tab(true, 1),
                ),
        )
        .with_step(
            TestStep::new("Open second external session in a fresh tab while first is busy")
                .with_action(|app, _, _| {
                    open_external_session_for_test(
                        TestExternalSessionOptions {
                            working_directory: None,
                            command: Some(ExternalCommandSpec::Argv(vec![
                                "sh".to_owned(),
                                "-c".to_owned(),
                                "printf 'second_done'".to_owned(),
                            ])),
                            env_vars: vec![],
                            title: Some("Debug Concurrent".to_owned()),
                            new_window: false,
                            wait_after_command: Some(true),
                        },
                        app,
                    );
                })
                .add_assertion(assert_tab_count(3))
                .add_assertion(assert_focused_tab_index(2)),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions(
                "Verify second debug tab completed without closing first",
            )
            .add_named_assertion(
                "Second debug tab received precmd",
                assert_active_block_received_precmd(2, 0),
            )
            .add_named_assertion("Second debug tab printed output", |app, window_id| {
                validate_block_output("second_done", 2, 0, window_id, app)
            })
            .add_named_assertion("All three tabs remain open", |app, window_id| {
                assert_tab_count(3)(app, window_id)
            })
            .add_named_assertion(
                "Second debug tab has custom title",
                |app, window_id| {
                    let workspace = workspace_view(app, window_id);
                    let pane_group = workspace.read(app, |ws, _| {
                        ws.get_pane_group_view(2)
                            .expect("third tab should exist")
                            .clone()
                    });
                    pane_group.read(app, |pg, ctx| {
                        async_assert_eq!(pg.display_title(ctx), "Debug Concurrent")
                    })
                },
            ),
        )
}
