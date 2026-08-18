use pathfinder_color::ColorU;

use super::{
    DISABLED_MEMBER_TOOLTIP_TEXT, MemberUsageRow, SourceFilter, dimmed_row_text_color,
    disabled_member_tooltip_text,
};
use crate::auth::UserUid;
use crate::workspaces::team::MembershipRole;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, WorkspaceMember, WorkspaceMemberUsageInfo,
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

fn member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@warp.dev"),
        role: MembershipRole::User,
        is_disabled: false,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn disabled_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        is_disabled: true,
        ..member(uid)
    }
}

#[test]
fn per_member_rows_cover_exactly_the_supplied_roster() {
    // Callers pass the selected team's roster, so a workspace member from
    // another team gets no row at all — not even a zero-usage one.
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(VIEWER_UID),
        AiCreditsUsageSource::Local,
        10,
        5,
    )];

    let rows = MemberUsageRow::for_each_member(
        &entries,
        &[member(VIEWER_UID), member(OTHER_UID)],
        SourceFilter::All,
    );

    let named: Vec<_> = rows.iter().map(|r| r.display_name.as_str()).collect();
    assert_eq!(named, vec!["viewer-uid@warp.dev", "other-uid@warp.dev"]);
    assert_eq!(rows[0].total_credits, 10);
    assert_eq!(rows[1].total_credits, 0, "zero-usage roster member");

    let rows = MemberUsageRow::for_each_member(&entries, &[member(VIEWER_UID)], SourceFilter::All);
    assert_eq!(
        rows.iter()
            .map(|r| r.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["viewer-uid@warp.dev"],
        "members outside the roster must not get a row"
    );
}

#[test]
fn per_member_rows_mark_departed_users_as_former_members() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageSource::Local,
            10,
            5,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(OTHER_UID),
            AiCreditsUsageSource::Local,
            20,
            10,
        ),
    ];

    let rows = MemberUsageRow::for_each_member(&entries, &[member(VIEWER_UID)], SourceFilter::All);

    assert!(
        rows.iter()
            .find(|row| row.subject_uid.as_deref() == Some(VIEWER_UID))
            .is_some_and(|row| row.is_current_team_member)
    );
    assert!(
        rows.iter()
            .find(|row| row.subject_uid.as_deref() == Some(OTHER_UID))
            .is_some_and(|row| !row.is_current_team_member)
    );
}

#[test]
fn per_member_rows_do_not_mark_service_accounts_as_former_members() {
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::ServiceAccount,
        Some("agent-uid"),
        AiCreditsUsageSource::Cloud,
        20,
        10,
    )];

    let rows = MemberUsageRow::for_each_member(&entries, &[], SourceFilter::All);

    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_current_team_member);
}

#[test]
fn per_member_rows_flag_disabled_members_from_the_roster() {
    let rows = MemberUsageRow::for_each_member(
        &[],
        &[member(VIEWER_UID), disabled_member(OTHER_UID)],
        SourceFilter::All,
    );

    assert!(
        rows.iter()
            .find(|row| row.subject_uid.as_deref() == Some(VIEWER_UID))
            .is_some_and(|row| !row.is_disabled),
        "an active member's row must not be flagged disabled"
    );
    assert!(
        rows.iter()
            .find(|row| row.subject_uid.as_deref() == Some(OTHER_UID))
            .is_some_and(|row| row.is_disabled),
        "a disabled member's row should carry the disabled flag for the dimmed/tooltip treatment"
    );
}

#[test]
fn per_member_rows_never_flag_departed_members_as_disabled() {
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(OTHER_UID),
        AiCreditsUsageSource::Local,
        20,
        10,
    )];

    let rows = MemberUsageRow::for_each_member(&entries, &[member(VIEWER_UID)], SourceFilter::All);

    assert!(
        rows.iter()
            .find(|row| row.subject_uid.as_deref() == Some(OTHER_UID))
            .is_some_and(|row| !row.is_disabled)
    );
}

#[test]
fn disabled_row_renders_dimmed_and_tooltipped() {
    let main = ColorU::new(255, 255, 255, 255);
    let dimmed = ColorU::new(128, 128, 128, 255);

    assert_eq!(dimmed_row_text_color(main, dimmed, true), dimmed);
    assert_eq!(dimmed_row_text_color(main, dimmed, false), main);
    assert_eq!(
        disabled_member_tooltip_text(true),
        Some(DISABLED_MEMBER_TOOLTIP_TEXT)
    );
    assert_eq!(disabled_member_tooltip_text(false), None);
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
