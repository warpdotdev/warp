use super::schedule_is_visible_to_scope;
use crate::ai::ambient_agents::scheduled::{
    CloudScheduledAmbientAgent, CloudScheduledAmbientAgentModel, ScheduledAmbientAgent,
};
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::server::ids::{ServerId, SyncId};
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

fn schedule_with_owner(id: i64, owner: Owner) -> CloudScheduledAmbientAgent {
    let mut permissions = CloudObjectPermissions::mock_personal();
    permissions.owner = owner;
    CloudScheduledAmbientAgent::new(
        SyncId::ServerId(ServerId::from(id)),
        CloudScheduledAmbientAgentModel::new(ScheduledAmbientAgent::new(
            "Schedule".to_string(),
            "0 9 * * 1".to_string(),
            true,
            "Prompt".to_string(),
        )),
        CloudObjectMetadata::mock(),
        permissions,
    )
}

#[test]
fn schedule_scope_includes_personal_and_matching_team_schedules() {
    let selected_team_uid = ServerId::from(123);
    let selected_scope = TeamContextForOperation::new_for_test(selected_team_uid);
    let personal_schedule = schedule_with_owner(1, Owner::mock_current_user());
    let selected_team_schedule = schedule_with_owner(
        2,
        Owner::Team {
            team_uid: selected_team_uid,
        },
    );
    let other_team_schedule = schedule_with_owner(
        3,
        Owner::Team {
            team_uid: ServerId::from(456),
        },
    );

    assert!(schedule_is_visible_to_scope(
        &personal_schedule,
        &selected_scope
    ));
    assert!(schedule_is_visible_to_scope(
        &selected_team_schedule,
        &selected_scope
    ));
    assert!(!schedule_is_visible_to_scope(
        &other_team_schedule,
        &selected_scope
    ));
}

#[test]
fn teamless_schedule_scope_includes_only_personal_schedules() {
    let personal_schedule = schedule_with_owner(1, Owner::mock_current_user());
    let team_schedule = schedule_with_owner(
        2,
        Owner::Team {
            team_uid: ServerId::from(123),
        },
    );

    assert!(schedule_is_visible_to_scope(
        &personal_schedule,
        &TeamlessScopeForTest
    ));
    assert!(!schedule_is_visible_to_scope(
        &team_schedule,
        &TeamlessScopeForTest
    ));
}
