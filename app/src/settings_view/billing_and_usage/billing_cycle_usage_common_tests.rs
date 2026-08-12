use super::{
    BarSegment, aggregate_segments, filter_entries_by_attributed_team, filter_legacy_buckets,
    has_non_viewer_data, legend_cost_types,
};
use crate::workspaces::team::Team;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";
const SHARED_UID: &str = "shared-uid";

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
        attributed_team_uid: None,
        cost_type,
        usage_bucket,
        usage_source,
        credits_used,
        cost_cents,
    }
}

/// An entry with an explicit attribution, for
/// `filter_entries_by_attributed_team` tests. Deriving the attribution
/// string from the `Team` itself (rather than a hand-typed string) avoids
/// any mismatch with the padded `ServerId` `team_with_uid` builds. Other
/// fields are given arbitrary but valid defaults, since these tests only
/// care about attribution.
fn attributed_entry(
    subject_type: AiCreditsUsageAndCostSubjectType,
    subject_uid: Option<&str>,
    attributed_team: Option<&Team>,
    credits_used: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        attributed_team_uid: attributed_team.map(|team| team.uid.to_string()),
        ..entry(
            subject_type,
            subject_uid,
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            credits_used,
            0,
        )
    }
}

/// `Team::uid` is a fixed-width `ServerId` (exactly 22 characters); pad an
/// arbitrary short label out to that width so tests can use readable names.
fn team_with_uid(label: &str) -> Team {
    let uid = format!("{label:0>22}");
    Team::from_local_cache(
        crate::server::ids::ServerId::from_string_lossy(&uid),
        "Team".to_string(),
        None,
        None,
        None,
    )
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

#[test]
fn filter_entries_by_attributed_team_passes_through_unfiltered_with_no_team_context() {
    let team_a = team_with_uid("team-a");
    let team_b = team_with_uid("team-b");
    let entries = vec![
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some("a-only-uid"),
            Some(&team_a),
            10,
        ),
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some("b-only-uid"),
            Some(&team_b),
            20,
        ),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, None);

    assert_eq!(filtered.len(), 2, "no team context means no filtering");
}

#[test]
fn filter_entries_by_attributed_team_keeps_only_entries_matching_the_selected_team() {
    let team_a = team_with_uid("team-a");
    let team_b = team_with_uid("team-b");
    let entries = vec![
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some("a-only-uid"),
            Some(&team_a),
            10,
        ),
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some("b-only-uid"),
            Some(&team_b),
            999,
        ),
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some("nobody"),
            None,
            999,
        ),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, Some(&team_a));

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].subject_uid.as_deref(), Some("a-only-uid"));
}

#[test]
fn filter_entries_by_attributed_team_splits_a_shared_members_usage_by_attribution() {
    // CRITICAL regression case: a member who belongs to both team A and team
    // B has *two* entries, one attributed to each team. Roster membership
    // alone can't tell these apart (the member is in both rosters), which is
    // exactly the leak this filter must close: selecting team A must show
    // only the A-attributed entry, never the B-attributed one, and vice
    // versa.
    let team_a = team_with_uid("team-a");
    let team_b = team_with_uid("team-b");
    let entries = vec![
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(SHARED_UID),
            Some(&team_a),
            10,
        ),
        attributed_entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(SHARED_UID),
            Some(&team_b),
            20,
        ),
    ];

    let under_a = filter_entries_by_attributed_team(&entries, Some(&team_a));
    assert_eq!(
        under_a.len(),
        1,
        "only the A-attributed entry should survive"
    );
    assert_eq!(under_a[0].credits_used, 10);

    let under_b = filter_entries_by_attributed_team(&entries, Some(&team_b));
    assert_eq!(
        under_b.len(),
        1,
        "only the B-attributed entry should survive"
    );
    assert_eq!(under_b[0].credits_used, 20);
}

#[test]
fn filter_entries_by_attributed_team_drops_unattributed_entries_when_team_selected() {
    let team_a = team_with_uid("team-a");
    let entries = vec![attributed_entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some("legacy-uid"),
        None,
        10,
    )];

    assert!(filter_entries_by_attributed_team(&entries, Some(&team_a)).is_empty());
}

#[test]
fn filter_entries_by_attributed_team_keeps_the_synthetic_team_row_when_attributed_to_the_selected_team()
 {
    // The TeamAggregate "other members" rollup is scoped correctly as long
    // as the server attributes it to a specific team; unlike roster
    // membership, attribution can express this even though the row has no
    // subject_uid.
    let team_a = team_with_uid("team-a");
    let entries = vec![attributed_entry(
        AiCreditsUsageAndCostSubjectType::Team,
        None,
        Some(&team_a),
        500,
    )];

    assert_eq!(
        filter_entries_by_attributed_team(&entries, Some(&team_a)).len(),
        1
    );
}

#[test]
fn filter_entries_by_attributed_team_drops_the_synthetic_team_row_attributed_elsewhere() {
    let team_a = team_with_uid("team-a");
    let team_b = team_with_uid("team-b");
    let entries = vec![attributed_entry(
        AiCreditsUsageAndCostSubjectType::Team,
        None,
        Some(&team_b),
        500,
    )];

    assert!(filter_entries_by_attributed_team(&entries, Some(&team_a)).is_empty());
}

#[test]
fn filter_entries_by_attributed_team_keeps_a_departed_members_historical_entry() {
    // A former team member no longer appears in `Team.members`, but their
    // historical usage while they were on the team is still attributed to
    // it, so it must remain visible — attribution, not current roster
    // membership, is what decides this.
    let team_a = team_with_uid("team-a");
    let entries = vec![attributed_entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some("departed-uid"),
        Some(&team_a),
        10,
    )];

    assert_eq!(
        filter_entries_by_attributed_team(&entries, Some(&team_a)).len(),
        1
    );
}

#[test]
fn filter_entries_by_attributed_team_keeps_a_service_accounts_attributed_entry() {
    // Service accounts are never listed in `Team.members`, but their entries
    // still carry real attribution and must not be dropped.
    let team_a = team_with_uid("team-a");
    let entries = vec![attributed_entry(
        AiCreditsUsageAndCostSubjectType::ServiceAccount,
        Some("agent-uid"),
        Some(&team_a),
        10,
    )];

    assert_eq!(
        filter_entries_by_attributed_team(&entries, Some(&team_a)).len(),
        1
    );
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
