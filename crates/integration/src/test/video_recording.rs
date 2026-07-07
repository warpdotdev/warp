use std::future::Future;
use std::pin::Pin;

use pathfinder_geometry::vector::vec2f;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    assert_view_has_text_selection, clear_blocklist_to_remove_bootstrapped_blocks,
    execute_command_for_single_terminal_in_tab, execute_echo_str,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warpui_core::event::{Event, ModifiersState};
use warpui_core::integration::{TestStep, ARTIFACTS_DIR_ENV_VAR};

use crate::Builder;

/// Exercises the video recording, screenshot, and overlay annotation APIs.
///
/// This test is meant to be run manually with a real display to verify
/// that frame capture, video encoding, overlay compositing, and artifact
/// export all work end-to-end:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_video_recording
/// ```
///
/// The recording exercises every overlay type:
/// - Mouse click indicators (filled dot + expanding ring)
/// - Drag trails (selecting terminal output text)
/// - Key sequence pills (typing commands, Ctrl-C, Cmd-A / Cmd-C)
pub fn test_video_recording() -> Builder {
    Builder::new()
        .with_real_display()
        .with_on_finish(
            move |_app, _window_id, _persisted_data| -> Pin<Box<dyn Future<Output = ()> + Send>> {
                Box::pin(async move {
                    let artifacts_root = std::env::var(ARTIFACTS_DIR_ENV_VAR)
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|_| {
                            std::env::temp_dir().join("warp_integration_test_artifacts")
                        });
                    let test_dir = artifacts_root.join("test_video_recording");

                    let latest_run = std::fs::read_dir(&test_dir).ok().and_then(|entries| {
                        entries
                            .filter_map(|entry| entry.ok())
                            .filter(|entry| entry.path().is_dir())
                            .max_by_key(|entry| entry.file_name())
                            .map(|entry| entry.path())
                    });

                    let Some(run_dir) = latest_run else {
                        panic!(
                            "No timestamped run directory found under {}",
                            test_dir.display()
                        );
                    };

                    let bootstrap_png = run_dir.join("after_bootstrap.png");
                    let commands_png = run_dir.join("after_commands.png");
                    let video_mp4 = run_dir.join("recording.mp4");
                    let log_file = run_dir.join("recording.log");

                    assert!(
                        bootstrap_png.exists(),
                        "Expected after_bootstrap.png in {}",
                        run_dir.display()
                    );
                    assert!(
                        commands_png.exists(),
                        "Expected after_commands.png in {}",
                        run_dir.display()
                    );
                    assert!(
                        video_mp4.exists(),
                        "Expected recording.mp4 in {}",
                        run_dir.display()
                    );
                    assert!(
                        log_file.exists(),
                        "Expected recording.log in {}",
                        run_dir.display()
                    );

                    let video_size = std::fs::metadata(&video_mp4)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    assert!(
                        video_size > 1000,
                        "recording.mp4 is suspiciously small ({video_size} bytes)"
                    );

                    log::info!(
                        "All artifacts verified in {}. recording.mp4 = {} bytes",
                        run_dir.display(),
                        video_size
                    );
                })
            },
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            TestStep::new("Take screenshot after bootstrap")
                .with_take_screenshot("after_bootstrap.png"),
        )
        .with_step(TestStep::new("Start recording").with_start_recording())
        .with_step(execute_echo_str(0, "hello from the video test"))
        .with_step(execute_echo_str(0, "second line of output"))
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "ls".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Click in terminal")
                .with_event(Event::LeftMouseDown {
                    position: vec2f(300.0, 250.0),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseUp {
                    position: vec2f(300.0, 250.0),
                    modifiers: ModifiersState::default(),
                }),
        )
        .with_step(
            new_step_with_default_assertions("Close left panel")
                .with_click_on_saved_position("workspace:toggle_left_panel"),
        )
        .with_step(
            new_step_with_default_assertions("Start drag-select")
                .with_event_fn(|app, window_id| {
                    let presenter = app.presenter(window_id).expect("presenter");
                    let bounds = presenter
                        .borrow()
                        .position_cache()
                        .get_position("block_index:0")
                        .expect("block_index:0 position");
                    Event::LeftMouseDown {
                        position: bounds.origin(),
                        modifiers: ModifiersState::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    }
                })
                .with_event_fn(|app, window_id| {
                    let presenter = app.presenter(window_id).expect("presenter");
                    let b1 = presenter
                        .borrow()
                        .position_cache()
                        .get_position("block_index:1")
                        .expect("block_index:1 position");
                    Event::LeftMouseDragged {
                        position: b1.center(),
                        modifiers: ModifiersState::default(),
                    }
                })
                .with_event_fn(|app, window_id| {
                    let presenter = app.presenter(window_id).expect("presenter");
                    let b1 = presenter
                        .borrow()
                        .position_cache()
                        .get_position("block_index:1")
                        .expect("block_index:1 position");
                    Event::LeftMouseDragged {
                        position: b1.lower_right(),
                        modifiers: ModifiersState::default(),
                    }
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("End drag-select")
                .with_event_fn(|app, window_id| {
                    let presenter = app.presenter(window_id).expect("presenter");
                    let b1 = presenter
                        .borrow()
                        .position_cache()
                        .get_position("block_index:1")
                        .expect("block_index:1 position");
                    Event::LeftMouseUp {
                        position: b1.lower_right(),
                        modifiers: ModifiersState::default(),
                    }
                })
                .add_assertion(assert_view_has_text_selection(false)),
        )
        .with_step(TestStep::new("Copy selection").with_keystrokes(&["cmd-c"]))
        .with_step(
            TestStep::new("Type text for ctrl editing").with_input_string("hello world", None),
        )
        .with_step(
            TestStep::new("Ctrl-A, Ctrl-E, Ctrl-U")
                .with_keystrokes(&["ctrl-a", "ctrl-e", "ctrl-u"]),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "echo 'video recording test complete'".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(TestStep::new("Select all").with_keystrokes(&["cmd-a"]))
        .with_step(TestStep::new("Stop recording").with_stop_recording())
        .with_step(
            TestStep::new("Take screenshot after commands")
                .with_take_screenshot("after_commands.png"),
        )
}
