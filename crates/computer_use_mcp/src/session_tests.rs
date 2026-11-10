use computer_use::{Action, Key, MouseButton, Vector2I};

use super::*;
use crate::RecordingDestination;
use crate::coords::{Point, ScreenGeometry};
use crate::keys::{KeyDialect, parse_chord};
use crate::test_util::{FakeActorLog, FakeRecorder, synthetic_screenshot, test_config};

fn session(log: &FakeActorLog) -> Session {
    Session::new(test_config(), log.factory(), Box::new(FakeRecorder))
}

fn full_screen_geometry() -> ScreenGeometry {
    let constraints = test_config().image_constraints;
    ScreenGeometry::from_screenshot(
        &synthetic_screenshot(constraints.screenshot_params(None)),
        constraints,
    )
}

fn click(x: i32, y: i32) -> ToolRequest {
    ToolRequest::Click {
        at: Point { x, y },
        button: MouseButton::Left,
        count: 1,
        modifiers: None,
    }
}

#[tokio::test]
async fn first_coordinate_tool_captures_the_screen_to_learn_its_geometry() {
    let log = FakeActorLog::default();
    let mut session = session(&log);
    session.handle(click(100, 200), true).await.unwrap();

    let batches = log.batches();
    assert_eq!(batches.len(), 2);
    assert!(batches[0].actions.is_empty());
    assert!(batches[0].screenshot_params.is_some());
    assert_eq!(
        batches[1].actions[0].action,
        Action::MouseDown {
            button: MouseButton::Left,
            at: Some(
                full_screen_geometry()
                    .to_native(Point { x: 100, y: 200 })
                    .unwrap()
            ),
        }
    );
}

#[tokio::test]
async fn screenshots_describe_their_size_and_return_the_image() {
    let log = FakeActorLog::default();
    let output = session(&log)
        .handle(ToolRequest::Screenshot, true)
        .await
        .unwrap();
    let (width, height) = full_screen_geometry().image_size();
    assert!(
        output.text.starts_with(&format!(
            "Screenshot is {width}x{height} px (native screen 2940x1912 px)"
        )),
        "{}",
        output.text
    );
    assert!(output.image.is_some());
}

#[tokio::test]
async fn input_returns_a_screenshot_unless_opted_out() {
    let log = FakeActorLog::default();
    let mut session = session(&log);
    session.handle(ToolRequest::Screenshot, true).await.unwrap();

    let with_screenshot = session.handle(click(1, 1), true).await.unwrap();
    assert!(with_screenshot.image.is_some());
    assert_eq!(with_screenshot.text, "Left click at (1, 1).");

    let without_screenshot = session.handle(click(1, 1), false).await.unwrap();
    assert!(without_screenshot.image.is_none());
    assert!(log.batches().last().unwrap().screenshot_params.is_none());
}

#[tokio::test]
async fn zoom_does_not_change_the_coordinate_space() {
    let log = FakeActorLog::default();
    let mut session = session(&log);
    session.handle(ToolRequest::Screenshot, true).await.unwrap();
    let zoom = session
        .handle(
            ToolRequest::Zoom {
                top_left: Point { x: 0, y: 0 },
                bottom_right: Point { x: 50, y: 50 },
            },
            false,
        )
        .await
        .unwrap();
    assert!(
        zoom.text
            .contains("coordinates still refer to the last full screenshot")
    );
    assert!(
        log.batches()
            .last()
            .unwrap()
            .screenshot_params
            .unwrap()
            .region
            .is_some(),
        "zoom should capture a region"
    );

    session.handle(click(100, 200), false).await.unwrap();
    assert_eq!(
        log.batches().last().unwrap().actions[0].action,
        Action::MouseDown {
            button: MouseButton::Left,
            at: Some(
                full_screen_geometry()
                    .to_native(Point { x: 100, y: 200 })
                    .unwrap()
            ),
        }
    );
}

#[tokio::test]
async fn the_actor_is_reused_across_calls() {
    let log = FakeActorLog::default();
    let mut session = session(&log);
    session.handle(ToolRequest::Screenshot, true).await.unwrap();
    session.handle(click(1, 1), true).await.unwrap();
    session
        .handle(
            ToolRequest::TypeText {
                text: "hi".to_owned(),
            },
            true,
        )
        .await
        .unwrap();
    assert_eq!(log.actors_created(), 1);
}

#[tokio::test]
async fn failed_input_releases_pressed_keys() {
    let log = FakeActorLog::default();
    let mut session = session(&log);
    log.fail_next_input("display went away");
    let error = session
        .handle(
            ToolRequest::Key {
                keys: "ctrl+c".to_owned(),
                repeat: 1,
            },
            true,
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Computer use failed: display went away");

    let release = log.batches().last().unwrap().clone();
    let released: Vec<_> = release
        .actions
        .into_iter()
        .map(|targeted| targeted.action)
        .collect();
    let control =
        parse_chord("ctrl", KeyPlatform::current(), KeyDialect::Anthropic).unwrap()[0].clone();
    assert_eq!(
        released,
        vec![
            Action::KeyUp {
                key: Key::Char('c')
            },
            Action::KeyUp { key: control },
        ]
    );
}

#[tokio::test]
async fn cursor_position_is_reported_in_screenshot_coordinates() {
    let log = FakeActorLog::default();
    let output = session(&log)
        .handle(ToolRequest::CursorPosition, false)
        .await
        .unwrap();
    let expected = full_screen_geometry().to_model(Vector2I::new(1470, 956));
    assert_eq!(
        output.text,
        serde_json::json!({ "x": expected.x, "y": expected.y }).to_string()
    );
    assert!(output.image.is_none());
}

fn recording_session(log: &FakeActorLog, recordings: &std::path::Path) -> Session {
    let config = ComputerUseMcpConfig {
        recording: Some(RecordingDestination::Directory(recordings.to_owned())),
        ..test_config()
    };
    Session::new(config, log.factory(), Box::new(FakeRecorder))
}

fn start_recording() -> SessionCommand {
    SessionCommand::StartRecording {
        title: Some("Demo".to_owned()),
        description: None,
    }
}

fn stop_recording(recording_id: &str, discard: bool) -> SessionCommand {
    SessionCommand::StopRecording {
        recording_id: recording_id.to_owned(),
        discard,
    }
}

#[tokio::test]
async fn recording_requires_a_destination() {
    let log = FakeActorLog::default();
    let error = session(&log).execute(start_recording()).await.unwrap_err();
    assert_eq!(error.to_string(), "Recording is disabled.");
}

#[tokio::test]
async fn input_is_logged_while_recording() {
    let log = FakeActorLog::default();
    let recordings = tempfile::tempdir().unwrap();
    let mut session = recording_session(&log, recordings.path());

    let started = session.execute(start_recording()).await.unwrap();
    assert_eq!(
        started.text,
        serde_json::json!({ "recording_id": "recording-1", "width_px": 1920, "height_px": 1080 })
            .to_string()
    );
    let already_started = session.execute(start_recording()).await.unwrap_err();
    assert!(already_started.to_string().contains("already in progress"));

    session.handle(ToolRequest::Screenshot, true).await.unwrap();
    session
        .handle(
            ToolRequest::Key {
                keys: "ctrl+s".to_owned(),
                repeat: 1,
            },
            true,
        )
        .await
        .unwrap();
    log.fail_next_input("display went away");
    session.handle(click(1, 1), true).await.unwrap_err();

    let recorded: Vec<_> = log.batches().iter().map(|batch| batch.recorded).collect();
    // The screenshot isn't input, so only the key press and the click are recorded.
    assert_eq!(recorded, [false, true, true]);
    let actions = session.active_recording().unwrap().actions();
    let [entry] = actions else {
        panic!("only the successful key press should be logged, got {actions:?}");
    };
    assert_eq!(entry.labels, ["ctrl+s"]);
}

#[tokio::test]
async fn stopping_requires_the_recording_id_and_can_discard() {
    let log = FakeActorLog::default();
    let recordings = tempfile::tempdir().unwrap();
    let mut session = recording_session(&log, recordings.path());
    session.execute(start_recording()).await.unwrap();

    let wrong_id = session
        .execute(stop_recording("recording-7", false))
        .await
        .unwrap_err();
    assert_eq!(
        wrong_id.to_string(),
        "No recording has ID \"recording-7\"; the recording in progress is \"recording-1\"."
    );
    assert!(session.active_recording().is_some());

    let discarded = session
        .execute(stop_recording("recording-1", true))
        .await
        .unwrap();
    assert_eq!(discarded.text, r#"{"status":"discarded"}"#);
    assert!(session.active_recording().is_none());
    assert_eq!(
        session
            .execute(stop_recording("recording-1", false))
            .await
            .unwrap_err()
            .to_string(),
        "No recording is in progress."
    );
}

#[tokio::test]
async fn failing_to_stop_a_recording_ends_it() {
    let log = FakeActorLog::default();
    let recordings = tempfile::tempdir().unwrap();
    let mut session = recording_session(&log, recordings.path());
    session.execute(start_recording()).await.unwrap();

    let error = session
        .execute(stop_recording("recording-1", false))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to stop the recording: Recording failed to finalize: test recordings have no video"
    );
    assert!(session.active_recording().is_none());
    assert!(session.finish_recording().await.is_none());
}
