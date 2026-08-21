use super::*;

#[test]
fn intersects_when_fully_contained() {
    let screen: SimpleRect = (0., 0., 1920., 1080.);
    let window: SimpleRect = (100., 100., 800., 600.);
    assert!(rect_intersects_any_screen(window, &[screen]));
}

#[test]
fn intersects_when_partially_overlapping_screen_edge() {
    let screen: SimpleRect = (0., 0., 1920., 1080.);
    // Window mostly off the right edge of the screen, but still partially on it.
    let window: SimpleRect = (1900., 0., 800., 600.);
    assert!(rect_intersects_any_screen(window, &[screen]));
}

#[test]
fn intersects_one_of_multiple_screens() {
    let main_screen: SimpleRect = (0., 0., 1920., 1080.);
    let external_screen: SimpleRect = (1920., 0., 2560., 1440.);
    let window: SimpleRect = (2000., 100., 800., 600.);
    assert!(rect_intersects_any_screen(
        window,
        &[main_screen, external_screen]
    ));
}

#[test]
fn does_not_intersect_when_fully_disjoint() {
    let screen: SimpleRect = (0., 0., 1920., 1080.);
    // Saved position is on a display that used to sit below the main screen; once that
    // display is disconnected the window has no reachable screen (see GH#15184).
    let window: SimpleRect = (100., -1200., 800., 600.);
    assert!(!rect_intersects_any_screen(window, &[screen]));
}

#[test]
fn does_not_intersect_when_only_touching_edges() {
    let screen: SimpleRect = (0., 0., 1920., 1080.);
    // The window starts exactly where the screen ends: zero-area overlap, so no visible
    // pixels would actually land on the screen.
    let window: SimpleRect = (1920., 0., 800., 600.);
    assert!(!rect_intersects_any_screen(window, &[screen]));
}

#[test]
fn no_screens_never_intersects() {
    let window: SimpleRect = (100., 100., 800., 600.);
    assert!(!rect_intersects_any_screen(window, &[]));
}

#[test]
fn degenerate_screen_or_window_never_intersects() {
    let zero_size_screen: SimpleRect = (0., 0., 0., 0.);
    let window: SimpleRect = (0., 0., 800., 600.);
    assert!(!rect_intersects_any_screen(window, &[zero_size_screen]));

    let screen: SimpleRect = (0., 0., 1920., 1080.);
    let zero_size_window: SimpleRect = (100., 100., 0., 0.);
    assert!(!rect_intersects_any_screen(zero_size_window, &[screen]));
}
