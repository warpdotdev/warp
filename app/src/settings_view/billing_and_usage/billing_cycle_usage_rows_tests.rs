use super::{MemberUsageRow, SourceFilter};
use crate::auth::UserUid;
use crate::settings_view::billing_and_usage::billing_cycle_usage_common::prepare_team_scoped_entries;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";

fn team_member(uid: &str, email: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
    }
}

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

#[test]
fn for_each_member_zero_fills_only_the_given_roster() {
    // Regression for the cross-team leak: `for_each_member` zero-fills every
    // member it's handed, so callers must pass the *current team's* roster
    // rather than the full workspace roster (which can span other teams
    // sharing the workspace) — otherwise members of other teams would get a
    // synthetic zero-usage row just for sharing a workspace with the viewer.
    let team_a_roster = vec![team_member(VIEWER_UID, "viewer@example.com")];
    let wider_workspace_roster = vec![
        team_member(VIEWER_UID, "viewer@example.com"),
        team_member(OTHER_UID, "other-team-member@example.com"),
    ];

    let team_a_rows = MemberUsageRow::for_each_member(&[], &team_a_roster, SourceFilter::All);
    assert_eq!(team_a_rows.len(), 1);
    assert_eq!(team_a_rows[0].subject_uid.as_deref(), Some(VIEWER_UID));

    let wider_rows =
        MemberUsageRow::for_each_member(&[], &wider_workspace_roster, SourceFilter::All);
    assert_eq!(
        wider_rows.len(),
        2,
        "sanity check: passing a wider roster does zero-fill more rows, which is exactly why \
         the call site must pass the team roster, not the workspace roster"
    );
}

#[test]
fn prepare_team_scoped_entries_then_for_each_member_excludes_other_team_usage() {
    // End-to-end: raw workspace-scoped entries (mixed team A / team B /
    // unattributed) go through the same `prepare_team_scoped_entries`
    // pipeline the section uses, then into `for_each_member` with team A's
    // roster. Team B's usage must not surface anywhere in the output — not
    // as a row, and not folded into team A's numbers.
    let team_a_roster = vec![
        team_member(VIEWER_UID, "viewer@example.com"),
        team_member("idle-uid", "idle@example.com"),
    ];
    let mut viewer_entry = entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(VIEWER_UID),
        AiCreditsUsageSource::Local,
        10,
        5,
    );
    viewer_entry.attributed_team_uid = Some("team-a".to_string());
    let mut other_team_entry = entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(OTHER_UID),
        AiCreditsUsageSource::Local,
        999,
        999,
    );
    other_team_entry.attributed_team_uid = Some("team-b".to_string());
    let mut unattributed_entry = entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some("unattributed-uid"),
        AiCreditsUsageSource::Local,
        50,
        50,
    );
    unattributed_entry.attributed_team_uid = None;
    let raw_entries = vec![viewer_entry, other_team_entry, unattributed_entry];

    let scoped = prepare_team_scoped_entries(&raw_entries, Some("team-a"));
    let rows = MemberUsageRow::for_each_member(&scoped, &team_a_roster, SourceFilter::All);

    assert_eq!(
        rows.len(),
        2,
        "only team A's roster should get rows: the viewer (with usage) and the idle member (zero-filled)"
    );
    let viewer_row = rows
        .iter()
        .find(|row| row.subject_uid.as_deref() == Some(VIEWER_UID))
        .expect("viewer should have a row");
    assert_eq!(viewer_row.total_credits, 10);
    let idle_row = rows
        .iter()
        .find(|row| row.subject_uid.as_deref() == Some("idle-uid"))
        .expect("idle team member should still get a zero-usage row");
    assert_eq!(idle_row.total_credits, 0);
    assert!(
        rows.iter()
            .all(|row| row.subject_uid.as_deref() != Some(OTHER_UID)),
        "team B's member must not appear as a row"
    );
}

#[test]
fn for_each_member_includes_zero_usage_team_members() {
    // A team member with no usage this cycle still gets a row.
    let members = vec![
        team_member(VIEWER_UID, "viewer@example.com"),
        team_member("idle-uid", "idle@example.com"),
    ];
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(VIEWER_UID),
        AiCreditsUsageSource::Local,
        10,
        5,
    )];

    let rows = MemberUsageRow::for_each_member(&entries, &members, SourceFilter::All);

    assert_eq!(rows.len(), 2);
    let idle_row = rows
        .iter()
        .find(|row| row.subject_uid.as_deref() == Some("idle-uid"))
        .expect("idle team member should still get a zero-usage row");
    assert_eq!(idle_row.total_credits, 0);
}
