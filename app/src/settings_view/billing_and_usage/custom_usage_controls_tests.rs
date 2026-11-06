use warp_graphql::billing::CustomUsageAmounts;

use super::parse_usage_dollars;

const POLICY: CustomUsageAmounts = CustomUsageAmounts {
    minimum_cents: 1_000,
    maximum_cents: 1_000_000,
    step_cents: 100,
};

#[test]
fn accepts_whole_dollars_including_policy_boundaries() {
    assert_eq!(parse_usage_dollars("10", POLICY), Some(1_000));
    assert_eq!(parse_usage_dollars("37", POLICY), Some(3_700));
    assert_eq!(parse_usage_dollars("10000", POLICY), Some(1_000_000));
}

#[test]
fn rejects_out_of_range_or_non_integer_amounts_without_rounding() {
    assert_eq!(parse_usage_dollars("9", POLICY), None);
    assert_eq!(parse_usage_dollars("10001", POLICY), None);
    assert_eq!(parse_usage_dollars("37.50", POLICY), None);
    assert_eq!(parse_usage_dollars("-37", POLICY), None);
    assert_eq!(parse_usage_dollars("1e2", POLICY), None);
    assert_eq!(parse_usage_dollars("2147483647", POLICY), None);
    assert_eq!(parse_usage_dollars("", POLICY), None);
}
