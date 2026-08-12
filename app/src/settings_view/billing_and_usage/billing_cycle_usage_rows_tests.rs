use super::{MemberUsageRow, SourceFilter, build_rows};
use crate::auth::UserUid;
use crate::workspaces::team::MembershipRole;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, UsageVisibility, UsageVisibilityGranularity,
    WorkspaceMember, WorkspaceMemberUsageInfo,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";

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
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source,
        credits_used,
        cost_cents,
        attributed_team_uid: None,
    }
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@warp.dev"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 100,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
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

fn full_breakdown() -> UsageVisibility {
    UsageVisibility {
        granularity: UsageVisibilityGranularity::FullBreakdown,
        max_prior_cycles: Default::default(),
    }
}

fn viewer() -> (Option<String>, String) {
    (Some(VIEWER_UID.to_string()), "viewer".to_string())
}

fn row_subject_uids(rows: &[MemberUsageRow]) -> Vec<String> {
    rows.iter().filter_map(|r| r.subject_uid.clone()).collect()
}

#[test]
fn build_rows_seeds_zero_usage_rows_only_from_the_roster_it_is_given() {
    // The rows are built from the roster the caller hands in rather than from
    // the workspace, which is what lets the section pass a team-scoped one.
    // Drop a member from that roster and their zero-usage row goes with them.
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(VIEWER_UID),
        AiCreditsUsageSource::Local,
        10,
        0,
    )];

    let rows = build_rows(
        &[workspace_member(VIEWER_UID), workspace_member(OTHER_UID)],
        &entries,
        &full_breakdown(),
        SourceFilter::All,
        &viewer(),
    );
    let mut uids = row_subject_uids(&rows);
    uids.sort();
    assert_eq!(
        uids,
        vec![OTHER_UID.to_string(), VIEWER_UID.to_string()],
        "every roster member gets a row, including the one with no usage"
    );

    let rows = build_rows(
        &[workspace_member(VIEWER_UID)],
        &entries,
        &full_breakdown(),
        SourceFilter::All,
        &viewer(),
    );
    assert_eq!(
        row_subject_uids(&rows),
        vec![VIEWER_UID.to_string()],
        "a member absent from the roster leaves no zero-usage row behind"
    );
}

#[test]
fn build_rows_keeps_non_roster_subjects_present_in_the_entries() {
    // Service accounts are never on the roster, but their usage is attributed
    // to the team in view, so narrowing the roster must not drop them.
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::ServiceAccount,
            Some("agent-uid"),
            AiCreditsUsageSource::Cloud,
            50,
            0,
        ),
    ];

    let rows = build_rows(
        &[workspace_member(VIEWER_UID)],
        &entries,
        &full_breakdown(),
        SourceFilter::All,
        &viewer(),
    );

    assert_eq!(
        row_subject_uids(&rows),
        vec!["agent-uid".to_string(), VIEWER_UID.to_string()],
        "sorted by total credits desc"
    );
}
