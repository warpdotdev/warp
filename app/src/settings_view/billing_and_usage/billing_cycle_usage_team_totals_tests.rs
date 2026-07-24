use super::{TeamTotalCardSummary, build_team_total_card_summaries};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, UsageVisibility, UsageVisibilityGranularity,
};

fn entry(
    usage_source: AiCreditsUsageSource,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some("u".to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source,
        credits_used,
        cost_cents,
    }
}

fn visibility(granularity: UsageVisibilityGranularity) -> UsageVisibility {
    UsageVisibility {
        granularity,
        max_prior_cycles: Default::default(),
    }
}

fn entries_two_per_source() -> Vec<BillingCycleUsageEntry> {
    vec![
        entry(AiCreditsUsageSource::Local, 30, 10),
        entry(AiCreditsUsageSource::Cloud, 70, 25),
    ]
}

fn titles(summaries: &[TeamTotalCardSummary]) -> Vec<&'static str> {
    summaries.iter().map(|s| s.title).collect()
}

#[test]
fn team_aggregate_visibility_yields_overall_card_only() {
    // Server collapses teammates' usage into an `Aggregate`-source row under
    // TeamAggregate, so the Local/Cloud split can't be honestly attributed
    // — only the Overall card is meaningful.
    let summaries = build_team_total_card_summaries(
        &entries_two_per_source(),
        &visibility(UsageVisibilityGranularity::TeamAggregate),
    );
    assert_eq!(titles(&summaries), vec!["Overall usage"]);
}

#[test]
fn own_only_visibility_yields_overall_card_only() {
    // OwnOnly viewers don't normally render the team-totals block at all,
    // but the builder should still degrade gracefully to a single card.
    let summaries = build_team_total_card_summaries(
        &entries_two_per_source(),
        &visibility(UsageVisibilityGranularity::OwnOnly),
    );
    assert_eq!(titles(&summaries), vec!["Overall usage"]);
}

#[test]
fn per_user_totals_visibility_yields_overall_card_only() {
    let summaries = build_team_total_card_summaries(
        &entries_two_per_source(),
        &visibility(UsageVisibilityGranularity::PerUserTotals),
    );
    assert_eq!(titles(&summaries), vec!["Overall usage"]);
}

#[test]
fn full_breakdown_visibility_returns_three_cards_with_partitioned_sums() {
    let summaries = build_team_total_card_summaries(
        &entries_two_per_source(),
        &visibility(UsageVisibilityGranularity::FullBreakdown),
    );

    assert_eq!(
        titles(&summaries),
        vec!["Overall usage", "Local agent usage", "Cloud agent usage"]
    );

    // Overall = Local + Cloud; Local card = only Local entries; Cloud card =
    // only Cloud entries. Distinct credits/cost per source catch any swapped
    // filter.
    assert_eq!(summaries[0].total_credits, 30 + 70);
    assert_eq!(summaries[0].total_cost_cents, 10 + 25);
    assert_eq!(summaries[1].total_credits, 30);
    assert_eq!(summaries[1].total_cost_cents, 10);
    assert_eq!(summaries[2].total_credits, 70);
    assert_eq!(summaries[2].total_cost_cents, 25);
}
