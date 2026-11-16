use super::CustomWeight;

#[test]
fn from_css_numeric_maps_named_steps() {
    assert_eq!(
        CustomWeight::from_css_numeric(100),
        Some(CustomWeight::Thin)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(200),
        Some(CustomWeight::ExtraLight)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(300),
        Some(CustomWeight::Light)
    );
    assert_eq!(CustomWeight::from_css_numeric(400), None);
    assert_eq!(
        CustomWeight::from_css_numeric(500),
        Some(CustomWeight::Medium)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(600),
        Some(CustomWeight::Semibold)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(700),
        Some(CustomWeight::Bold)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(800),
        Some(CustomWeight::ExtraBold)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(900),
        Some(CustomWeight::Black)
    );
}

#[test]
fn from_css_numeric_rounds_to_nearest_hundred() {
    // Off-scale values round to the nearest named step.
    assert_eq!(
        CustomWeight::from_css_numeric(340),
        Some(CustomWeight::Light)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(660),
        Some(CustomWeight::Bold)
    );
    // Values rounding to 400 have no custom weight.
    assert_eq!(CustomWeight::from_css_numeric(380), None);
    assert_eq!(CustomWeight::from_css_numeric(449), None);
}

#[test]
fn from_css_numeric_clamps_out_of_range() {
    assert_eq!(CustomWeight::from_css_numeric(1), Some(CustomWeight::Thin));
    assert_eq!(CustomWeight::from_css_numeric(50), Some(CustomWeight::Thin));
    assert_eq!(
        CustomWeight::from_css_numeric(1000),
        Some(CustomWeight::Black)
    );
}

#[test]
fn from_css_numeric_does_not_overflow_on_extreme_input() {
    // Values far outside the CSS 1..=1000 range must not panic (debug) or
    // wrap around (release) when added to before clamping.
    assert_eq!(
        CustomWeight::from_css_numeric(i32::MAX),
        Some(CustomWeight::Black)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(i32::MIN),
        Some(CustomWeight::Thin)
    );
    assert_eq!(CustomWeight::from_css_numeric(0), Some(CustomWeight::Thin));
    assert_eq!(CustomWeight::from_css_numeric(-5), Some(CustomWeight::Thin));
    assert_eq!(
        CustomWeight::from_css_numeric(1000),
        Some(CustomWeight::Black)
    );
    assert_eq!(
        CustomWeight::from_css_numeric(1_000_000),
        Some(CustomWeight::Black)
    );
}
