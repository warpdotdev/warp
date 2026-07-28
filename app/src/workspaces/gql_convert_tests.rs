use super::*;

fn team(name: &str, member_uids: &[&str]) -> Team {
    Team::from_local_cache(
        ServerId::from_string_lossy(format!("{name:0>22}")),
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
                })
                .collect(),
        ),
    )
}

fn workspace(teams: Vec<Team>) -> Workspace {
    Workspace::from_local_cache(
        format!("{:0>22}", "workspace").into(),
        "workspace".to_string(),
        Some(teams),
    )
}

fn team_names(workspace: &Workspace) -> Vec<&str> {
    workspace
        .teams
        .iter()
        .map(|team| team.name.as_str())
        .collect()
}

#[test]
fn order_authenticated_teams_before_non_member_teams() {
    let mut workspace = workspace(vec![
        team("non-member", &["other-user"]),
        team("member", &["current-user"]),
    ]);

    order_authenticated_teams_first(&mut workspace, UserUid::new("current-user"));

    assert_eq!(team_names(&workspace), ["member", "non-member"]);
}

#[test]
fn preserve_relative_order_within_member_groups() {
    let mut workspace = workspace(vec![
        team("non-member-one", &["other-user"]),
        team("member-one", &["current-user"]),
        team("non-member-two", &["another-user"]),
        team("member-two", &["current-user"]),
    ]);

    order_authenticated_teams_first(&mut workspace, UserUid::new("current-user"));

    assert_eq!(
        team_names(&workspace),
        [
            "member-one",
            "member-two",
            "non-member-one",
            "non-member-two"
        ]
    );
}

#[test]
fn preserve_server_order_when_user_has_no_team_membership() {
    let mut workspace = workspace(vec![
        team("first", &["other-user"]),
        team("second", &["another-user"]),
    ]);

    order_authenticated_teams_first(&mut workspace, UserUid::new("current-user"));

    assert_eq!(team_names(&workspace), ["first", "second"]);
}
