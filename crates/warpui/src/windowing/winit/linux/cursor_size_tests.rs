use super::compute_cursor_size;

#[test]
fn test_unscaled_display_keeps_logical_size() {
    assert_eq!(compute_cursor_size(24, 1.0), 24);
    assert_eq!(compute_cursor_size(48, 1.0), 48);
}

#[test]
fn test_2x_display_doubles_size() {
    // The reported bug: GNOME cursor-size 48 on a scale-2.0 display must
    // yield 96 physical pixels to match the compositor's cursor.
    assert_eq!(compute_cursor_size(48, 2.0), 96);
    assert_eq!(compute_cursor_size(24, 2.0), 48);
}

#[test]
fn test_fractional_scale_rounds_to_nearest() {
    assert_eq!(compute_cursor_size(24, 1.5), 36);
    assert_eq!(compute_cursor_size(24, 1.25), 30);
    assert_eq!(compute_cursor_size(33, 1.5), 50);
}

#[test]
fn test_scale_below_one_is_clamped() {
    assert_eq!(compute_cursor_size(24, 0.5), 24);
    assert_eq!(compute_cursor_size(24, 0.0), 24);
    assert_eq!(compute_cursor_size(24, -1.0), 24);
}
