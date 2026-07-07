use super::*;

#[test]
fn test_scroll_delta_for_axis_fully_into_view() {
    let mode = ScrollToPositionMode::FullyIntoView;
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Horizontal,
            RectF::new(vec2f(100., 0.), vec2f(250., 250.)),
            RectF::new(vec2f(400., 50.), vec2f(50., 50.)),
            mode,
        ),
        100.
    );
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Horizontal,
            RectF::new(vec2f(200., 0.), vec2f(250., 250.)),
            RectF::new(vec2f(100., 50.), vec2f(50., 50.)),
            mode,
        ),
        -100.
    );
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Horizontal,
            RectF::new(vec2f(100., 0.), vec2f(250., 250.)),
            RectF::new(vec2f(325., 50.), vec2f(50., 50.)),
            mode,
        ),
        25.
    );
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Horizontal,
            RectF::new(vec2f(100., 0.), vec2f(250., 250.)),
            RectF::new(vec2f(150., 50.), vec2f(50., 50.)),
            mode,
        ),
        0.
    );
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Horizontal,
            RectF::new(vec2f(100., 0.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 50.), vec2f(350., 50.)),
            mode,
        ),
        0.
    );
}

#[test]
fn test_scroll_delta_for_axis_top_into_view() {
    let mode = ScrollToPositionMode::TopIntoView;

    // --- Element LARGER than the viewport ---

    // Element taller than viewport, below viewport: align top with
    // viewport top.
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 100.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 400.), vec2f(50., 300.)),
            mode,
        ),
        300.
    );

    // Element taller than viewport, above viewport: align top with
    // viewport top.
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 200.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 100.), vec2f(50., 300.)),
            mode,
        ),
        -100.
    );

    // Element taller than viewport, top at viewport top: align top
    // (delta = 0).
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 100.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 100.), vec2f(50., 300.)),
            mode,
        ),
        0.
    );

    // Element taller than viewport, top visible but bottom extends
    // past: align top with viewport top (shows max content from top).
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 100.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 200.), vec2f(50., 300.)),
            mode,
        ),
        100.
    );

    // Element taller than viewport, spans entire viewport (top above,
    // bottom below): align top with viewport top.
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 100.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 50.), vec2f(50., 400.)),
            mode,
        ),
        -50.
    );

    // --- Element FITS in the viewport (delegates to FullyIntoView) ---

    // Small element below viewport: scroll down (bottom to viewport
    // bottom).
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 100.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 400.), vec2f(50., 50.)),
            mode,
        ),
        100.
    );

    // Small element above viewport: scroll up (top to viewport top).
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 200.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 100.), vec2f(50., 50.)),
            mode,
        ),
        -100.
    );

    // Small element fully visible: no scroll.
    assert_eq!(
        scroll_delta_for_axis(
            Axis::Vertical,
            RectF::new(vec2f(0., 100.), vec2f(250., 250.)),
            RectF::new(vec2f(50., 150.), vec2f(50., 50.)),
            mode,
        ),
        0.
    );
}
