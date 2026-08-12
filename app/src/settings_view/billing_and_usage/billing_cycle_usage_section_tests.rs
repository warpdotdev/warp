use chrono::TimeZone;

use super::*;
use crate::server::ids::ServerId;
use crate::settings_view::billing_and_usage::billing_cycle_usage_team_totals::build_team_total_card_summaries;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageBucket, AiCreditsUsageSource,
    BillingCycleUsageData, WorkspaceUid,
};

fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn summary(start: DateTime<Utc>, end: DateTime<Utc>) -> BillingCycleUsageSummary {
    BillingCycleUsageSummary {
        period_start: start,
        period_end: end,
        entries: vec![],
    }
}

fn entry(
    attributed_team_uid: Option<&str>,
    cost_type: AiCreditsUsageAndCostType,
    usage_bucket: AiCreditsUsageBucket,
    usage_source: AiCreditsUsageSource,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some("user-uid".to_string()),
        subject_display_name: None,
        cost_type,
        usage_bucket,
        usage_source,
        credits_used,
        cost_cents,
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
    }
}

fn workspace_with_summaries(summaries: Vec<BillingCycleUsageSummary>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(1)),
        "Workspace".to_string(),
        None,
    );
    let current_period_start = summaries
        .first()
        .map_or_else(|| utc(2026, 1, 1), |s| s.period_start);
    let current_period_end = summaries
        .first()
        .map_or_else(|| utc(2026, 2, 1), |s| s.period_end);
    workspace.billing_cycle_usage = Some(BillingCycleUsageData {
        current_period_start,
        current_period_end,
        summaries,
    });
    workspace
}

// Regression coverage for the cross-team usage leak: `render_team_usage`,
// `shows_team_section`, and the header/legend all derive their entries from
// `team_scoped_entries_for_period`, so these tests exercise that single
// pipeline function directly with the exact mixed-team payloads the real
// call sites hand it. If a future edit stops calling this function from one
// of those call sites, it has nothing else to fall back on for filtering,
// so the call site itself will visibly regress rather than silently
// diverging from a helper whose predicate still tests "correct" in isolation.
#[test]
fn team_scoped_entries_for_period_scopes_to_team_and_drops_legacy_buckets() {
    let current_start = utc(2026, 6, 1);
    let current_end = utc(2026, 7, 1);
    let team_a_entry = entry(
        Some("team-a"),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        10,
        5,
    );
    // Team B has a cost category (Payg) that team A doesn't use this cycle.
    // Without team scoping this would leak into team A's legend even though
    // it never appears in A's totals or rows.
    let team_b_only_category_entry = entry(
        Some("team-b"),
        AiCreditsUsageAndCostType::Payg,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Cloud,
        999,
        999,
    );
    let unattributed_entry = entry(
        None,
        AiCreditsUsageAndCostType::BonusGrant,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        50,
        50,
    );
    let legacy_team_a_entry = entry(
        Some("team-a"),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Voice,
        AiCreditsUsageSource::Local,
        3,
        0,
    );
    let workspace = workspace_with_summaries(vec![BillingCycleUsageSummary {
        period_start: current_start,
        period_end: current_end,
        entries: vec![
            team_a_entry,
            team_b_only_category_entry,
            unattributed_entry,
            legacy_team_a_entry,
        ],
    }]);

    let scoped = team_scoped_entries_for_period(&workspace, None, Some("team-a"));

    assert_eq!(
        scoped.len(),
        1,
        "only team A's non-legacy entry should survive"
    );
    assert_eq!(scoped[0].attributed_team_uid.as_deref(), Some("team-a"));
    assert_eq!(scoped[0].cost_type, AiCreditsUsageAndCostType::BaseLimit);

    // The legend (and every other team-scoped consumer) derives from this
    // same scoped slice, so team B's Payg category never surfaces in team
    // A's legend.
    assert_eq!(
        legend_cost_types(&scoped),
        vec![AiCreditsUsageAndCostType::BaseLimit],
        "team B's Pay-as-you-go category must not leak into team A's legend"
    );
}

#[test]
fn team_scoped_entries_for_period_applies_same_scoping_to_historical_periods() {
    let current_end = utc(2026, 7, 1);
    let older_start = utc(2026, 5, 1);
    let older_end = utc(2026, 6, 1);

    let current_summary = BillingCycleUsageSummary {
        period_start: utc(2026, 6, 1),
        period_end: current_end,
        entries: vec![entry(
            Some("team-a"),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            111,
            0,
        )],
    };
    let older_summary = BillingCycleUsageSummary {
        period_start: older_start,
        period_end: older_end,
        entries: vec![
            entry(
                Some("team-a"),
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai,
                AiCreditsUsageSource::Local,
                20,
                0,
            ),
            entry(
                Some("team-b"),
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai,
                AiCreditsUsageSource::Local,
                999,
                0,
            ),
        ],
    };
    let workspace = workspace_with_summaries(vec![current_summary, older_summary]);

    let scoped = team_scoped_entries_for_period(&workspace, Some(older_end), Some("team-a"));

    assert_eq!(scoped.len(), 1);
    assert_eq!(
        scoped[0].credits_used, 20,
        "must select the historical period's team A entry, not the current period's"
    );
}

#[test]
fn team_scoped_entries_for_period_stays_unfiltered_when_no_team_is_resolved() {
    // Workspace-level / own-usage views pass `team_uid: None` and must keep
    // seeing the full (legacy-filtered only) period.
    let workspace = workspace_with_summaries(vec![BillingCycleUsageSummary {
        period_start: utc(2026, 6, 1),
        period_end: utc(2026, 7, 1),
        entries: vec![
            entry(
                Some("team-a"),
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai,
                AiCreditsUsageSource::Local,
                10,
                0,
            ),
            entry(
                Some("team-b"),
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai,
                AiCreditsUsageSource::Local,
                20,
                0,
            ),
        ],
    }]);

    let scoped = team_scoped_entries_for_period(&workspace, None, None);

    assert_eq!(scoped.len(), 2);
}

#[test]
fn team_scoped_entries_feed_only_team_a_into_totals() {
    let workspace = workspace_with_summaries(vec![BillingCycleUsageSummary {
        period_start: utc(2026, 6, 1),
        period_end: utc(2026, 7, 1),
        entries: vec![
            entry(
                Some("team-a"),
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai,
                AiCreditsUsageSource::Local,
                30,
                10,
            ),
            entry(
                Some("team-b"),
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai,
                AiCreditsUsageSource::Local,
                999,
                999,
            ),
        ],
    }]);

    let scoped = team_scoped_entries_for_period(&workspace, None, Some("team-a"));
    let visibility = UsageVisibility {
        granularity: UsageVisibilityGranularity::FullBreakdown,
        max_prior_cycles: Default::default(),
    };
    let summaries = build_team_total_card_summaries(&scoped, &visibility);

    assert_eq!(
        summaries[0].total_credits, 30,
        "team B's usage must not inflate team A's Overall total"
    );
    assert_eq!(summaries[0].total_cost_cents, 10);
}

fn sample_summaries() -> Vec<BillingCycleUsageSummary> {
    vec![
        summary(utc(2026, 6, 27), utc(2026, 7, 27)),
        summary(utc(2026, 5, 27), utc(2026, 6, 27)),
        summary(utc(2026, 4, 27), utc(2026, 5, 27)),
    ]
}

#[test]
fn builds_one_plain_item_per_period() {
    let summaries = sample_summaries();
    let items = build_period_menu_items(&summaries);

    assert_eq!(items.len(), summaries.len());
    for (item, summary) in items.iter().zip(summaries.iter()) {
        match item {
            MenuItem::Item(fields) => {
                assert_eq!(fields.icon(), None, "items should not carry a marker icon");
                match fields.on_select_action() {
                    Some(BillingCycleUsageAction::SelectPeriod(Some(end))) => {
                        assert_eq!(*end, summary.period_end);
                    }
                    other => panic!("expected SelectPeriod action, got {other:?}"),
                }
            }
            other => panic!("expected MenuItem::Item, got {other:?}"),
        }
    }
}

#[test]
fn selects_most_recent_period_when_none_selected() {
    let summaries = sample_summaries();
    assert_eq!(selected_period_index(&summaries, None), Some(0));
}

#[test]
fn selects_explicitly_selected_period() {
    let summaries = sample_summaries();
    assert_eq!(
        selected_period_index(&summaries, Some(utc(2026, 6, 27))),
        Some(1),
    );
    assert_eq!(
        selected_period_index(&summaries, Some(utc(2026, 5, 27))),
        Some(2),
    );
}

#[test]
fn selects_nothing_when_selection_absent() {
    let summaries = sample_summaries();
    assert_eq!(
        selected_period_index(&summaries, Some(utc(1999, 1, 1))),
        None
    );
}

#[test]
fn selects_nothing_when_no_summaries() {
    assert_eq!(selected_period_index(&[], None), None);
    assert_eq!(selected_period_index(&[], Some(utc(2026, 7, 27))), None);
}
