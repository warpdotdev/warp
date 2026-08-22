use std::sync::Arc;

use chrono::TimeZone;
use settings::PrivatePreferences;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle, WindowId};

use super::*;
use crate::ai::request_usage_model::AIRequestUsageModel;
use crate::auth::auth_manager::AuthManager;
use crate::auth::{AuthStateProvider, UserUid};
use crate::network::NetworkStatus;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::team::{MembershipRole, Team, TeamMember, TeamVisibility};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageData, WorkspaceMemberUsageInfo,
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

// ── Section-level team scoping ──────────────────────────────────────────
//
// `visible_entries`/`visible_members` are where this vertical's window-to-team resolution
// actually gets exercised end to end, so the predicate-level coverage in
// `user_workspaces_tests.rs` alone doesn't prove this view never mixes teams.

fn team_for_test(uid: i64, name: &str, member_uid: &str, member_email: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_string(),
        color: None,
        invite_link: None,
        members: vec![TeamMember {
            uid: UserUid::new(member_uid),
            email: member_email.to_string(),
            role: MembershipRole::Owner,
        }],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
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

fn entry_attributed_to(team_uid: Option<String>) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: None,
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used: 10,
        cost_cents: 0,
        attributed_team_uid: team_uid,
    }
}

/// A workspace with two teams (A, B), a workspace-wide roster covering both teams' members,
/// and one billing-cycle-usage entry attributed to each team plus one unattributed entry —
/// mirroring the real shape of `billingCycleUsageHistory`, which is workspace-wide and relies
/// on `attributed_team_uid` / team membership alone to scope to one team.
fn two_team_workspace() -> (Team, Team, Workspace) {
    let team_a = team_for_test(100, "Team A", "admin-a", "admin-a@example.com");
    let team_b = team_for_test(200, "Team B", "admin-b", "admin-b@example.com");

    let members = vec![
        workspace_member("admin-a", "admin-a@example.com"),
        workspace_member("admin-b", "admin-b@example.com"),
    ];
    let entries = vec![
        entry_attributed_to(Some(team_a.uid.to_string())),
        entry_attributed_to(Some(team_b.uid.to_string())),
        entry_attributed_to(None),
    ];

    let workspace = Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![team_a.clone(), team_b.clone()],
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: Some(BillingCycleUsageData {
            current_period_start: utc(2026, 1, 1),
            current_period_end: utc(2026, 2, 1),
            summaries: vec![BillingCycleUsageSummary {
                period_start: utc(2026, 1, 1),
                period_end: utc(2026, 2, 1),
                entries,
            }],
        }),
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members,
        total_requests_used_since_last_refresh: 0,
    };
    (team_a, team_b, workspace)
}

fn initialize_app(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(|_| warp_core::ui::appearance::Appearance::mock());
    app.add_singleton_model(crate::settings::PrivacySettings::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    if app.models_of_type::<PrivatePreferences>().is_empty() {
        app.update(crate::settings::init_and_register_user_preferences);
    }
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

fn create_section_window(app: &mut App) -> (WindowId, ViewHandle<BillingCycleUsageSectionView>) {
    app.add_window(
        WindowStyle::NotStealFocus,
        BillingCycleUsageSectionView::new,
    )
}

fn attributed_uids(entries: &[BillingCycleUsageEntry]) -> Vec<Option<String>> {
    entries
        .iter()
        .map(|e| e.attributed_team_uid.clone())
        .collect()
}

fn member_emails(members: &[WorkspaceMember]) -> Vec<String> {
    members.iter().map(|m| m.email.clone()).collect()
}

/// Team A's window sees only team A's entries and members; team B's window (a second window in
/// the same app, reading the same workspace-wide data) sees only team B's — neither ever mixes
/// in the other team's data or the unattributed entry.
#[test]
fn visible_entries_and_members_scope_to_each_windows_own_team() {
    let (team_a, team_b, workspace) = two_team_workspace();

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace.clone()]);
        let (window_a, view_a) = create_section_window(&mut app);
        let (window_b, view_b) = create_section_window(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_a, team_a.uid, ctx);
            user_workspaces.set_team_for_window(window_b, team_b.uid, ctx);
        });

        app.read(|ctx| {
            let entries_a = view_a.as_ref(ctx).visible_entries(&workspace, ctx);
            assert_eq!(
                attributed_uids(&entries_a),
                vec![Some(team_a.uid.to_string())],
                "team A's window must see only team A's usage entry"
            );
            assert_eq!(
                member_emails(&view_a.as_ref(ctx).visible_members(&workspace, ctx)),
                vec!["admin-a@example.com".to_string()],
                "team A's window must see only team A's member"
            );

            let entries_b = view_b.as_ref(ctx).visible_entries(&workspace, ctx);
            assert_eq!(
                attributed_uids(&entries_b),
                vec![Some(team_b.uid.to_string())],
                "team B's window must see only team B's usage entry"
            );
            assert_eq!(
                member_emails(&view_b.as_ref(ctx).visible_members(&workspace, ctx)),
                vec!["admin-b@example.com".to_string()],
                "team B's window must see only team B's member"
            );
        });
    })
}

/// When a window's team assignment changes (team A leaves the workspace, so the window
/// reconciles onto the only remaining team), the section's visible entries and members move
/// with it rather than staying pinned to the original team or mixing both.
#[test]
fn visible_entries_and_members_follow_a_window_team_change() {
    let (team_a, team_b, workspace) = two_team_workspace();

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![workspace.clone()]);
        let (window_id, view) = create_section_window(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.set_team_for_window(window_id, team_a.uid, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                attributed_uids(&view.as_ref(ctx).visible_entries(&workspace, ctx)),
                vec![Some(team_a.uid.to_string())],
                "before the change, the window should see team A's entry"
            );
        });

        // Team A leaves the workspace; the window's assignment reconciles onto team B.
        let mut workspace_without_team_a = workspace.clone();
        workspace_without_team_a
            .teams
            .retain(|t| t.uid != team_a.uid);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(vec![workspace_without_team_a.clone()], ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                UserWorkspaces::as_ref(ctx).team_uid_for_window(window_id),
                Some(team_b.uid),
                "the window should have reconciled onto team B"
            );
            let entries = view
                .as_ref(ctx)
                .visible_entries(&workspace_without_team_a, ctx);
            assert_eq!(
                attributed_uids(&entries),
                vec![Some(team_b.uid.to_string())],
                "after the change, the same window should see only team B's entry, not team A's"
            );
            assert_eq!(
                member_emails(
                    &view
                        .as_ref(ctx)
                        .visible_members(&workspace_without_team_a, ctx)
                ),
                vec!["admin-b@example.com".to_string()],
                "after the change, the same window should see only team B's member"
            );
        });
    })
}

/// A teamless viewer (no team resolved for the window) gets the unfiltered, workspace-wide
/// entries and members rather than an artificially scoped-down (or scoped-to-the-wrong-team)
/// view — there is no team to exclude data on behalf of.
#[test]
fn visible_entries_and_members_fall_back_to_unfiltered_without_a_team() {
    let (_team_a, _team_b, workspace) = two_team_workspace();

    App::test((), |mut app| async move {
        initialize_app(&mut app, vec![]);
        let (window_id, view) = create_section_window(&mut app);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                view.as_ref(ctx).visible_entries(&workspace, ctx).len(),
                3,
                "a teamless window should see every entry, including the unattributed one"
            );
            assert_eq!(
                view.as_ref(ctx).visible_members(&workspace, ctx).len(),
                2,
                "a teamless window should see the full workspace roster"
            );
        });
    })
}
