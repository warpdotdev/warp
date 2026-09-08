use super::{SortKey, SortOrder, UserSortingCriteria, sort_user_items_in_place};

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
