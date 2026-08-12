use chrono::TimeZone;

use super::*;
use crate::auth::UserUid;
use crate::server::ids::ServerId;
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

// ── entries_for_team / team_has_multiple_members render-wiring regressions ─
//
// These exercise the exact functions `render_team_usage`,
// `render_own_usage_with_workspace`, and `render_visibility_cta_banner` call
// to scope workspace-wide state down to the current team, so a caller that
// stops routing through them (the class of bug this page previously had)
// is caught here rather than only at the pure-predicate level.

const VIEWER_UID: &str = "viewer-uid";
const TEAM_A_ID: i64 = 1;
const TEAM_B_ID: i64 = 2;

fn team_with_members(uid: i64, member_uids: &[&str]) -> Team {
    Team::from_local_cache(
        ServerId::from(uid),
        "Team".to_string(),
        None,
        None,
        Some(
            member_uids
                .iter()
                .map(|uid| TeamMember {
                    uid: UserUid::new(uid),
                    email: format!("{uid}@example.com"),
                    role: MembershipRole::User,
                })
                .collect(),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn entry_for_team(
    subject_uid: &str,
    attributed_team_uid: &str,
    cost_type: AiCreditsUsageAndCostType,
    credits_used: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        attributed_team_uid: Some(attributed_team_uid.to_string()),
        cost_type,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents: 0,
    }
}

#[test]
fn entries_for_team_excludes_a_viewers_own_entries_on_another_team() {
    // Regression: a viewer who belongs to both teams must not see their
    // Team B usage bleed into Team A's window, even though a subject-uid-only
    // filter (like `MemberUsageRow::for_viewer`) would treat both entries as
    // "theirs" and merge them together.
    let team_a = team_with_members(TEAM_A_ID, &[VIEWER_UID]);
    let team_a_uid = team_a.uid.to_string();
    let team_b_uid = ServerId::from(TEAM_B_ID).to_string();
    let raw_entries = vec![
        entry_for_team(
            VIEWER_UID,
            &team_a_uid,
            AiCreditsUsageAndCostType::BaseLimit,
            10,
        ),
        entry_for_team(
            VIEWER_UID,
            &team_b_uid,
            AiCreditsUsageAndCostType::BaseLimit,
            999,
        ),
    ];

    let scoped = entries_for_team(&raw_entries, Some(&team_a));

    assert_eq!(scoped.len(), 1);
    assert_eq!(
        scoped[0].attributed_team_uid.as_deref(),
        Some(team_a_uid.as_str())
    );
    assert_eq!(scoped[0].credits_used, 10);
}

#[test]
fn entries_for_team_fails_closed_when_team_unresolved() {
    let team_a_uid = ServerId::from(TEAM_A_ID).to_string();
    let raw_entries = vec![entry_for_team(
        VIEWER_UID,
        &team_a_uid,
        AiCreditsUsageAndCostType::BaseLimit,
        10,
    )];

    assert!(
        entries_for_team(&raw_entries, None).is_empty(),
        "must never fall back to showing unscoped, workspace-wide entries"
    );
}

#[test]
fn entries_for_team_scopes_legend_cost_types_to_current_team() {
    // Regression: the legend must reflect only the current team's cost
    // types, not a sibling team's, even when both are present in the raw
    // workspace-wide entries for the period.
    let team_a = team_with_members(TEAM_A_ID, &[VIEWER_UID]);
    let team_a_uid = team_a.uid.to_string();
    let team_b_uid = ServerId::from(TEAM_B_ID).to_string();
    let raw_entries = vec![
        entry_for_team(
            VIEWER_UID,
            &team_a_uid,
            AiCreditsUsageAndCostType::BaseLimit,
            10,
        ),
        entry_for_team(
            "other-uid",
            &team_b_uid,
            AiCreditsUsageAndCostType::BonusGrant,
            50,
        ),
    ];

    let scoped = entries_for_team(&raw_entries, Some(&team_a));

    assert_eq!(
        legend_cost_types(&scoped),
        vec![AiCreditsUsageAndCostType::BaseLimit],
        "legend must not include a cost type only a sibling team's usage justifies"
    );
}

#[test]
fn entries_for_team_hides_legend_when_only_sibling_team_has_usage() {
    // Regression: Team A must not show a legend at all when Team A itself
    // has no qualifying usage, even if Team B (sharing the workspace) does.
    let team_a = team_with_members(TEAM_A_ID, &[VIEWER_UID]);
    let team_b_uid = ServerId::from(TEAM_B_ID).to_string();
    let raw_entries = vec![entry_for_team(
        "other-uid",
        &team_b_uid,
        AiCreditsUsageAndCostType::BonusGrant,
        50,
    )];

    let scoped = entries_for_team(&raw_entries, Some(&team_a));

    assert!(
        legend_cost_types(&scoped).is_empty(),
        "a team with no usage of its own must not show a legend justified by a sibling team"
    );
}

#[test]
fn team_has_multiple_members_true_for_multi_member_team() {
    let team = team_with_members(TEAM_A_ID, &[VIEWER_UID, "other-uid"]);
    assert!(team_has_multiple_members(Some(&team)));
}

#[test]
fn team_has_multiple_members_false_for_solo_team_despite_multi_member_workspace() {
    // Regression: a solo Team A must not get the team-level visibility CTA
    // just because sibling Team B (in the same workspace) has other members.
    let solo_team_a = team_with_members(TEAM_A_ID, &[VIEWER_UID]);
    assert!(!team_has_multiple_members(Some(&solo_team_a)));
}

#[test]
fn team_has_multiple_members_fails_closed_when_team_unresolved() {
    assert!(!team_has_multiple_members(None));
}
