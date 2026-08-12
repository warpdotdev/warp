use std::sync::Arc;

use chrono::TimeZone;
use warpui::platform::WindowStyle;
use warpui::{AddSingletonModel, App, WindowId};

use super::*;
use crate::auth::UserUid;
use crate::network::NetworkStatus;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::PrivacySettings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageData, BillingCycleUsageEntry, WorkspaceMember,
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

// ============================================================================
// Team scoping
//
// These exercise the whole chain the leak ran through: the window's team is
// resolved through `UserWorkspaces::team_for_view_handle`, and that team is
// what narrows the workspace-wide usage history and roster. Testing the pure
// helpers alone would not catch the filter being dropped from `scoped_usage`.
// ============================================================================

const ALICE: &str = "alice-uid";
const BOB: &str = "bob-uid";

fn team(uid_seed: i64, member_uids: &[&str]) -> Team {
    Team {
        uid: uid_seed.into(),
        name: format!("team-{uid_seed}"),
        color: None,
        invite_code: None,
        members: member_uids
            .iter()
            .map(|uid| TeamMember {
                uid: UserUid::new(uid),
                email: format!("{uid}@warp.dev"),
                role: MembershipRole::User,
            })
            .collect(),
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@warp.dev"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 100,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn usage_entry(
    subject_uid: &str,
    attributed_team: Option<&Team>,
    credits_used: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type: AiCreditsUsageAndCostType::BaseLimit,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents: 0,
        attributed_team_uid: attributed_team.map(|team| team.uid.uid()),
    }
}

fn workspace_with(
    teams: Vec<Team>,
    members: Vec<WorkspaceMember>,
    entries: Vec<BillingCycleUsageEntry>,
) -> Workspace {
    let mut workspace = Workspace::from_local_cache(
        "workspace_uid123456789".to_string().into(),
        "test".to_string(),
        Some(teams),
    );
    workspace.members = members;
    workspace.billing_cycle_usage = Some(BillingCycleUsageData {
        current_period_start: utc(2026, 6, 27),
        current_period_end: utc(2026, 7, 27),
        summaries: vec![BillingCycleUsageSummary {
            period_start: utc(2026, 6, 27),
            period_end: utc(2026, 7, 27),
            entries,
        }],
    });
    workspace
}

/// Registers the singletons `BillingCycleUsageSectionView` subscribes to on
/// construction, plus the auth state `scoped_usage` reads the viewer from.
fn init_section_test_app(app: &mut App, workspaces: Vec<Workspace>) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    // `TeamUpdateManager::new` subscribes to both of these on construction.
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::new);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

/// Opens a window rooted at the usage section and points it at `team`.
fn open_section_window(
    app: &mut App,
    team: &Team,
) -> (WindowId, ViewHandle<BillingCycleUsageSectionView>) {
    let (window_id, view) = app.add_window(
        WindowStyle::NotStealFocus,
        BillingCycleUsageSectionView::new,
    );
    let team_uid = team.uid;
    UserWorkspaces::handle(app).update(app, |user_workspaces, ctx| {
        user_workspaces.register_window(window_id, Some(team_uid), ctx);
    });
    (window_id, view)
}

fn credits(scoped: &TeamScopedUsage) -> Vec<i32> {
    scoped.entries.iter().map(|e| e.credits_used).collect()
}

fn member_uids(scoped: &TeamScopedUsage) -> Vec<String> {
    scoped.members.iter().map(|m| m.uid.as_string()).collect()
}

#[test]
fn scoped_usage_follows_the_team_the_window_is_pointed_at() {
    // The bug in one picture: one workspace, two teams, one usage history.
    // Each window is pointed at a different team and must see only that
    // team's usage and only that team's members.
    let team_a = team(1, &[ALICE]);
    let team_b = team(2, &[BOB]);
    let workspace = workspace_with(
        vec![team_a.clone(), team_b.clone()],
        vec![workspace_member(ALICE), workspace_member(BOB)],
        vec![
            usage_entry(ALICE, Some(&team_a), 10),
            usage_entry(BOB, Some(&team_b), 999),
            usage_entry("carol-uid", None, 7),
        ],
    );

    App::test((), |mut app| async move {
        init_section_test_app(&mut app, vec![workspace.clone()]);
        let (_, section_a) = open_section_window(&mut app, &team_a);
        let (_, section_b) = open_section_window(&mut app, &team_b);

        app.read(|ctx| {
            let scoped = section_a.as_ref(ctx).scoped_usage(&workspace, ctx);
            assert_eq!(
                credits(&scoped),
                vec![10],
                "team A's window must not see team B's usage, nor the \
                 unattributed row"
            );
            assert_eq!(member_uids(&scoped), vec![ALICE.to_string()]);

            let scoped = section_b.as_ref(ctx).scoped_usage(&workspace, ctx);
            assert_eq!(
                credits(&scoped),
                vec![999],
                "the same workspace data, scoped the other way"
            );
            assert_eq!(member_uids(&scoped), vec![BOB.to_string()]);
        });
    })
}

#[test]
fn scoped_usage_keeps_the_viewers_own_usage_on_another_teams_page() {
    // A workspace admin gets every team in the workspace, and the settings
    // window defaults to the first one -- which may be a team they are not in
    // and where none of their usage is attributed. Their own row must still
    // show real numbers rather than zero.
    //
    // Bob's team-B row is the control: it is neither the viewer's own nor
    // team A's, so the carve-out must not widen into "show all of team B".
    let team_a = team(1, &[ALICE]);
    let team_b = team(2, &[BOB]);

    App::test((), |mut app| async move {
        init_section_test_app(&mut app, vec![]);
        let mut viewer_uid = String::new();
        app.update(|ctx| {
            viewer_uid = AuthStateProvider::as_ref(ctx)
                .get()
                .user_id()
                .expect("the test auth state should have a user")
                .as_string();
        });

        let workspace = workspace_with(
            vec![team_a.clone(), team_b.clone()],
            vec![workspace_member(ALICE), workspace_member(&viewer_uid)],
            vec![
                usage_entry(&viewer_uid, Some(&team_b), 42),
                usage_entry(ALICE, Some(&team_a), 10),
                usage_entry(BOB, Some(&team_b), 999),
            ],
        );
        let workspaces = vec![workspace.clone()];
        let workspace_uid = workspace.uid;
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.update_workspaces(workspaces, ctx);
            // `UserWorkspaces::mock` pins the current workspace at
            // construction and this app was built with none, so without this
            // the page resolves no workspace, no team, and silently exercises
            // the unscoped pass-through instead of the filter.
            user_workspaces.set_current_workspace_uid(workspace_uid, ctx);
        });
        let (_, section) = open_section_window(&mut app, &team_a);

        app.read(|ctx| {
            let scoped = section.as_ref(ctx).scoped_usage(&workspace, ctx);
            assert_eq!(
                credits(&scoped),
                vec![42, 10],
                "the viewer's own team-B usage survives on team A's page, \
                 alongside team A's own usage -- but Bob's team-B usage does \
                 not come along with it"
            );
        });
    })
}

#[test]
fn scoped_usage_passes_everything_through_with_no_team_in_view() {
    // Personal / no-team viewers have nothing to scope against.
    let workspace = workspace_with(
        vec![],
        vec![workspace_member(ALICE), workspace_member(BOB)],
        vec![usage_entry(ALICE, None, 10), usage_entry(BOB, None, 20)],
    );

    App::test((), |mut app| async move {
        init_section_test_app(&mut app, vec![workspace.clone()]);
        let (window_id, section) = app.add_window(
            WindowStyle::NotStealFocus,
            BillingCycleUsageSectionView::new,
        );
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, None, ctx);
        });

        app.read(|ctx| {
            let scoped = section.as_ref(ctx).scoped_usage(&workspace, ctx);
            assert_eq!(credits(&scoped), vec![10, 20]);
            assert_eq!(
                member_uids(&scoped),
                vec![ALICE.to_string(), BOB.to_string()]
            );
        });
    })
}
