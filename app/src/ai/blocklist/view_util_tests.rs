use warp_core::features::FeatureFlag;

use super::{format_credits, format_credits_with_cost};

#[test]
fn flag_off_matches_format_credits_exactly_regardless_of_cost() {
    let _guard = FeatureFlag::PricingTransparency.override_enabled(false);
    assert_eq!(
        format_credits_with_cost(20.0, Some(36.0)),
        format_credits(20.0)
    );
    assert_eq!(format_credits_with_cost(20.0, None), format_credits(20.0));
}

#[test]
fn flag_on_appends_dollar_cost_when_available() {
    let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
    assert_eq!(
        format_credits_with_cost(20.0, Some(36.0)),
        "20 credits ($0.36)"
    );
    assert_eq!(format_credits_with_cost(1.0, Some(5.0)), "1 credit ($0.05)");
}

#[test]
fn flag_on_omits_dollar_cost_gracefully_when_unavailable() {
    let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
    // `None` must never be rendered as "$0.00" -- it means unknown/legacy,
    // not zero.
    assert_eq!(format_credits_with_cost(20.0, None), format_credits(20.0));
}

#[test]
fn flag_on_rounds_cents_to_two_decimal_places() {
    let _guard = FeatureFlag::PricingTransparency.override_enabled(true);
    assert_eq!(
        format_credits_with_cost(5.0, Some(0.4)),
        "5 credits ($0.00)"
    );
    assert_eq!(
        format_credits_with_cost(5.0, Some(999.6)),
        "5 credits ($10.00)"
    );
}
