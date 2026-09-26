use pathfinder_geometry::vector::vec2f;
use warpui_core::units::{IntoPixels, Pixels};

use super::{MAX_COLUMNS, MAX_ROWS, SizeInfo};

/// A pane size far larger than any real display could report (e.g. computed before real window
/// geometry is established) must not translate into an unbounded row/column count, since that
/// count directly becomes the capacity of the terminal grid's underlying allocation (APP-5808).
#[test]
fn size_info_clamps_implausibly_large_pane_size() {
    let size = SizeInfo::new(
        vec2f(1_000_000_000., 1_000_000_000.),
        1.0.into_pixels(),
        1.0.into_pixels(),
        Pixels::zero(),
        Pixels::zero(),
    );

    assert_eq!(size.columns(), MAX_COLUMNS);
    assert_eq!(size.rows(), MAX_ROWS);
}

#[test]
fn size_info_still_enforces_minimums() {
    let size = SizeInfo::new(
        vec2f(0., 0.),
        1.0.into_pixels(),
        1.0.into_pixels(),
        Pixels::zero(),
        Pixels::zero(),
    );

    assert_eq!(size.columns(), 2);
    assert_eq!(size.rows(), 1);
}
