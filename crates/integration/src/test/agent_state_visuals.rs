//! End-to-end coverage for configurable CLI-agent state visuals.

use std::time::Duration;

use settings::Setting as _;
use warp::cmd_or_ctrl_shift;
use warp::features::FeatureFlag;
use warp::integration_testing::agent_state_visuals::{
    agent_tab_style_color, agent_tab_styles_are_default, agent_tab_styles_path,
    cli_agent_display_state, emit_cli_agent_notification,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::{terminal_view, workspace_view};
use warp::themes::theme::AnsiColorIdentifier;
use warp::workspace::tab_settings::{
    TabSettings, VerticalTabsDisplayGranularity, VerticalTabsTabItemMode,
};
use warpui_core::integration::{AssertionCallback, TestStep};
use warpui_core::{SingletonEntity, async_assert, async_assert_eq};

use super::{Builder, new_builder};

const ANNOTATED_DEFAULT: &str = r#"# Agent-state styling for the vertical-tabs sidebar.
# Colors are Warp's theme-aware ANSI colors: red, green, yellow, blue, magenta, cyan.
version: 1

states:
  idle:
    color: blue
    badge_size: regular
    layers: [tab_bg, badge_icon]
  processing:
    color: yellow
    badge_size: bigger
    layers: [tab_bg, badge_icon]
  success:
    color: green
    badge_size: big
    layers: [tab_bg, tab_text, badge_icon]
  needs_attention:
    color: red
    badge_size: bigger
    layers: [tab_bg, tab_text, badge_icon]

group_outline:
  # Empty disables automatic outlines. The group UUID chooses one entry stably.
  colors: [blue, magenta, cyan, green]
"#;

const PRECEDENCE_CONFIG: &str = r#"version: 1
states:
  idle:
    color: blue
    badge_size: regular
    layers: [badge_icon]
  processing:
    color: yellow
    badge_size: bigger
    layers: [tab_bg]
  success:
    color: green
    badge_size: big
    layers: [tab_text, badge_icon]
  needs_attention:
    color: red
    badge_size: bigger
    layers: [tab_bg, tab_text, badge_icon]
group_outline:
  colors: [cyan]
"#;

fn config_path() -> std::path::PathBuf {
    agent_tab_styles_path()
}

fn fast(step: TestStep) -> TestStep {
    step.set_post_step_pause(Duration::from_millis(75))
        .set_pause_on_failure(Duration::ZERO)
}

fn assert_cli_state(tab_index: usize, expected: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let view_id = terminal_view(app, window_id, tab_index, 0).id();
        let state = app.read(|ctx| cli_agent_display_state(ctx, view_id));
        async_assert_eq!(
            state,
            Some(expected),
            "tab {tab_index} should be in {expected}"
        )
    })
}

fn assert_error_toast_count(expected: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);
        workspace.read(app, |workspace, ctx| {
            async_assert_eq!(
                workspace.integration_test_agent_tab_styles_toast_count(ctx),
                expected
            )
        })
    })
}

fn notification_body(event: &str, fields: &str) -> String {
    format!(r#"{{"v":1,"agent":"claude","event":"{event}","session_id":"s1-01"{fields}}}"#)
}

fn send_notification(tab_index: usize, event: &str, fields: &str) -> TestStep {
    let body = notification_body(event, fields);
    fast(
        TestStep::new("Emit CLI-agent notification through terminal dispatcher").with_action(
            move |app, window_id, _| {
                let terminal = terminal_view(app, window_id, tab_index, 0);
                emit_cli_agent_notification(app, &terminal, body.clone());
            },
        ),
    )
}

fn set_active_pane_name(name: &'static str) -> TestStep {
    fast(
        TestStep::new("Name active agent-state pane").with_action(move |app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            let pane_group = workspace.read(app, |workspace, _| {
                workspace
                    .get_pane_group_view(workspace.active_tab_index())
                    .expect("active pane group should exist")
                    .clone()
            });
            pane_group.update(app, |pane_group, ctx| {
                let pane_id = pane_group.focused_pane_id(ctx);
                pane_group
                    .pane_by_id(pane_id)
                    .expect("focused pane should exist")
                    .pane_configuration()
                    .update(ctx, |configuration, ctx| {
                        configuration.set_custom_vertical_tabs_title(name, ctx);
                    });
            });
        }),
    )
}

fn set_vertical_mode(
    granularity: VerticalTabsDisplayGranularity,
    tab_mode: VerticalTabsTabItemMode,
) -> TestStep {
    fast(
        TestStep::new("Set vertical tab rendering mode").with_action(move |app, _, _| {
            TabSettings::handle(app).update(app, |settings, ctx| {
                settings
                    .use_vertical_tabs
                    .set_value(true, ctx)
                    .expect("vertical tabs should enable");
                settings
                    .vertical_tabs_display_granularity
                    .set_value(granularity, ctx)
                    .expect("granularity should update");
                settings
                    .vertical_tabs_tab_item_mode
                    .set_value(tab_mode, ctx)
                    .expect("tab item mode should update");
            });
        }),
    )
}

fn set_horizontal_tabs() -> TestStep {
    fast(
        TestStep::new("Render excluded horizontal-tab surface").with_action(|app, _, _| {
            TabSettings::handle(app).update(app, |settings, ctx| {
                settings
                    .use_vertical_tabs
                    .set_value(false, ctx)
                    .expect("vertical tabs should disable");
            });
        }),
    )
}

fn set_manual_colors() -> TestStep {
    fast(
        TestStep::new("Set manual tab colors for precedence").with_action(|app, window_id, _| {
            workspace_view(app, window_id).update(app, |workspace, ctx| {
                workspace.integration_test_set_all_tab_colors(AnsiColorIdentifier::Magenta, ctx);
            });
        }),
    )
}

fn group_all_tabs() -> TestStep {
    fast(
        TestStep::new("Group all agent tabs with independent group color").with_action(
            |app, window_id, _| {
                workspace_view(app, window_id).update(app, |workspace, ctx| {
                    workspace.integration_test_group_all_tabs(AnsiColorIdentifier::Cyan, ctx);
                });
            },
        ),
    )
}

pub fn test_agent_tab_styles_config_lifecycle() -> Builder {
    new_builder()
        .with_timeout(Duration::from_secs(5 * 60))
        .with_setup(|utils| {
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some("10".to_string()));
            let path = config_path();
            std::fs::create_dir_all(path.parent().expect("config parent should exist"))
                .expect("config parent should be created");
            if path.exists() {
                std::fs::remove_file(path).expect("old agent style config should be removed");
            }
        })
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
                "annotated defaults created and loaded on startup",
                |app, _| {
                    let created = std::fs::read_to_string(config_path()).ok();
                    async_assert!(
                        created.as_deref() == Some(ANNOTATED_DEFAULT)
                            && app.read(agent_tab_styles_are_default),
                        "startup should create and load the exact annotated defaults"
                    )
                },
            ),
        )
        .with_step(
            TestStep::new("Hot reload a valid agent style config")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_| {
                    std::fs::write(
                        config_path(),
                        "version: 1\nstates:\n  success:\n    color: cyan\n",
                    )
                    .expect("valid config should be written");
                })
                .add_named_assertion("valid reload applied", |app, _| {
                    async_assert_eq!(
                        app.read(|ctx| agent_tab_style_color(ctx, "success")),
                        "cyan"
                    )
                })
                .add_assertion(assert_error_toast_count(0)),
        )
        .with_step(
            TestStep::new("Reject invalid hot reload and show one error toast")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_| {
                    std::fs::write(config_path(), "version: 2\n")
                        .expect("invalid config should be written");
                })
                .add_named_assertion("last-known-good config retained", |app, _| {
                    async_assert_eq!(
                        app.read(|ctx| agent_tab_style_color(ctx, "success")),
                        "cyan"
                    )
                })
                .add_assertion(assert_error_toast_count(1)),
        )
        .with_step(
            TestStep::new("A second invalid reload replaces rather than duplicates the toast")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_| {
                    std::fs::write(config_path(), "version: 1\nunknown: true\n")
                        .expect("second invalid config should be written");
                })
                .add_named_assertion("last-known-good still retained", |app, _| {
                    async_assert_eq!(
                        app.read(|ctx| agent_tab_style_color(ctx, "success")),
                        "cyan"
                    )
                })
                .add_assertion(assert_error_toast_count(1)),
        )
        .with_step(
            TestStep::new("Delete config and immediately restore compiled defaults")
                .set_timeout(Duration::from_secs(30))
                .with_setup(|_| {
                    std::fs::remove_file(config_path()).expect("config should be deleted");
                })
                .add_named_assertion("deletion loaded defaults", |app, _| {
                    async_assert!(app.read(agent_tab_styles_are_default))
                })
                .add_assertion(assert_error_toast_count(0)),
        )
}

pub fn test_cli_agent_event_routes_and_acknowledgement() -> Builder {
    FeatureFlag::PluggableNotifications.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            send_notification(0, "session_start", "").add_named_assertion(
                "TerminalView route registered idle session",
                assert_cli_state(0, "idle"),
            ),
        )
        .with_step(
            send_notification(0, "prompt_submit", r#","query":"Route coverage""#)
                .add_named_assertion(
                    "listener route applied prompt",
                    assert_cli_state(0, "processing"),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Create another focused tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            send_notification(0, "stop", r#","query":"Route coverage""#).add_named_assertion(
                "unfocused completion remains unseen",
                assert_cli_state(0, "success"),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Focus completed session")
                .with_keystrokes(&["cmdorctrl-1"])
                .add_named_assertion(
                    "focus subscription acknowledges success",
                    assert_cli_state(0, "idle"),
                ),
        )
        .with_step(send_notification(
            0,
            "prompt_submit",
            r#","query":"Focused completion""#,
        ))
        .with_step(
            send_notification(0, "stop", r#","query":"Focused completion""#).add_named_assertion(
                "completion subscription acknowledges success",
                assert_cli_state(0, "idle"),
            ),
        )
}

pub fn vertical_tabs_state_and_badge_matrix_warposs() -> Builder {
    FeatureFlag::VerticalTabs.set_enabled(true);
    FeatureFlag::VerticalTabsSummaryMode.set_enabled(true);
    FeatureFlag::GroupedTabs.set_enabled(true);
    FeatureFlag::PluggableNotifications.set_enabled(true);

    new_builder()
        .with_timeout(Duration::from_secs(5 * 60))
        .with_setup(|utils| {
            utils.set_env(
                "WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS",
                Some("1".to_string()),
            );
            utils.set_env("WARP_CONFIG_WATCHER_DELAY_MS", Some("10".to_string()));
            let path = config_path();
            std::fs::create_dir_all(path.parent().expect("config parent should exist"))
                .expect("config parent should be created");
            std::fs::write(path, PRECEDENCE_CONFIG).expect("precedence config should be written");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(send_notification(0, "session_start", ""))
        .with_step(set_active_pane_name(
            "Idle — regular badge; manual background",
        ))
        .with_step(
            new_step_with_default_assertions("Create Processing tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(send_notification(1, "session_start", ""))
        .with_step(send_notification(
            1,
            "prompt_submit",
            r#","query":"Processing — bigger badge suppressed""#,
        ))
        .with_step(set_active_pane_name(
            "Processing — state background; badge suppressed",
        ))
        .with_step(
            new_step_with_default_assertions("Create Success tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(send_notification(2, "session_start", ""))
        .with_step(send_notification(
            2,
            "prompt_submit",
            r#","query":"Success — big badge and text""#,
        ))
        .with_step(set_active_pane_name("Success — big badge and state text"))
        .with_step(
            new_step_with_default_assertions("Create Needs Attention tab")
                .with_keystrokes(&[cmd_or_ctrl_shift("t")]),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(3))
        .with_step(send_notification(
            2,
            "stop",
            r#","query":"Success — big badge and text""#,
        ))
        .with_step(send_notification(3, "session_start", ""))
        .with_step(send_notification(
            3,
            "prompt_submit",
            r#","query":"Needs Attention — bigger badge""#,
        ))
        .with_step(send_notification(
            3,
            "permission_request",
            r#","summary":"Needs Attention — bigger badge""#,
        ))
        .with_step(set_active_pane_name(
            "Needs Attention — bigger badge and state colors",
        ))
        .with_step(
            TestStep::new("All four routed states are present")
                .set_timeout(Duration::from_secs(30))
                .add_assertion(assert_cli_state(0, "idle"))
                .add_assertion(assert_cli_state(1, "processing"))
                .add_assertion(assert_cli_state(2, "success"))
                .add_assertion(assert_cli_state(3, "needs_attention")),
        )
        .with_step(
            new_step_with_default_assertions("Focus unseen Success and acknowledge it")
                .with_keystrokes(&["cmdorctrl-3"])
                .add_assertion(assert_cli_state(2, "idle")),
        )
        .with_step(send_notification(
            2,
            "prompt_submit",
            r#","query":"Success — big badge and text""#,
        ))
        .with_step(
            new_step_with_default_assertions("Return focus to Needs Attention")
                .with_keystrokes(&["cmdorctrl-4"])
                .add_assertion(assert_cli_state(2, "processing")),
        )
        .with_step(
            send_notification(2, "stop", r#","query":"Success — big badge and text""#)
                .set_timeout(Duration::from_secs(30))
                .add_assertion(assert_cli_state(2, "success")),
        )
        .with_step(send_notification(
            3,
            "prompt_submit",
            r#","query":"Focused stop""#,
        ))
        .with_step(
            send_notification(3, "stop", r#","query":"Focused stop""#).add_named_assertion(
                "focused completion acknowledged",
                assert_cli_state(3, "idle"),
            ),
        )
        .with_step(send_notification(
            3,
            "prompt_submit",
            r#","query":"Needs Attention — bigger badge""#,
        ))
        .with_step(send_notification(
            3,
            "permission_request",
            r#","summary":"Needs Attention — bigger badge""#,
        ))
        .with_step(set_manual_colors())
        .with_step(set_horizontal_tabs())
        .with_step(
            TestStep::new("Capture excluded horizontal tabs")
                .with_take_screenshot("horizontal_tabs_excluded_surface.png"),
        )
        .with_step(set_vertical_mode(
            VerticalTabsDisplayGranularity::Panes,
            VerticalTabsTabItemMode::FocusedSession,
        ))
        .with_step(
            TestStep::new("Capture ungrouped Panes matrix")
                .with_take_screenshot("vertical_tabs_state_and_badge_matrix_warposs.png"),
        )
        .with_step(set_vertical_mode(
            VerticalTabsDisplayGranularity::Tabs,
            VerticalTabsTabItemMode::FocusedSession,
        ))
        .with_step(
            TestStep::new("Capture ungrouped Focused Session")
                .with_take_screenshot("ungrouped_focused_session.png"),
        )
        .with_step(set_vertical_mode(
            VerticalTabsDisplayGranularity::Tabs,
            VerticalTabsTabItemMode::Summary,
        ))
        .with_step(
            TestStep::new("Capture ungrouped Summary aggregation")
                .with_take_screenshot("ungrouped_summary.png"),
        )
        .with_step(group_all_tabs())
        .with_step(set_vertical_mode(
            VerticalTabsDisplayGranularity::Panes,
            VerticalTabsTabItemMode::FocusedSession,
        ))
        .with_step(
            TestStep::new("Capture grouped Panes matrix").with_take_screenshot("grouped_panes.png"),
        )
        .with_step(set_vertical_mode(
            VerticalTabsDisplayGranularity::Tabs,
            VerticalTabsTabItemMode::FocusedSession,
        ))
        .with_step(
            TestStep::new("Capture grouped Focused Session")
                .with_take_screenshot("grouped_focused_session.png"),
        )
        .with_step(set_vertical_mode(
            VerticalTabsDisplayGranularity::Tabs,
            VerticalTabsTabItemMode::Summary,
        ))
        .with_step(
            TestStep::new("Capture grouped Summary aggregation")
                .with_take_screenshot("grouped_summary.png"),
        )
        .with_step(
            TestStep::new("Verify routed states survive every render path")
                .add_assertion(assert_cli_state(0, "idle"))
                .add_assertion(assert_cli_state(1, "processing"))
                .add_assertion(assert_cli_state(2, "success"))
                .add_assertion(assert_cli_state(3, "needs_attention"))
                .add_named_assertion("four tabs remain", |app, window_id| {
                    let count = workspace_view(app, window_id)
                        .read(app, |workspace, _| workspace.tab_count());
                    async_assert_eq!(count, 4)
                }),
        )
}
