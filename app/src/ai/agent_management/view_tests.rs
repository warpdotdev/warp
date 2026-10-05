use warp_core::features::FeatureFlag;

use super::format_request_usage;
use crate::settings::UsageDisplayUnit;

#[test]
fn request_usage_uses_selected_display_unit() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_request_usage(20.0, Some(36.0), UsageDisplayUnit::Dollars),
        "Usage: $0.36"
    );
    assert_eq!(
        format_request_usage(20.0, Some(36.0), UsageDisplayUnit::Credits),
        "Credits used: 20 credits"
    );
}

#[test]
fn request_usage_falls_back_to_credits_without_dollar_data() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(true);

    assert_eq!(
        format_request_usage(20.0, None, UsageDisplayUnit::Dollars),
        "Credits used: 20 credits"
    );
}

#[test]
fn request_usage_uses_credits_when_pricing_transparency_is_disabled() {
    let _flag = FeatureFlag::PricingTransparency.override_enabled(false);

    assert_eq!(
        format_request_usage(20.0, Some(36.0), UsageDisplayUnit::Dollars),
        "Credits used: 20 credits"
    );
}
