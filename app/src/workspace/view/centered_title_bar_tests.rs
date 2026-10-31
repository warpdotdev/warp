use super::center_slot;

#[test]
fn center_is_at_row_midpoint_despite_asymmetric_sides() {
    // Wide left group (e.g. macOS traffic lights + toolbar) vs. narrow right group.
    let (x, width) = center_slot(1000., 180., 60., 336.);

    assert_eq!(width, 336.);
    assert_eq!(x + width / 2., 500.);
}

#[test]
fn center_shifts_toward_narrow_side_before_overlapping_wide_side() {
    // Midpoint placement would start at 82, inside the 180px left group.
    let (x, width) = center_slot(500., 180., 60., 336.);

    assert_eq!(width, 260.);
    assert_eq!(x, 180.);
}

#[test]
fn center_keeps_full_width_off_midpoint_when_gap_allows() {
    let (x, width) = center_slot(600., 180., 60., 336.);

    assert_eq!(width, 336.);
    // Pushed right of the 132px midpoint origin to clear the left group.
    assert_eq!(x, 180.);
}

#[test]
fn center_shrinks_to_gap_and_stays_visible_in_narrow_rows() {
    let (x, width) = center_slot(300., 180., 60., 336.);

    assert_eq!(width, 60.);
    assert_eq!(x, 180.);
}

#[test]
fn center_collapses_when_sides_fill_the_row() {
    let (x, width) = center_slot(200., 180., 60., 336.);

    assert_eq!(width, 0.);
    assert_eq!(x, 180.);
}
