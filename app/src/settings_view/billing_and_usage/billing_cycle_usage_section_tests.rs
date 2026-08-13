use chrono::TimeZone;

use super::*;
use crate::auth::UserUid;
use crate::settings_view::billing_and_usage::billing_cycle_usage_team_totals::build_team_total_card_summaries;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageBucket, AiCreditsUsageSource,
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

fn team_with_uid_and_members(uid: i64, members: Vec<TeamMember>) -> Team {
    Team {
        uid: uid.into(),
        name: "team".to_string(),
        color: None,
        invite_code: None,
        members,
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn team_member(uid: &str, email: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
    }
}

fn usage_entry(
    subject_uid: &str,
    attributed_team_uid: Option<&str>,
    cost_type: AiCreditsUsageAndCostType,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents,
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
    }
}

/// Regression covering the full team-usage data-preparation wiring: given
/// entries mixing team A, team B, and unattributed usage, only team A's
/// entries, roster, totals, and legend categories should reach rendering.
/// A future change that reintroduces the workspace-wide leak in
/// `prepare_team_usage_data`, `render_team_usage`, or the legend wiring
/// would fail this test even though the standalone filter/roster/totals
/// unit tests elsewhere stay green.
#[test]
fn prepare_team_usage_data_scopes_entries_roster_totals_and_legend_to_the_team() {
    let team_a_member = team_member("member-a", "a@example.com");
    let team_a = team_with_uid_and_members(111, vec![team_a_member.clone()]);
    let team_a_uid = team_a.uid.to_string();
    let team_b_uid = team_with_uid_and_members(222, vec![]).uid.to_string();

    let entries = vec![
        usage_entry(
            "member-a",
            Some(&team_a_uid),
            AiCreditsUsageAndCostType::BaseLimit,
            10,
            5,
        ),
        usage_entry(
            "member-b",
            Some(&team_b_uid),
            AiCreditsUsageAndCostType::Payg,
            999,
            999,
        ),
        usage_entry(
            "member-c",
            None,
            AiCreditsUsageAndCostType::AmbientBonusGrant,
            999,
            999,
        ),
    ];

    let data = prepare_team_usage_data(&entries, Some(&team_a));

    assert_eq!(data.entries.len(), 1, "only team A's entry should survive");
    assert_eq!(data.entries[0].credits_used, 10);

    assert_eq!(data.roster.len(), 1, "roster must be team A's members only");
    assert_eq!(data.roster[0].uid, team_a_member.uid.as_str());

    let visibility = UsageVisibility {
        granularity: UsageVisibilityGranularity::PerUserTotals,
        max_prior_cycles: Default::default(),
    };
    let totals = build_team_total_card_summaries(&data.entries, &visibility);
    assert_eq!(
        totals[0].total_credits, 10,
        "totals downstream of the prepared entries must reflect team A only"
    );

    let legend_types = legend_cost_types(&data.entries);
    assert_eq!(
        legend_types,
        vec![AiCreditsUsageAndCostType::BaseLimit],
        "the legend must not surface a category that exists only because of team B's or unattributed usage"
    );
}

#[test]
fn prepare_team_usage_data_fails_closed_when_team_is_unresolved() {
    // If the window's current team can't be resolved (the invariant that
    // guards `render_team_usage` was somehow violated), prefer hiding all
    // team data over silently falling back to the unfiltered/workspace-wide
    // set, which would reintroduce the original cross-team leak.
    let entries = vec![usage_entry(
        "member-a",
        Some("team-a"),
        AiCreditsUsageAndCostType::BaseLimit,
        10,
        5,
    )];

    let data = prepare_team_usage_data(&entries, None);

    assert!(data.entries.is_empty());
    assert!(data.roster.is_empty());
}
