use super::{
    SortKey, SortOrder, TEAM_MEMBERS_USAGE_LABEL, TEAM_MEMBERS_USAGE_WORKSPACE_WIDE_CAPTION,
    UserSortingCriteria, resolve_team_scoped_members, sort_user_items_in_place,
};
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::workspace::{
    Workspace, WorkspaceMember, WorkspaceMemberUsageInfo, WorkspaceUid,
};

fn workspace_member(uid: &str, email: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn team_member(uid: &str, email: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
    }
}

fn workspace_with_members(members: Vec<WorkspaceMember>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(0i64)),
        "workspace".to_string(),
        None,
    );
    workspace.members = members;
    workspace
}

#[test]
fn resolve_team_scoped_members_fails_closed_when_team_unresolved() {
    let workspace = workspace_with_members(vec![workspace_member("a", "a@warp.dev")]);
    assert!(resolve_team_scoped_members(Some(&workspace), None).is_empty());
}

#[test]
fn resolve_team_scoped_members_fails_closed_when_workspace_unresolved() {
    let team = Team::from_local_cache(
        ServerId::from(1i64),
        "Team A".to_string(),
        None,
        None,
        Some(vec![team_member("a", "a@warp.dev")]),
    );
    assert!(resolve_team_scoped_members(None, Some(&team)).is_empty());
}

#[test]
fn resolve_team_scoped_members_excludes_members_outside_the_team() {
    let workspace = workspace_with_members(vec![
        workspace_member("a", "a@warp.dev"),
        workspace_member("b", "b@warp.dev"),
    ]);
    let team = Team::from_local_cache(
        ServerId::from(1i64),
        "Team A".to_string(),
        None,
        None,
        Some(vec![team_member("a", "a@warp.dev")]),
    );

    let scoped = resolve_team_scoped_members(Some(&workspace), Some(&team));

    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].email, "a@warp.dev");
}

#[test]
fn team_members_usage_label_and_caption_do_not_claim_team_scoping() {
    // Regression: `WorkspaceMemberUsageInfo.requests_used_since_last_refresh`
    // has no per-team attribution, so the summed row and per-member counters
    // built from it are workspace-wide, not scoped to the team being viewed.
    // The label must not claim otherwise, and the caption must disclose it.
    assert_ne!(
        TEAM_MEMBERS_USAGE_LABEL, "Team total",
        "the label must not claim these workspace-wide counters are team-scoped"
    );
    let caption_lower = TEAM_MEMBERS_USAGE_WORKSPACE_WIDE_CAPTION.to_lowercase();
    assert!(
        caption_lower.contains("every team") || caption_lower.contains("workspace"),
        "caption must plainly disclose that the figures span every team the member belongs to, got: {TEAM_MEMBERS_USAGE_WORKSPACE_WIDE_CAPTION}"
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
