use super::{MemberUsageRow, SourceFilter};
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::settings_view::billing_and_usage::billing_cycle_usage_common::filter_entries_by_attributed_team;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, Workspace, WorkspaceMember,
    WorkspaceMemberUsageInfo,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";
const ADMIN_UID: &str = "admin-uid";
const A_ONLY_UID: &str = "a-only-uid";
const B_ONLY_UID: &str = "b-only-uid";
const SHARED_UID: &str = "shared-uid";

fn entry(
    subject_type: AiCreditsUsageAndCostSubjectType,
    subject_uid: Option<&str>,
    usage_source: AiCreditsUsageSource,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type,
        subject_uid: subject_uid.map(|s| s.to_string()),
        subject_display_name: None,
        attributed_team_uid: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source,
        credits_used,
        cost_cents,
    }
}

/// An entry carrying an explicit team attribution, for pipeline tests that
/// exercise `filter_entries_by_attributed_team` ahead of row construction.
/// Deriving the attribution string from the `Team` itself avoids any
/// mismatch with the padded `ServerId` the `team` helper builds.
fn attributed_entry(
    subject_uid: &str,
    attributed_team: &Team,
    credits_used: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        attributed_team_uid: Some(attributed_team.uid.to_string()),
        ..entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(subject_uid),
            AiCreditsUsageSource::Local,
            credits_used,
            0,
        )
    }
}

#[test]
fn build_own_usage_row_drops_team_subject_entries() {
    // Team-aggregate rows belong to "everyone else" by construction; they
    // must never contribute to the viewer's own row totals.
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            5,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::Team,
            None,
            AiCreditsUsageSource::Aggregate,
            999,
            999,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::All,
    );
    assert_eq!(row.total_credits, 10);
    assert_eq!(row.total_cost_cents, 5);
}

#[test]
fn build_own_usage_row_drops_other_users_entries() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(OTHER_UID),
            AiCreditsUsageSource::Local,
            999,
            999,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::All,
    );
    assert_eq!(row.total_credits, 10);
    assert_eq!(row.total_cost_cents, 0);
}

#[test]
fn build_own_usage_row_local_filter_drops_cloud_entries() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Cloud,
            20,
            0,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::Local,
    );
    assert_eq!(row.total_credits, 10);
}

#[test]
fn build_own_usage_row_cloud_filter_drops_local_entries() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Cloud,
            20,
            0,
        ),
    ];
    let row = MemberUsageRow::for_viewer(
        &entries,
        Some(VIEWER_UID),
        "viewer".to_string(),
        SourceFilter::Cloud,
    );
    assert_eq!(row.total_credits, 20);
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 100,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

/// `Team::uid` is a fixed-width `ServerId`; pad an arbitrary short label out
/// to that width so tests can use readable names.
fn team(uid_label: &str, member_uids: &[&str]) -> Team {
    let uid = format!("{uid_label:0>22}");
    Team::from_local_cache(
        ServerId::from_string_lossy(&uid),
        "Team".to_string(),
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

#[test]
fn team_usage_pipeline_excludes_other_team_members_and_splits_shared_members_usage() {
    // End-to-end regression test for the reported leak: this composes the
    // exact sequence `render_team_usage`/`build_rows` run in production
    // (`Workspace::members_for_team` for the roster, then
    // `filter_entries_by_attributed_team` for the usage entries, then
    // `MemberUsageRow::for_each_member` for the rows) instead of hand-
    // picking already-scoped inputs, so it fails if either filter is
    // dropped or weakened back to a roster-only check.
    let mut workspace = Workspace::from_local_cache(
        ServerId::from_string_lossy("workspace_uid123456789").into(),
        "Workspace".to_string(),
        None,
    );
    workspace.members = vec![
        workspace_member(ADMIN_UID),
        workspace_member(A_ONLY_UID),
        workspace_member(B_ONLY_UID),
        workspace_member(SHARED_UID),
    ];
    let team_a = team("team-a", &[ADMIN_UID, A_ONLY_UID, SHARED_UID]);
    let team_b = team("team-b", &[B_ONLY_UID, SHARED_UID]);

    let raw_entries = vec![
        attributed_entry(ADMIN_UID, &team_a, 10),
        attributed_entry(B_ONLY_UID, &team_b, 999),
        // The shared member has usage attributed to *both* teams; only the
        // team-A slice may survive when team A is selected.
        attributed_entry(SHARED_UID, &team_a, 7),
        attributed_entry(SHARED_UID, &team_b, 999),
    ];

    let scoped_members: Vec<WorkspaceMember> = workspace
        .members_for_team(Some(&team_a))
        .into_iter()
        .cloned()
        .collect();
    let scoped_entries = filter_entries_by_attributed_team(&raw_entries, Some(&team_a));
    let rows = MemberUsageRow::for_each_member(&scoped_entries, &scoped_members, SourceFilter::All);

    assert!(
        !rows
            .iter()
            .any(|r| r.subject_uid.as_deref() == Some(B_ONLY_UID)),
        "b-only is not on team A and has no A-attributed usage; must not appear"
    );
    let shared_row = rows
        .iter()
        .find(|r| r.subject_uid.as_deref() == Some(SHARED_UID))
        .expect("shared member is on team A and must still get a row");
    assert_eq!(
        shared_row.total_credits, 7,
        "shared member's row must reflect only their A-attributed usage, not the B-attributed 999"
    );
}

#[test]
fn for_each_member_still_gives_zero_usage_members_a_row() {
    // a-only has no usage this cycle but must still render a zeroed row.
    let members = vec![workspace_member(ADMIN_UID), workspace_member(A_ONLY_UID)];
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(ADMIN_UID),
        AiCreditsUsageSource::Local,
        10,
        0,
    )];

    let rows = MemberUsageRow::for_each_member(&entries, &members, SourceFilter::All);

    let a_only_row = rows
        .iter()
        .find(|r| r.subject_uid.as_deref() == Some(A_ONLY_UID))
        .expect("a-only should still render a zero-usage row");
    assert_eq!(a_only_row.total_credits, 0);
}
