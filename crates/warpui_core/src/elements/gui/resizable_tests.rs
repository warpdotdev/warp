use super::*;

#[test]
fn clamped_resize_tracks_cursor_when_drag_direction_reverses() {
    let cases = [
        (
            "right",
            DragBarSide::Right,
            vec2f(100., 0.),
            vec2f(120., 0.),
            vec2f(115., 0.),
        ),
        (
            "left",
            DragBarSide::Left,
            vec2f(0., 0.),
            vec2f(-20., 0.),
            vec2f(-15., 0.),
        ),
        (
            "bottom",
            DragBarSide::Bottom,
            vec2f(0., 100.),
            vec2f(0., 120.),
            vec2f(0., 115.),
        ),
        (
            "top",
            DragBarSide::Top,
            vec2f(0., 0.),
            vec2f(0., -20.),
            vec2f(0., -15.),
        ),
    ];

    for (name, side, start, past_max, reversed) in cases {
        let mut state = ResizableState::new(100.);
        state.bounds = Some((50., 110.));
        state.begin_resizing(start);

        state.check_for_resize(past_max, Some(Vector2F::zero()), side);
        assert_eq!(state.size(), 110., "{name} drag should clamp at max");

        state.check_for_resize(reversed, Some(Vector2F::zero()), side);
        assert_eq!(
            state.size(),
            105.,
            "{name} drag should respond immediately after reversing"
        );
    }
}
