use super::*;

#[test]
fn comment_list_bounds_clamp_min_for_small_windows() {
    let (min, max) = comment_list_bounds(vec2f(100.0, 34.0));

    assert!(max >= min);
    assert_eq!(max, min);
}
