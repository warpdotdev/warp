use computer_use::{Action, Key, MouseButton, Vector2I};

use super::*;
use crate::coords::{Point, ScreenGeometry};
use crate::keys::{KeyDialect, parse_chord};
use crate::test_util::{FakeActorLog, synthetic_screenshot, test_config};

fn session(log: &FakeActorLog) -> Session {
    Session::new(test_config(), log.factory())
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
