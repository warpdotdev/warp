use chrono::TimeZone;

use super::*;
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType as AiCostType,
    AiCreditsUsageBucket, AiCreditsUsageSource, UsageVisibilityPolicy, WorkspaceMemberUsageInfo,
    WorkspaceUid,
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

fn usage_entry(subject_uid: &str, attributed_team_uid: Option<ServerId>) -> BillingCycleUsageEntry {
    usage_entry_with_cost_type(subject_uid, attributed_team_uid, AiCostType::BaseLimit)
}

fn usage_entry_with_cost_type(
    subject_uid: &str,
    attributed_team_uid: Option<ServerId>,
    cost_type: AiCostType,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used: 10,
        cost_cents: 0,
        attributed_team_uid: attributed_team_uid.map(|uid| uid.to_string()),
    }
}

/// Sets a `FullBreakdown` admin-visibility policy on `workspace`, so
/// `resolve_usage_visibility(true)` yields a non-`OwnOnly` granularity —
/// the branch that requires team scoping.
fn with_full_breakdown_visibility(mut workspace: Workspace) -> Workspace {
    workspace.billing_metadata.tier.usage_visibility_policy = Some(UsageVisibilityPolicy {
        admin_granularity: UsageVisibilityGranularity::FullBreakdown,
        max_prior_cycles: MaxPriorCycles::None,
    });
    workspace
}

fn workspace_with_members(members: Vec<WorkspaceMember>) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(ServerId::from(0i64)),
        "workspace".to_string(),
        None,
    );
    workspace.members = members;
    workspace
}

#[test]
fn resolve_team_scoped_usage_fails_closed_when_team_unresolved() {
    let workspace = workspace_with_members(vec![workspace_member("a", "a@warp.dev")]);
    let entries = vec![usage_entry("a", Some(ServerId::from(0i64)))];

    assert!(resolve_team_scoped_usage(None, &workspace, &entries).is_none());
}

#[test]
fn resolve_team_scoped_usage_filters_entries_and_scopes_members() {
    let team_a_uid = ServerId::from(1i64);
    let team_b_uid = ServerId::from(2i64);

    let workspace = workspace_with_members(vec![
        workspace_member("a", "a@warp.dev"),
        workspace_member("b", "b@warp.dev"),
    ]);
    let team = Team::from_local_cache(
        team_a_uid,
        "Team A".to_string(),
        None,
        None,
        Some(vec![team_member("a", "a@warp.dev")]),
    );

    let entries = vec![
        usage_entry("a", Some(team_a_uid)),
        usage_entry("b", Some(team_b_uid)),
        usage_entry("a", None),
    ];

    let (scoped_entries, scoped_members) =
        resolve_team_scoped_usage(Some(&team), &workspace, &entries)
            .expect("team resolved, should scope");

    assert_eq!(
        scoped_entries.len(),
        1,
        "only team A's attributed entry should remain"
    );
    assert_eq!(scoped_entries[0].subject_uid.as_deref(), Some("a"));

    assert_eq!(
        scoped_members.len(),
        1,
        "only team A's member should remain"
    );
    assert_eq!(scoped_members[0].email, "a@warp.dev");
}

// --- Route-level regression tests for `plan_usage_section` -----------------
//
// These exercise the same decision `render()` uses to pick what to build,
// with the same `entries`/`members` that get threaded into the header,
// legend, team totals, and per-member rows. A bug where some renderer
// bypassed the scoping (e.g. reading raw workspace entries directly) would
// not be caught by tests on `filter_entries_by_attributed_team` /
// `scope_members_to_team` alone, since those helpers were never wrong —
// the leak was in a caller not using their output. Asserting on the plan's
// output closes that gap.

#[test]
fn plan_usage_section_fails_closed_when_team_unresolved_for_non_own_only_visibility() {
    let workspace = with_full_breakdown_visibility(workspace_with_members(vec![
        workspace_member("a", "a@warp.dev"),
        workspace_member("b", "b@warp.dev"),
    ]));
    let entries = vec![usage_entry("a", Some(ServerId::from(1i64)))];

    // No team resolved for this window, but visibility requires one
    // (is_admin=true with a FullBreakdown policy) -- must fail closed.
    let plan = plan_usage_section(&workspace, None, true, Some("a"), &entries);

    assert!(
        matches!(plan, UsageSectionPlan::Empty),
        "expected Empty plan (no usage UI at all) when the team can't be resolved, got {plan:?}"
    );
}

#[test]
fn plan_usage_section_team_usage_excludes_other_teams_cost_type_categories() {
    // Team A only ever spends BaseLimit credits; Team B (a different team in
    // the same workspace) spends Payg credits attributed to itself. Team A's
    // admin must never see "Pay-as-you-go" surface from the plan's entries --
    // this is the render-level regression for the bug where the header's
    // legend read raw workspace-wide entries directly, bypassing the filter.
    let team_a_uid = ServerId::from(1i64);
    let team_b_uid = ServerId::from(2i64);

    // Team A has two members ("a", "c") so the plan routes to the full
    // TeamUsage block rather than collapsing to the own-usage view; "b"
    // belongs to a different team in the same workspace.
    let workspace = with_full_breakdown_visibility(workspace_with_members(vec![
        workspace_member("a", "a@warp.dev"),
        workspace_member("b", "b@warp.dev"),
        workspace_member("c", "c@warp.dev"),
    ]));
    let team_a = Team::from_local_cache(
        team_a_uid,
        "Team A".to_string(),
        None,
        None,
        Some(vec![
            team_member("a", "a@warp.dev"),
            team_member("c", "c@warp.dev"),
        ]),
    );

    let entries = vec![
        usage_entry_with_cost_type("a", Some(team_a_uid), AiCostType::BaseLimit),
        usage_entry_with_cost_type("b", Some(team_b_uid), AiCostType::Payg),
    ];

    let plan = plan_usage_section(&workspace, Some(&team_a), true, Some("a"), &entries);

    let UsageSectionPlan::TeamUsage {
        entries: scoped_entries,
        ..
    } = plan
    else {
        panic!("expected TeamUsage plan, got {plan:?}");
    };

    assert_eq!(
        scoped_entries.len(),
        1,
        "team B's entry must be filtered out of team A's plan"
    );

    let legend = legend_cost_types(&scoped_entries);
    assert!(
        !legend.contains(&AiCostType::Payg),
        "team A's legend must not include team B's Pay-as-you-go category, got {legend:?}"
    );
    assert_eq!(legend, vec![AiCostType::BaseLimit]);
}

#[test]
fn plan_usage_section_own_only_visibility_never_reaches_team_usage() {
    // Default visibility (no usage_visibility_policy configured) is OwnOnly,
    // regardless of admin status -- must never route to the Team block.
    let workspace = workspace_with_members(vec![
        workspace_member("a", "a@warp.dev"),
        workspace_member("b", "b@warp.dev"),
    ]);
    let entries = vec![usage_entry("a", None), usage_entry("b", None)];

    let plan = plan_usage_section(&workspace, None, false, Some("a"), &entries);

    assert!(
        matches!(plan, UsageSectionPlan::OwnUsageWithWorkspace { .. }),
        "expected OwnUsageWithWorkspace plan for OwnOnly visibility, got {plan:?}"
    );
}
