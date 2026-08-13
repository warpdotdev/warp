use super::{
    BarSegment, aggregate_segments, filter_entries_by_attributed_team, filter_legacy_buckets,
    has_non_viewer_data, legend_cost_types, scope_usage_to_team,
};
use crate::auth::UserUid;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, WorkspaceMember, WorkspaceMemberUsageInfo,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";

fn entry(
    subject_type: AiCreditsUsageAndCostSubjectType,
    subject_uid: Option<&str>,
    cost_type: AiCreditsUsageAndCostType,
    usage_bucket: AiCreditsUsageBucket,
    usage_source: AiCreditsUsageSource,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type,
        subject_uid: subject_uid.map(|s| s.to_string()),
        subject_display_name: None,
        cost_type,
        usage_bucket,
        usage_source,
        credits_used,
        cost_cents,
        attributed_team_uid: None,
    }
}

/// Boilerplate viewer-owned User row for predicate tests.
fn viewer_user_entry() -> BillingCycleUsageEntry {
    entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(VIEWER_UID),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        10,
        0,
    )
}

#[test]
fn has_non_viewer_data_returns_false_when_entries_empty() {
    assert!(!has_non_viewer_data(&[], Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_false_when_only_viewer_user_rows() {
    let entries = vec![viewer_user_entry(), viewer_user_entry()];
    assert!(!has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_true_for_team_aggregate_row() {
    // TeamAggregate visibility represents "everyone else's usage" as a single
    // Team-typed row, even when the workspace currently has only one member
    // (e.g. a teammate left mid-cycle after incurring AI costs).
    let entries = vec![
        viewer_user_entry(),
        entry(
            AiCreditsUsageAndCostSubjectType::Team,
            None,
            AiCreditsUsageAndCostType::Aggregate,
            AiCreditsUsageBucket::Aggregate,
            AiCreditsUsageSource::Aggregate,
            500,
            300,
        ),
    ];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_true_for_other_user_row() {
    // PerUserTotals / FullBreakdown emit per-user rows, so a departed teammate
    // shows up as a User entry with a non-viewer UID.
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(OTHER_UID),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        50,
        0,
    )];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_true_for_service_account_row() {
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::ServiceAccount,
        Some("sa-uid"),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Cloud,
        25,
        0,
    )];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_treats_missing_subject_uid_as_non_viewer() {
    // Defensive: a User row with no UID is conservatively treated as a non-
    // viewer subject so we never accidentally drop team scaffolding.
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        None,
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        1,
        0,
    )];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_treats_missing_viewer_uid_as_non_viewer() {
    // Signed-out / unidentified viewer: any subject we can't prove belongs
    // to them counts as non-viewer data.
    let entries = vec![viewer_user_entry()];
    assert!(has_non_viewer_data(&entries, None));
}

#[test]
fn filter_legacy_buckets_drops_voice_and_suggested_code_diffs_in_input_order() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Voice,
            AiCreditsUsageSource::Local,
            3,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageSource::Local,
            5,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::SuggestedCodeDiffs,
            AiCreditsUsageSource::Local,
            7,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Aggregate,
            AiCreditsUsageBucket::Aggregate,
            AiCreditsUsageSource::Aggregate,
            100,
            50,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Platform,
            AiCreditsUsageSource::Cloud,
            2,
            0,
        ),
    ];

    let filtered = filter_legacy_buckets(&entries);

    let kept_buckets: Vec<_> = filtered.iter().map(|e| e.usage_bucket.clone()).collect();
    assert_eq!(
        kept_buckets,
        vec![
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageBucket::Aggregate,
            AiCreditsUsageBucket::Platform,
        ],
        "expected Voice + SuggestedCodeDiffs dropped while preserving the rest in input order"
    );
}

#[test]
fn aggregate_segments_merges_dupes_drops_zeros_and_sorts() {
    let entries = [
        // Same (BonusGrant, Compute) appears twice across different sources;
        // should merge into one segment.
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BonusGrant,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageSource::Local,
            10,
            5,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BonusGrant,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageSource::Cloud,
            7,
            3,
        ),
        // BaseLimit/Ai — should sort before any BonusGrant entry.
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            20,
            0,
        ),
        // Zero-credit entry: must be dropped before totals are computed (so
        // the stray cost_cents don't leak into the row total).
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Payg,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            0,
            42,
        ),
    ];

    let (segments, total_credits, total_cost_cents) = aggregate_segments(entries.iter());

    let key = |s: &BarSegment| (s.cost_type.clone(), s.usage_bucket.clone());
    let keys: Vec<_> = segments.iter().map(key).collect();
    assert_eq!(
        keys,
        vec![
            (
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai
            ),
            (
                AiCreditsUsageAndCostType::BonusGrant,
                AiCreditsUsageBucket::Compute
            ),
        ],
        "expected BaseLimit/Ai before BonusGrant/Compute, Payg zero-credit dropped"
    );

    let bonus = &segments[1];
    assert_eq!(bonus.credits, 17, "10 + 7 merged credits");
    assert_eq!(bonus.cost_cents, 8, "5 + 3 merged cost cents");

    // Totals are summed *after* the zero-credit segment is dropped, so the
    // stray 42 cents on the Payg/Ai entry must not appear here.
    assert_eq!(total_credits, 20 + 17);
    assert_eq!(total_cost_cents, 8);
}

#[test]
fn legend_cost_types_excludes_zero_credit_bucket() {
    // Regression: a base-limit row with no usage must not surface "Base" in
    // the legend while only Pay-as-you-go credits were actually spent.
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            0,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Payg,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            50,
            120,
        ),
    ];

    assert_eq!(
        legend_cost_types(&entries),
        vec![AiCreditsUsageAndCostType::Payg],
        "zero-credit BaseLimit row must be dropped from the legend"
    );
}

#[test]
fn legend_cost_types_includes_used_buckets_in_display_order() {
    // Buckets with real usage appear in the canonical legend order regardless
    // of input order (Payg listed before BaseLimit here).
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Payg,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            5,
            10,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            8,
            0,
        ),
    ];

    assert_eq!(
        legend_cost_types(&entries),
        vec![
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageAndCostType::Payg,
        ],
        "used buckets should render in canonical order, not input order"
    );
}

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

/// Entry helper for attributed-team tests: a `User` subject with a real
/// `subject_uid`, parameterized only on `credits_used` and
/// `attributed_team_uid` since the other fields are irrelevant here.
fn entry_for_team(
    subject_uid: &str,
    credits_used: i32,
    attributed_team_uid: Option<&str>,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents: 0,
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
    }
}

#[test]
fn filter_entries_by_attributed_team_keeps_only_matching_team() {
    let entries = vec![
        entry_for_team("user-a", 10, Some("team-a")),
        entry_for_team("user-b", 20, Some("team-b")),
        entry_for_team("user-c", 5, None),
    ];
    let filtered = filter_entries_by_attributed_team(&entries, "team-a");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].subject_uid.as_deref(), Some("user-a"));
}

#[test]
fn filter_entries_by_attributed_team_excludes_unattributed_entries() {
    // Null attribution (e.g. pre-backfill data) must never leak into a
    // specific team's view, matching the web's strict-equality filter.
    let entries = vec![entry_for_team("user-a", 10, None)];
    assert!(filter_entries_by_attributed_team(&entries, "team-a").is_empty());
}

#[test]
fn scope_usage_to_team_filters_entries_and_roster_for_multi_team_workspace() {
    // Regression for the cross-team data leak: an admin of team A must not
    // see team B's subjects or credits, and unattributed entries must be
    // dropped from the team-scoped view.
    let entries = vec![
        entry_for_team("admin-a", 10, Some("team-a")),
        entry_for_team("member-b", 999, Some("team-b")),
        entry_for_team("unattributed", 50, None),
    ];
    let workspace_members = vec![
        workspace_member("admin-a", "admin-a@example.com"),
        workspace_member("member-b", "member-b@example.com"),
    ];
    let team_a_members = vec![team_member("admin-a", "admin-a@example.com")];

    let (scoped_entries, scoped_members) = scope_usage_to_team(
        &entries,
        &workspace_members,
        2,
        Some(("team-a", &team_a_members)),
    );

    assert_eq!(scoped_entries.len(), 1);
    assert_eq!(scoped_entries[0].subject_uid.as_deref(), Some("admin-a"));
    assert_eq!(scoped_members.len(), 1, "roster must be limited to team A");
    assert_eq!(scoped_members[0].email, "admin-a@example.com");
}

#[test]
fn scope_usage_to_team_is_unfiltered_for_single_team_workspace() {
    // Single-team workspaces have nothing to scope away from, and their
    // entries may predate attribution backfill (null attributed_team_uid),
    // so filtering here must be a no-op.
    let entries = vec![entry_for_team("user-a", 10, None)];
    let workspace_members = vec![workspace_member("user-a", "user-a@example.com")];
    let team_members = vec![team_member("user-a", "user-a@example.com")];

    let (scoped_entries, scoped_members) = scope_usage_to_team(
        &entries,
        &workspace_members,
        1,
        Some(("team-a", &team_members)),
    );

    assert_eq!(scoped_entries.len(), entries.len());
    assert_eq!(scoped_members.len(), workspace_members.len());
}

#[test]
fn scope_usage_to_team_is_unfiltered_when_current_team_unresolved() {
    // Workspace-level views (no resolvable current team, e.g. the admin's
    // workspace-wide overview) must keep seeing every team's entries.
    let entries = vec![entry_for_team("user-a", 10, Some("team-a"))];
    let workspace_members = vec![workspace_member("user-a", "user-a@example.com")];

    let (scoped_entries, scoped_members) =
        scope_usage_to_team(&entries, &workspace_members, 2, None);

    assert_eq!(scoped_entries.len(), entries.len());
    assert_eq!(scoped_members.len(), workspace_members.len());
}

#[test]
fn legend_cost_types_excludes_legacy_only_buckets() {
    // Voice / SuggestedCodeDiffs usage is written as BaseLimit credits but is
    // dropped from the bars; the legend must match and not show "Base".
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Voice,
            AiCreditsUsageSource::Local,
            12,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::SuggestedCodeDiffs,
            AiCreditsUsageSource::Local,
            4,
            0,
        ),
    ];

    assert!(
        legend_cost_types(&entries).is_empty(),
        "legacy-only base-limit usage must not surface any legend bucket"
    );
}
