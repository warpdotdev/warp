use ordered_float::OrderedFloat;

use super::{
    Divisor, SortKey, SortOrder, UsageCents, UserSortingCriteria, format_usage_count,
    member_usage_cents, shows_dollars, sort_user_items_in_place,
};
use crate::settings::UsageDisplayUnit;
use crate::workspaces::workspace::WorkspaceMemberUsageInfo;

fn member_usage_info(
    is_unlimited: bool,
    included_usage_cents: Option<f64>,
    usage_cents_used_since_last_refresh: Option<f64>,
) -> WorkspaceMemberUsageInfo {
    WorkspaceMemberUsageInfo {
        is_unlimited,
        request_limit: 1000,
        requests_used_since_last_refresh: 250,
        included_usage_cents: included_usage_cents.map(OrderedFloat),
        usage_cents_used_since_last_refresh: usage_cents_used_since_last_refresh.map(OrderedFloat),
        is_request_limit_prorated: false,
    }
}

#[test]
fn usage_count_shows_dollars_only_when_every_figure_has_a_dollar_value() {
    let limit = Some(Divisor::Limit(1_500));
    let used_of_limit = Some(UsageCents {
        used: 70.2,
        limit: Some(1800.0),
    });
    assert_eq!(
        format_usage_count(1_250, limit, used_of_limit),
        "$0.70/$18.00"
    );
    assert!(shows_dollars(limit, used_of_limit));

    let used_only = Some(UsageCents {
        used: 70.2,
        limit: None,
    });
    assert_eq!(format_usage_count(1_250, limit, used_only), "1,250/1,500");
    assert!(!shows_dollars(limit, used_only));
    assert_eq!(format_usage_count(1_250, limit, None), "1,250/1,500");
    assert!(!shows_dollars(limit, None));
}

#[test]
fn usage_count_without_a_limit_shows_the_used_figure_in_either_unit() {
    let used_only = Some(UsageCents {
        used: 70.2,
        limit: None,
    });
    assert_eq!(format_usage_count(1_250, None, used_only), "$0.70");
    assert!(shows_dollars(None, used_only));
    assert_eq!(format_usage_count(1_250, None, None), "1,250");

    let unlimited = Some(Divisor::Unlimited);
    assert_eq!(
        format_usage_count(1_250, unlimited, used_only),
        "$0.70/Unlimited"
    );
    assert_eq!(
        format_usage_count(1_250, unlimited, None),
        "1,250/Unlimited"
    );
    assert!(!shows_dollars(unlimited, None));
}

#[test]
fn member_usage_cents_requires_the_dollars_unit_and_the_used_figure() {
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Dollars,
            &member_usage_info(false, Some(1800.0), Some(450.5))
        ),
        Some(UsageCents {
            used: 450.5,
            limit: Some(1800.0),
        })
    );
    // A missing limit still renders the used figure's dollars for rows without a limit; the
    // row formatter falls back to credits when it has one.
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Dollars,
            &member_usage_info(false, None, Some(450.5))
        ),
        Some(UsageCents {
            used: 450.5,
            limit: None,
        })
    );
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Dollars,
            &member_usage_info(false, Some(1800.0), None)
        ),
        None
    );
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Dollars,
            &member_usage_info(false, None, None)
        ),
        None
    );
    // Cents the server sends are never shown when displaying in credits.
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Credits,
            &member_usage_info(false, Some(1800.0), Some(450.5))
        ),
        None
    );
    // Unlimited members keep the credit display even if dollar figures were supplied.
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Dollars,
            &member_usage_info(true, None, None)
        ),
        None
    );
    assert_eq!(
        member_usage_cents(
            UsageDisplayUnit::Dollars,
            &member_usage_info(true, Some(1800.0), Some(450.5))
        ),
        None
    );
}

#[test]
pub fn test_default_sorting_pins_current_user_first_then_display_name_asc() {
    let mut items = vec![
        UserSortingCriteria::new("Zed".to_string(), 10, ()),
        UserSortingCriteria::new("Alice".to_string(), 5, ()),
        UserSortingCriteria::new("Bob".to_string(), 15, ()),
    ];

    sort_user_items_in_place(&mut items, "Bob", None, SortOrder::Asc);

    // Expected: Bob (current user) first, then Alice, then Zed (by display name asc)
    assert_eq!(items[0].display_name, "Bob");
    assert_eq!(items[1].display_name, "Alice");
    assert_eq!(items[2].display_name, "Zed");
}

#[test]
fn test_display_name_az_sorting_pins_current_user() {
    let mut items = vec![
        UserSortingCriteria::new("Zed".to_string(), 10, ()),
        UserSortingCriteria::new("Alice".to_string(), 5, ()),
        UserSortingCriteria::new("Bob".to_string(), 15, ()),
        UserSortingCriteria::new("charlie@example.com".to_string(), 8, ()), // Using email as display name fallback
    ];

    sort_user_items_in_place(
        &mut items,
        "Bob",
        Some(SortKey::DisplayName),
        SortOrder::Asc,
    );

    // Expected: Bob (current user) first, then Alice, charlie@ (fallback to email), Zed
    assert_eq!(items[0].display_name, "Bob");
    assert_eq!(items[1].display_name, "Alice");
    assert_eq!(items[2].display_name, "charlie@example.com"); // Email as display name fallback
    assert_eq!(items[3].display_name, "Zed");
}

#[test]
fn test_display_name_za_sorting_pins_current_user() {
    let mut items = vec![
        UserSortingCriteria::new("Zed".to_string(), 10, ()),
        UserSortingCriteria::new("Alice".to_string(), 5, ()),
        UserSortingCriteria::new("Bob".to_string(), 15, ()),
    ];

    sort_user_items_in_place(
        &mut items,
        "Alice",
        Some(SortKey::DisplayName),
        SortOrder::Desc,
    );

    // Expected: Alice (current user) first, then Zed, Bob (by name desc)
    assert_eq!(items[0].display_name, "Alice");
    assert_eq!(items[1].display_name, "Zed");
    assert_eq!(items[2].display_name, "Bob");
}

#[test]
fn test_requests_usage_desc_sorting_pins_current_user_with_display_name_tie_breaker() {
    let mut items = vec![
        UserSortingCriteria::new("Alice".to_string(), 10, ()),
        UserSortingCriteria::new("Bob".to_string(), 15, ()),
        UserSortingCriteria::new("Charlie".to_string(), 10, ()), // Same usage as Alice
        UserSortingCriteria::new("Diana".to_string(), 5, ()),
    ];

    sort_user_items_in_place(
        &mut items,
        "Diana",
        Some(SortKey::Requests),
        SortOrder::Desc,
    );

    // Expected: Diana (current user) first, then Bob (15), then Alice/Charlie by name (10 tie)
    assert_eq!(items[0].display_name, "Diana");
    assert_eq!(items[1].display_name, "Bob"); // Highest usage (15)
    assert_eq!(items[2].display_name, "Alice"); // Tied at 10, "Alice" < "Charlie"
    assert_eq!(items[3].display_name, "Charlie");
}

#[test]
fn test_requests_usage_asc_sorting_pins_current_user_with_display_name_tie_breaker() {
    let mut items = vec![
        UserSortingCriteria::new("Alice".to_string(), 10, ()),
        UserSortingCriteria::new("Bob".to_string(), 15, ()),
        UserSortingCriteria::new("Charlie".to_string(), 10, ()), // Same usage as Alice
        UserSortingCriteria::new("Diana".to_string(), 5, ()),
    ];

    sort_user_items_in_place(&mut items, "Bob", Some(SortKey::Requests), SortOrder::Asc);

    // Expected: Bob (current user) first, then Diana (5), then Alice/Charlie by name (10 tie)
    assert_eq!(items[0].display_name, "Bob");
    assert_eq!(items[1].display_name, "Diana"); // Lowest usage (5)
    assert_eq!(items[2].display_name, "Alice"); // Tied at 10, "Alice" < "Charlie"
    assert_eq!(items[3].display_name, "Charlie");
}

#[test]
fn test_display_name_az_sorting_with_emails() {
    let mut items = vec![
        UserSortingCriteria::new("zuser@example.com".to_string(), 10, ()),
        UserSortingCriteria::new("Alice".to_string(), 5, ()),
        UserSortingCriteria::new("buser@example.com".to_string(), 15, ()),
    ];

    sort_user_items_in_place(
        &mut items,
        "Alice",
        Some(SortKey::DisplayName),
        SortOrder::Asc,
    );

    // Expected: Alice (current user) first, then buser@... < zuser@... (by email fallback)
    assert_eq!(items[0].display_name, "Alice");
    assert_eq!(items[1].display_name, "buser@example.com"); // Email as display name
    assert_eq!(items[2].display_name, "zuser@example.com"); // Email as display name
}

#[test]
fn test_case_insensitive_display_name_sorting() {
    let mut items = vec![
        UserSortingCriteria::new("alice".to_string(), 10, ()),
        UserSortingCriteria::new("Bob".to_string(), 5, ()),
        UserSortingCriteria::new("CHARLIE".to_string(), 8, ()),
        UserSortingCriteria::new("Diana".to_string(), 12, ()),
    ];

    sort_user_items_in_place(
        &mut items,
        "Diana",
        Some(SortKey::DisplayName),
        SortOrder::Asc,
    );

    // Expected: Diana (current user) first, then alice, Bob, CHARLIE (case-insensitive asc)
    assert_eq!(items[0].display_name, "Diana");
    assert_eq!(items[1].display_name, "alice"); // "alice" (lowercase)
    assert_eq!(items[2].display_name, "Bob"); // "Bob"
    assert_eq!(items[3].display_name, "CHARLIE"); // "CHARLIE"
}
