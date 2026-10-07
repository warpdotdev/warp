use super::format_request_usage;
use crate::settings::UsageDisplayUnit;

/// The unit is resolved by the caller (see `effective_usage_unit`), so the formatter renders
/// whichever unit it is handed.
#[test]
fn request_usage_uses_selected_display_unit() {
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
    assert_eq!(
        format_request_usage(20.0, None, UsageDisplayUnit::Dollars),
        "Credits used: 20 credits"
    );
}
