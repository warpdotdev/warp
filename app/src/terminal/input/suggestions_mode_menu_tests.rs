use pathfinder_geometry::vector::vec2f;

use super::*;

#[test]
fn suggestions_bounds_clamp_min_for_small_windows() {
    let small_window = vec2f(27.2, 27.2);
    let (width_min, width_max) = suggestions_width_bounds(small_window);
    let (height_min, height_max) = suggestions_height_bounds(small_window);

    assert!(width_max >= width_min);
    assert_eq!(width_max, width_min);
    assert!(height_max >= height_min);
    assert_eq!(height_max, height_min);
}
