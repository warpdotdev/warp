use super::*;

#[test]
fn repeated_click_pairs_preserve_their_ordinal_on_press_and_release() {
    let down = Action::MouseDown {
        button: MouseButton::Left,
        at: Some(Vector2I::new(20, 30)),
    };
    let up = Action::MouseUp {
        button: MouseButton::Left,
    };
    let mut sequence = ClickSequence::default();

    assert_eq!(sequence.next(Target::Screen, &down), 1);
    assert_eq!(sequence.next(Target::Screen, &up), 1);
    assert_eq!(sequence.next(Target::Screen, &down), 2);
    assert_eq!(sequence.next(Target::Screen, &up), 2);
    assert_eq!(sequence.next(Target::Screen, &down), 3);
    assert_eq!(sequence.next(Target::Screen, &up), 3);
}

#[test]
fn a_different_click_or_intervening_action_starts_a_new_sequence() {
    let down = Action::MouseDown {
        button: MouseButton::Left,
        at: Some(Vector2I::new(20, 30)),
    };
    let up = Action::MouseUp {
        button: MouseButton::Left,
    };
    for (target, action) in [
        (
            Target::Screen,
            Action::MouseDown {
                button: MouseButton::Left,
                at: Some(Vector2I::new(21, 30)),
            },
        ),
        (
            Target::Window {
                window_id: 7,
                pid: 12,
            },
            down.clone(),
        ),
        (
            Target::Screen,
            Action::MouseDown {
                button: MouseButton::Right,
                at: Some(Vector2I::new(20, 30)),
            },
        ),
    ] {
        let mut sequence = ClickSequence::default();
        sequence.next(Target::Screen, &down);
        sequence.next(Target::Screen, &up);
        assert_eq!(sequence.next(target, &action), 1);
    }

    let mut sequence = ClickSequence::default();
    sequence.next(Target::Screen, &down);
    sequence.next(Target::Screen, &up);
    assert_eq!(
        sequence.next(
            Target::Screen,
            &Action::MouseMove {
                to: Vector2I::new(20, 30),
            },
        ),
        0
    );
    assert_eq!(sequence.next(Target::Screen, &down), 1);
}
