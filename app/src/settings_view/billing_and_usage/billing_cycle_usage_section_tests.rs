use chrono::TimeZone;

use super::*;
use crate::auth::UserUid;
use crate::server::ids::ServerId;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageBucket, AiCreditsUsageSource,
    WorkspaceMemberUsageInfo,
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

fn usage_entry(subject_uid: &str, attributed_team_uid: Option<&str>) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used: 10,
        cost_cents: 5,
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
    }
}

#[test]
fn filter_entries_by_attributed_team_keeps_only_matching_team() {
    let entries = vec![
        usage_entry("a-member", Some("team-a")),
        usage_entry("b-member", Some("team-b")),
        usage_entry("unassigned", None),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, "team-a");

    let subject_uids: Vec<&str> = filtered
        .iter()
        .map(|e| e.subject_uid.as_deref().unwrap())
        .collect();
    assert_eq!(subject_uids, ["a-member"]);
}

#[test]
fn filter_entries_by_attributed_team_keeps_service_accounts_attributed_to_team() {
    let mut service_account = usage_entry("service-account-a", Some("team-a"));
    service_account.subject_type = AiCreditsUsageAndCostSubjectType::ServiceAccount;
    let entries = vec![
        service_account,
        usage_entry("service-account-b", Some("team-b")),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, "team-a");

    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered[0].subject_uid.as_deref(),
        Some("service-account-a")
    );
}

fn team_member(uid: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
    }
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@example.com"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 1000,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn server_id(seed: &str) -> ServerId {
    ServerId::from_string_lossy(format!("{seed:0>22}"))
}

fn team(uid: &str, members: Vec<TeamMember>) -> Team {
    Team::from_local_cache(
        server_id(uid),
        format!("team-{uid}"),
        None,
        None,
        Some(members),
    )
}

/// This is the raw-to-scoped boundary every v2 rendering path reads
/// through (see [`ActiveTeamUsageScope`]). Starting from raw, mixed-team
/// data -- exactly what `workspace.billing_cycle_usage` /
/// `workspace.members` actually contain -- so this fails if the
/// production entry or roster filtering is ever removed or miswired.
#[test]
fn resolve_active_team_scope_keeps_only_active_teams_entries_and_roster() {
    let team_a = team("team-a", vec![team_member("a-only"), team_member("shared")]);
    let team_a_uid = team_a.uid.uid();
    let team_b_uid = server_id("team-b").uid();
    let workspace_members = vec![
        workspace_member("a-only"),
        workspace_member("b-only"),
        workspace_member("shared"),
    ];
    let mut service_account_b = usage_entry("service-account-b", Some(team_b_uid.as_str()));
    service_account_b.subject_type = AiCreditsUsageAndCostSubjectType::ServiceAccount;
    let raw_entries = vec![
        usage_entry("a-only", Some(team_a_uid.as_str())),
        usage_entry("shared", Some(team_a_uid.as_str())),
        usage_entry("b-only", Some(team_b_uid.as_str())),
        usage_entry("unassigned", None),
        service_account_b,
    ];

    let scope = resolve_active_team_scope(&raw_entries, &workspace_members, Some(&team_a));

    let entry_uids: Vec<&str> = scope
        .entries
        .iter()
        .map(|e| e.subject_uid.as_deref().unwrap())
        .collect();
    assert_eq!(entry_uids, ["a-only", "shared"]);

    let member_uids: Vec<&str> = scope.members.iter().map(|m| m.uid.as_str()).collect();
    assert_eq!(member_uids, ["a-only", "shared"]);
}

#[test]
fn resolve_active_team_scope_with_no_active_team_yields_empty_scope() {
    // Fail closed: no resolved active team must never fall back to the
    // whole workspace's entries or roster.
    let workspace_members = vec![workspace_member("a-only")];
    let team_a_uid = server_id("team-a").uid();
    let raw_entries = vec![usage_entry("a-only", Some(team_a_uid.as_str()))];

    let scope = resolve_active_team_scope(&raw_entries, &workspace_members, None);

    assert!(scope.entries.is_empty());
    assert!(scope.members.is_empty());
}

#[test]
fn resolve_active_team_scope_with_only_other_team_entries_yields_empty_entries() {
    // A member whose only usage in the cycle is attributed to a different
    // team must never surface that other team's data on this page.
    let team_a = team("team-a", vec![team_member("a-only")]);
    let team_b_uid = server_id("team-b").uid();
    let workspace_members = vec![workspace_member("a-only")];
    let raw_entries = vec![usage_entry("a-only", Some(team_b_uid.as_str()))];

    let scope = resolve_active_team_scope(&raw_entries, &workspace_members, Some(&team_a));

    assert!(scope.entries.is_empty());
    assert_eq!(scope.members.len(), 1);
}
