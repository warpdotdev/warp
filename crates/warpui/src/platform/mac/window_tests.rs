use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::screen_index_containing_center;

/// A built-in display as the primary screen, with a taller external display to its right.
fn screen_frames() -> [NSRect; 2] {
    [
        NSRect::new(NSPoint::new(0., 0.), NSSize::new(1512., 982.)),
        NSRect::new(NSPoint::new(1512., -166.), NSSize::new(2560., 1440.)),
    ]
}

#[test]
fn resolves_window_filling_secondary_display() {
    let screens = screen_frames();

    assert_eq!(
        screen_index_containing_center(&screens, screens[1]),
        Some(1)
    );
}

#[test]
fn resolves_window_on_primary_display() {
    let screens = screen_frames();
    let window = NSRect::new(NSPoint::new(100., 100.), NSSize::new(800., 600.));

    assert_eq!(screen_index_containing_center(&screens, window), Some(0));
}

#[test]
fn resolves_window_straddling_displays_by_its_center() {
    let screens = screen_frames();
    let window = NSRect::new(NSPoint::new(1112., 100.), NSSize::new(800., 600.));

    assert_eq!(screen_index_containing_center(&screens, window), Some(1));
}

#[test]
fn rejects_empty_window_frame() {
    assert_eq!(
        screen_index_containing_center(&screen_frames(), NSRect::ZERO),
        None
    );
}

#[test]
fn rejects_window_outside_every_display() {
    let window = NSRect::new(NSPoint::new(-4000., -4000.), NSSize::new(800., 600.));

    assert_eq!(
        screen_index_containing_center(&screen_frames(), window),
        None
    );
}
