use warp_core::features::FeatureFlag;

use super::*;

#[test]
fn format_credits_with_cost_omits_parenthetical_when_flag_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), Some(36.0)),
        format_credits(20.0)
    );
}

#[test]
fn format_credits_with_cost_appends_tokens_and_dollar_suffix_when_flag_enabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), Some(36.0)),
        "20 credits (12345 tokens, $0.36)"
    );
}

#[test]
fn format_credits_with_cost_omits_dollar_figure_when_cost_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), None),
        "20 credits (12345 tokens)"
    );
}

#[test]
fn format_credits_with_cost_omits_token_figure_when_tokens_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, None, Some(36.0)),
        "20 credits ($0.36)"
    );
}

#[test]
fn format_credits_with_cost_omits_parenthetical_when_both_are_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, None, None),
        format_credits(20.0)
    );
}
