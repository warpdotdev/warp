use serde_json::json;

use super::*;
use crate::server::ids::ServerId;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};

fn output() -> WhoamiOutput {
    WhoamiOutput {
        uid: "user-1".to_string(),
        principal_type: "user",
        display_name: Some("Ada".to_string()),
        email: Some("ada@example.com".to_string()),
        team_uids: vec![],
        team_names: vec![],
        workspace_uid: None,
        workspace_name: None,
    }
}

fn workspace(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "Acme".to_string(),
        Some(teams),
        None,
    )
}

fn team(uid: i64, name: &str, member_uids: &[&str]) -> Team {
    Team::from_local_cache(
        ServerId::from(uid),
        name.to_string(),
        None,
        None,
        Some(
            member_uids
                .iter()
                .map(|uid| TeamMember {
                    uid: UserUid::new(uid),
                    email: format!("{uid}@example.com"),
                    role: MembershipRole::User,
                    is_disabled: false,
                })
                .collect(),
        ),
        None,
    )
}

#[test]
fn single_team_omits_admin_visible_non_member_teams() {
    let mut output = output();
    output.set_workspace(
        Some(&workspace(vec![
            team(1, "Platform", &["user-1"]),
            team(2, "Admin Only", &["other-user"]),
        ])),
        UserUid::new("user-1"),
    );

    assert_eq!(
        serde_json::to_value(&output).unwrap(),
        json!({
            "uid": "user-1",
            "type": "user",
            "display_name": "Ada",
            "email": "ada@example.com",
            "team_uids": ["test_uid00000000000001"],
            "team_names": ["Platform"],
            "workspace_uid": "workspace_uid123456789",
            "workspace_name": "Acme",
        })
    );
    assert_eq!(
        output.pretty(PrincipalType::User),
        "User ID: user-1\nDisplay Name: Ada\nEmail: ada@example.com\nWorkspace UID: workspace_uid123456789\nWorkspace Name: Acme\nTeam ID: test_uid00000000000001\nTeam Name: Platform"
    );
}

#[test]
fn multiple_teams_include_workspace_and_repeat_pretty_team_labels() {
    let mut output = output();
    output.set_workspace(
        Some(&workspace(vec![
            team(1, "Platform", &["user-1"]),
            team(2, "Product", &["user-1"]),
            team(3, "Admin Only", &["other-user"]),
        ])),
        UserUid::new("user-1"),
    );

    assert_eq!(
        serde_json::to_value(&output).unwrap(),
        json!({
            "uid": "user-1",
            "type": "user",
            "display_name": "Ada",
            "email": "ada@example.com",
            "team_uids": ["test_uid00000000000001", "test_uid00000000000002"],
            "team_names": ["Platform", "Product"],
            "workspace_uid": "workspace_uid123456789",
            "workspace_name": "Acme",
        })
    );
    assert_eq!(
        output.pretty(PrincipalType::User),
        "User ID: user-1\nDisplay Name: Ada\nEmail: ada@example.com\nWorkspace UID: workspace_uid123456789\nWorkspace Name: Acme\nTeams:\n  Team ID: test_uid00000000000001\n  Team Name: Platform\n  Team ID: test_uid00000000000002\n  Team Name: Product"
    );
}
