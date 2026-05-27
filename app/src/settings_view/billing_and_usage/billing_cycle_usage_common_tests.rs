use super::{aggregate_segments, filter_legacy_buckets, has_non_viewer_data, BarSegment};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry,
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
