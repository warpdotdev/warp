use std::cmp::Ordering;

use super::natural_cmp;

#[test]
fn orders_numbers_by_value() {
    let mut names = ["L10", "L2", "L1", "L12", "L3", "L11"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, ["L1", "L2", "L3", "L10", "L11", "L12"]);
}

#[test]
fn numeric_aware_within_mixed_names() {
    assert_eq!(natural_cmp("img2.png", "img10.png"), Ordering::Less);

    let mut names = ["img12.png", "img2.png", "img1.png", "img10.png"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, ["img1.png", "img2.png", "img10.png", "img12.png"]);
}

#[test]
fn long_digit_runs_do_not_overflow() {
    let forty_ones = "1".repeat(40);
    let forty_twos = "2".repeat(40);
    assert_eq!(natural_cmp(&forty_ones, &forty_twos), Ordering::Less);
}

#[test]
fn equal_numeric_value_with_leading_zeros_is_deterministic() {
    assert_eq!(natural_cmp("file2", "file02"), Ordering::Less);
    assert_eq!(natural_cmp("file02", "file2"), Ordering::Greater);
}

#[test]
fn non_numeric_falls_back_to_char_order() {
    assert_eq!(natural_cmp("apple", "banana"), Ordering::Less);
    assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
    assert_eq!(natural_cmp("same", "same"), Ordering::Equal);
}

#[test]
fn handles_multibyte_chars() {
    assert_eq!(natural_cmp("café", "cafz"), "café".cmp("cafz"));
    assert_eq!(natural_cmp("café2", "café10"), Ordering::Less);
    assert_eq!(natural_cmp("café", "café"), Ordering::Equal);
}
