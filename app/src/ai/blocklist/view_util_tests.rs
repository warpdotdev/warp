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
        "20 credits (12,345 tokens, $0.36)"
    );
}

#[test]
fn format_credits_with_cost_formats_large_token_counts_with_thousands_separators() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(26.9, Some(719_124), Some(48.0)),
        "26.9 credits (719,124 tokens, $0.48)"
    );
}

#[test]
fn format_credits_with_cost_omits_dollar_figure_when_cost_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(12345), None),
        "20 credits (12,345 tokens)"
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
fn format_credits_with_cost_omits_token_figure_when_tokens_is_zero() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(0), Some(36.0)),
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

#[test]
fn format_credits_with_cost_omits_parenthetical_when_tokens_are_zero_and_cost_is_unknown() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_credits_with_cost(20.0, Some(0), None),
        format_credits(20.0)
    );
}
