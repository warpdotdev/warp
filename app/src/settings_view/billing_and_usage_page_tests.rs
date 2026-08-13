use super::{
    SortKey, SortOrder, UserSortingCriteria, sort_user_items_in_place,
    team_scoped_workspace_members,
};
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::workspace::{
    Workspace, WorkspaceMember, WorkspaceMemberUsageInfo, WorkspaceUid,
};

fn team_member(uid: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
    }
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 1000,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn team(uid_seed: i64, members: Vec<TeamMember>) -> Team {
    Team::from_local_cache(
        uid_seed.into(),
        format!("team-{uid_seed}"),
        None,
        None,
        Some(members),
    )
}

fn workspace_with_members(members: Vec<WorkspaceMember>) -> Workspace {
    let uid = WorkspaceUid::from(ServerId::from(1_i64));
    let mut workspace = Workspace::from_local_cache(uid, "Workspace".to_string(), None);
    workspace.members = members;
    workspace
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

#[test]
fn team_scoped_workspace_members_returns_only_the_active_teams_roster() {
    let workspace = workspace_with_members(vec![
        workspace_member("a-only"),
        workspace_member("b-only"),
        workspace_member("shared"),
    ]);
    let team_a = team(1, vec![team_member("a-only"), team_member("shared")]);
    let team_b = team(2, vec![team_member("b-only"), team_member("shared")]);

    let team_a_members = team_scoped_workspace_members(Some(&workspace), Some(&team_a));
    let team_b_members = team_scoped_workspace_members(Some(&workspace), Some(&team_b));

    let team_a_uids: Vec<&str> = team_a_members.iter().map(|m| m.uid.as_str()).collect();
    let team_b_uids: Vec<&str> = team_b_members.iter().map(|m| m.uid.as_str()).collect();
    assert_eq!(team_a_uids, ["a-only", "shared"]);
    assert_eq!(team_b_uids, ["b-only", "shared"]);
}

#[test]
fn team_scoped_workspace_members_fails_closed_when_no_active_team() {
    // Privacy-safe fallback: with no active team resolved, this must return
    // no members rather than the whole workspace roster.
    let workspace = workspace_with_members(vec![workspace_member("a-only")]);

    let members = team_scoped_workspace_members(Some(&workspace), None);

    assert!(members.is_empty());
}
