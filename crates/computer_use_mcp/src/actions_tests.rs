use std::time::Duration;

use computer_use::{
    Action, Key, MouseButton, Screenshot, ScreenshotRegion, ScrollDirection, ScrollDistance,
    Vector2I,
};

use super::*;
use crate::coords::{ImageConstraints, Point, ScreenGeometry};
use crate::keys::{KeyDialect, KeyPlatform};

const SETTLE: Duration = Duration::from_millis(500);
const SHIFT: Key = Key::Keycode(0x38);
const CONTROL: Key = Key::Keycode(0x3B);
const RETURN: Key = Key::Keycode(0x24);

/// A screen small enough that screenshot coordinates equal native coordinates.
fn unscaled_geometry() -> ScreenGeometry {
    ScreenGeometry::from_screenshot(
        &Screenshot {
            width: 1024,
            height: 768,
            original_width: 1024,
            original_height: 768,
            data: Vec::new(),
            mime_type: "image/png".into(),
        },
        ImageConstraints::ANTHROPIC,
    )
}

fn context(geometry: Option<&ScreenGeometry>, screenshot_after_input: bool) -> PlanContext<'_> {
    PlanContext {
        geometry,
        key_platform: KeyPlatform::Mac,
        key_dialect: KeyDialect::Anthropic,
        settle_delay: SETTLE,
        screenshot_after_input,
    }
}

fn plan_with_screen(request: ToolRequest) -> Result<ActionPlan, ToolError> {
    let geometry = unscaled_geometry();
    plan(&request, &context(Some(&geometry), true))
}

fn point(x: i32, y: i32) -> Point {
    Point { x, y }
}

fn press_at(x: i32, y: i32) -> Action {
    Action::MouseDown {
        button: MouseButton::Left,
        at: Some(Vector2I::new(x, y)),
    }
}

const RELEASE: Action = Action::MouseUp {
    button: MouseButton::Left,
};

#[test]
fn clicks_press_and_release_per_count_then_settle() {
    let plan = plan_with_screen(ToolRequest::Click {
        at: point(10, 20),
        button: MouseButton::Left,
        count: 2,
        modifiers: None,
    })
    .unwrap();
    assert_eq!(
        plan.actions,
        vec![
            press_at(10, 20),
            RELEASE,
            press_at(10, 20),
            RELEASE,
            Action::Wait(SETTLE)
        ]
    );
    assert_eq!(plan.summary, "Left double click at (10, 20)");
    assert_eq!(plan.capture, Capture::FullScreen);
}

#[test]
fn modifiers_are_held_around_mouse_actions() {
    let plan = plan_with_screen(ToolRequest::Click {
        at: point(10, 20),
        button: MouseButton::Left,
        count: 1,
        modifiers: Some("ctrl+shift".to_owned()),
    })
    .unwrap();
    assert_eq!(
        plan.actions,
        vec![
            Action::KeyDown { key: CONTROL },
            Action::KeyDown { key: SHIFT },
            press_at(10, 20),
            RELEASE,
            Action::KeyUp { key: SHIFT },
            Action::KeyUp { key: CONTROL },
            Action::Wait(SETTLE),
        ]
    );
}

#[test]
fn click_count_is_limited() {
    for count in [0, 4] {
        let error = plan_with_screen(ToolRequest::Click {
            at: point(10, 20),
            button: MouseButton::Left,
            count,
            modifiers: None,
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "count must be between 1 and 3.");
    }
}

#[test]
fn opting_out_of_screenshots_skips_settle_and_capture() {
    let geometry = unscaled_geometry();
    let plan = plan(
        &ToolRequest::MouseMove { to: point(5, 6) },
        &context(Some(&geometry), false),
    )
    .unwrap();
    assert_eq!(
        plan.actions,
        vec![Action::MouseMove {
            to: Vector2I::new(5, 6)
        }]
    );
    assert_eq!(plan.capture, Capture::Nothing);
}

#[test]
fn drags_press_move_and_release() {
    let plan = plan_with_screen(ToolRequest::Drag {
        from: point(1, 2),
        to: point(30, 40),
        button: MouseButton::Left,
        modifiers: None,
    })
    .unwrap();
    assert_eq!(
        plan.actions,
        vec![
            press_at(1, 2),
            Action::MouseMove {
                to: Vector2I::new(30, 40)
            },
            RELEASE,
            Action::Wait(SETTLE),
        ]
    );
    assert_eq!(plan.summary, "Left click drag from (1, 2) to (30, 40)");
}

#[test]
fn scrolls_by_wheel_clicks() {
    let plan = plan_with_screen(ToolRequest::Scroll {
        at: point(7, 8),
        direction: ScrollDirection::Down,
        amount: 3,
        modifiers: None,
    })
    .unwrap();
    assert_eq!(
        plan.actions[0],
        Action::MouseWheel {
            at: Vector2I::new(7, 8),
            direction: ScrollDirection::Down,
            distance: ScrollDistance::Clicks(3),
        }
    );
    assert!(
        plan_with_screen(ToolRequest::Scroll {
            at: point(7, 8),
            direction: ScrollDirection::Down,
            amount: 0,
            modifiers: None,
        })
        .is_err()
    );
}

#[test]
fn typed_newlines_press_return() {
    let plan = plan_with_screen(ToolRequest::TypeText {
        text: "a\r\nb\n".to_owned(),
    })
    .unwrap();
    assert_eq!(
        plan.actions,
        vec![
            Action::TypeText {
                text: "a".to_owned()
            },
            Action::KeyDown { key: RETURN },
            Action::KeyUp { key: RETURN },
            Action::TypeText {
                text: "b".to_owned()
            },
            Action::KeyDown { key: RETURN },
            Action::KeyUp { key: RETURN },
            Action::Wait(SETTLE),
        ]
    );
}

#[test]
fn key_chords_repeat() {
    let plan = plan_with_screen(ToolRequest::Key {
        keys: "ctrl+a".to_owned(),
        repeat: 2,
    })
    .unwrap();
    let chord = [
        Action::KeyDown { key: CONTROL },
        Action::KeyDown {
            key: Key::Char('a'),
        },
        Action::KeyUp {
            key: Key::Char('a'),
        },
        Action::KeyUp { key: CONTROL },
    ];
    let expected: Vec<_> = chord
        .iter()
        .chain(&chord)
        .cloned()
        .chain([Action::Wait(SETTLE)])
        .collect();
    assert_eq!(plan.actions, expected);
    assert_eq!(plan.summary, "Key \"ctrl+a\" x2");
}

#[test]
fn invalid_keys_are_reported() {
    let error = plan_with_screen(ToolRequest::Key {
        keys: "hyper+a".to_owned(),
        repeat: 1,
    })
    .unwrap_err();
    assert!(error.to_string().contains("\"hyper\" is not a valid key"));
}

#[test]
fn zoom_captures_the_native_region_without_input() {
    let geometry = ScreenGeometry::from_screenshot(
        &Screenshot {
            width: 1330,
            height: 865,
            original_width: 2940,
            original_height: 1912,
            data: Vec::new(),
            mime_type: "image/png".into(),
        },
        ImageConstraints::ANTHROPIC,
    );
    let plan = plan(
        &ToolRequest::Zoom {
            top_left: point(100, 100),
            bottom_right: point(300, 200),
        },
        &context(Some(&geometry), true),
    )
    .unwrap();
    assert!(plan.actions.is_empty());
    assert_eq!(
        plan.capture,
        Capture::Region(ScreenshotRegion {
            top_left: geometry.corner_to_native(point(100, 100)).unwrap(),
            bottom_right: geometry.corner_to_native(point(300, 200)).unwrap(),
        })
    );
}

#[test]
fn empty_zoom_regions_are_rejected() {
    let error = plan_with_screen(ToolRequest::Zoom {
        top_left: point(100, 100),
        bottom_right: point(100, 200),
    })
    .unwrap_err();
    assert!(error.to_string().contains("positive width"));
}

#[test]
fn waits_are_bounded_and_skip_the_settle_delay() {
    let plan = plan_with_screen(ToolRequest::Wait { seconds: 1.5 }).unwrap();
    assert_eq!(
        plan.actions,
        vec![Action::Wait(Duration::from_millis(1500))]
    );
    assert_eq!(plan.capture, Capture::FullScreen);
    for seconds in [0.0, -1.0, 30.5, f64::NAN] {
        assert!(plan_with_screen(ToolRequest::Wait { seconds }).is_err());
    }
}

#[test]
fn mouse_down_without_coordinates_presses_at_the_cursor() {
    let plan = plan(
        &ToolRequest::MouseDown {
            at: None,
            button: MouseButton::Right,
        },
        &context(None, false),
    )
    .unwrap();
    assert_eq!(
        plan.actions,
        vec![Action::MouseDown {
            button: MouseButton::Right,
            at: None
        }]
    );
    assert_eq!(plan.summary, "Right mouse down at cursor");
}

#[test]
fn coordinates_require_a_screenshot() {
    let error = plan(
        &ToolRequest::MouseMove { to: point(1, 1) },
        &context(None, true),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Take a screenshot before using coordinates."
    );
}
