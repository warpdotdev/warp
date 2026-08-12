use super::{TeamTotalCardSummary, build_team_total_card_summaries};
use crate::settings_view::billing_and_usage::billing_cycle_usage_common::prepare_team_scoped_entries;
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
        attributed_team_uid: None,
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
fn prepare_team_scoped_entries_then_totals_excludes_other_team_usage() {
    // Raw, unfiltered entries mixing team A and team B usage in the same
    // workspace-scoped payload (as `Workspace.billing_cycle_usage` actually
    // returns). Only team A's numbers must reach the Overall/Local/Cloud
    // cards once the section-level pipeline scopes them.
    let mut team_a_local = entry(AiCreditsUsageSource::Local, 30, 10);
    team_a_local.attributed_team_uid = Some("team-a".to_string());
    let mut team_a_cloud = entry(AiCreditsUsageSource::Cloud, 20, 5);
    team_a_cloud.attributed_team_uid = Some("team-a".to_string());
    let mut team_b_local = entry(AiCreditsUsageSource::Local, 999, 999);
    team_b_local.attributed_team_uid = Some("team-b".to_string());
    let raw_entries = vec![team_a_local, team_a_cloud, team_b_local];

    let scoped = prepare_team_scoped_entries(&raw_entries, Some("team-a"));
    let summaries = build_team_total_card_summaries(
        &scoped,
        &visibility(UsageVisibilityGranularity::FullBreakdown),
    );

    assert_eq!(
        summaries[0].total_credits,
        30 + 20,
        "team B's usage must not inflate team A's Overall total"
    );
    assert_eq!(summaries[0].total_cost_cents, 10 + 5);
    assert_eq!(
        summaries[1].total_credits, 30,
        "Local card must only include team A's Local entry"
    );
    assert_eq!(
        summaries[2].total_credits, 20,
        "Cloud card must only include team A's Cloud entry"
    );
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
