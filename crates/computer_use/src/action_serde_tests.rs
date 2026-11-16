use crate::{Action, MouseButton, Vector2I};

#[test]
fn mouse_down_round_trips_with_and_without_a_position() {
    for action in [
        Action::MouseDown {
            button: MouseButton::Left,
            at: Some(Vector2I::new(3, 4)),
        },
        Action::MouseDown {
            button: MouseButton::Right,
            at: None,
        },
    ] {
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), action);
    }
}

#[test]
fn mouse_down_deserializes_legacy_shape_and_omitted_position() {
    let legacy: Action =
        serde_json::from_str(r#"{"MouseDown":{"button":"Left","at":{"x":3,"y":4}}}"#).unwrap();
    assert_eq!(
        legacy,
        Action::MouseDown {
            button: MouseButton::Left,
            at: Some(Vector2I::new(3, 4)),
        }
    );

    let omitted: Action = serde_json::from_str(r#"{"MouseDown":{"button":"Left"}}"#).unwrap();
    assert_eq!(
        omitted,
        Action::MouseDown {
            button: MouseButton::Left,
            at: None,
        }
    );
}
