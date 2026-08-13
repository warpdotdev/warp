use chrono::TimeZone;

use super::*;
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::workspaces::team::MembershipRole;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageData, BillingMetadata, WorkspaceMember,
    WorkspaceMemberUsageInfo,
};

fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn team_member(uid: &str, email: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
    }
}

fn workspace_member(uid: &str, email: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: true,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn usage_entry(
    subject_uid: &str,
    attributed_team_uid: Option<&str>,
    credits_used: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents: 0,
    }
}

fn workspace_with_teams(
    teams: Vec<Team>,
    billing_cycle_usage: Option<BillingCycleUsageData>,
) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: BillingMetadata::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage,
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        // Deliberately non-empty and disjoint from either team's roster:
        // if the wiring ever falls back to `workspace.members` instead of
        // the viewed team's own members, these regression tests will
        // surface these emails instead of the expected team roster.
        members: vec![workspace_member(
            "workspace-only-uid",
            "workspace-only@example.com",
        )],
        total_requests_used_since_last_refresh: 0,
    }
}

/// Regression coverage for the cross-team usage leak: `TeamUsageDisplayData`
/// is the single source of truth `render_team_usage` and `shows_team_section`
/// both build from, so this exercises the exact wiring that scopes usage to
/// the team currently being viewed rather than the whole workspace.
#[test]
fn team_usage_display_data_scopes_entries_and_roster_to_the_viewed_team() {
    let team_a = Team::from_local_cache(
        ServerId::from(1),
        "Team A".to_string(),
        None,
        None,
        Some(vec![team_member("admin-uid", "admin@example.com")]),
    );
    let team_b = Team::from_local_cache(
        ServerId::from(2),
        "Team B".to_string(),
        None,
        None,
        Some(vec![
            team_member("other-uid-1", "other1@example.com"),
            team_member("other-uid-2", "other2@example.com"),
        ]),
    );
    let team_a_uid = team_a.uid.to_string();
    let team_b_uid = team_b.uid.to_string();

    let period_start = utc(2026, 6, 27);
    let period_end = utc(2026, 7, 27);
    let entries = vec![
        usage_entry("admin-uid", Some(&team_a_uid), 10),
        usage_entry("other-uid-1", Some(&team_b_uid), 500),
        usage_entry("other-uid-2", Some(&team_b_uid), 250),
        // Usage predating per-team attribution: must not leak into either
        // team's scoped view.
        usage_entry("someone-else", None, 999),
    ];
    let billing_cycle_usage = BillingCycleUsageData {
        current_period_start: period_start,
        current_period_end: period_end,
        summaries: vec![BillingCycleUsageSummary {
            period_start,
            period_end,
            entries,
        }],
    };
    let workspace = workspace_with_teams(
        vec![team_a.clone(), team_b.clone()],
        Some(billing_cycle_usage),
    );

    // Viewing team A: only team A's entry and roster member surface. If this
    // ever regresses to unfiltered workspace entries or `workspace.members`,
    // team B's (or the workspace-only) data would leak in here.
    let display_a = TeamUsageDisplayData::build(&workspace, Some(&team_a), None);
    assert_eq!(
        display_a.entries.len(),
        1,
        "expected only team A's entry; team B's and unassigned usage must be excluded"
    );
    assert_eq!(
        display_a.entries[0].subject_uid.as_deref(),
        Some("admin-uid")
    );
    assert_eq!(display_a.entries[0].credits_used, 10);
    assert_eq!(
        display_a
            .members
            .iter()
            .map(|m| m.email.as_str())
            .collect::<Vec<_>>(),
        vec!["admin@example.com"],
        "the zero-usage member roster must come from team A, not the workspace"
    );
    assert!(
        !display_a.shows_team_section(Some("admin-uid")),
        "team A has one member and no non-viewer data, so Team/Members must not render"
    );

    // Viewing team B: only team B's entries and roster surface.
    let display_b = TeamUsageDisplayData::build(&workspace, Some(&team_b), None);
    assert_eq!(display_b.entries.len(), 2);
    assert!(
        display_b
            .entries
            .iter()
            .all(|e| e.subject_uid.as_deref() != Some("admin-uid")),
        "team A's entry must not leak into team B's view"
    );
    assert_eq!(
        display_b
            .members
            .iter()
            .map(|m| m.email.as_str())
            .collect::<Vec<_>>(),
        vec!["other1@example.com", "other2@example.com"]
    );
    assert!(
        display_b.shows_team_section(Some("other-uid-1")),
        "team B has more than one member so Team/Members should render"
    );
}

#[test]
fn team_usage_display_data_passes_through_unfiltered_when_no_team_resolves() {
    let period_start = utc(2026, 6, 27);
    let period_end = utc(2026, 7, 27);
    let entries = vec![
        usage_entry("some-uid", Some("team-a-uid"), 10),
        usage_entry("other-uid", None, 20),
    ];
    let billing_cycle_usage = BillingCycleUsageData {
        current_period_start: period_start,
        current_period_end: period_end,
        summaries: vec![BillingCycleUsageSummary {
            period_start,
            period_end,
            entries,
        }],
    };
    let workspace = workspace_with_teams(vec![], Some(billing_cycle_usage));

    let display = TeamUsageDisplayData::build(&workspace, None, None);

    assert_eq!(
        display.entries.len(),
        2,
        "no team to scope to; entries pass through"
    );
    assert!(display.members.is_empty());
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
