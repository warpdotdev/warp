use std::sync::Arc;

use chrono::TimeZone;
use settings::PrivatePreferences;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::auth::UserUid;
use crate::network::NetworkStatus;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::billing_and_usage::billing_cycle_usage_team_totals::build_team_total_card_summaries;
use crate::workspaces::team::{MembershipRole, TeamMember};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageData, BillingMetadata, UsageVisibilityPolicy,
    WorkspaceMember, WorkspaceMemberUsageInfo,
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

// `ServerId::from_string_lossy` requires exactly 22 characters.
const TEAM_A_UID: &str = "team_a_uid000000000000";
const TEAM_B_UID: &str = "team_b_uid000000000000";
const TEAM_A_ADMIN_UID: &str = warp_server_auth::user_uid::TEST_USER_UID;
const TEAM_A_ADMIN_EMAIL: &str = warp_server_auth::user_uid::TEST_USER_EMAIL;
const TEAM_A_MEMBER_UID: &str = "team-a-zero-usage-user";
const TEAM_B_MEMBER_UID: &str = "team-b-user";

fn workspace_member(uid: &str, email: &str, role: MembershipRole) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 0,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn team_member(uid: &str, email: &str, role: MembershipRole) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: email.to_string(),
        role,
    }
}

fn usage_entry(
    subject_uid: &str,
    cost_type: AiCreditsUsageAndCostType,
    credits_used: i32,
    attributed_team_uid: &str,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::User,
        subject_uid: Some(subject_uid.to_string()),
        subject_display_name: None,
        cost_type,
        usage_bucket: AiCreditsUsageBucket::Ai,
        usage_source: AiCreditsUsageSource::Local,
        credits_used,
        cost_cents: 0,
        attributed_team_uid: Some(attributed_team_uid.to_string()),
    }
}

/// Registers only the singleton models `BillingCycleUsageSectionView::new`
/// touches (directly or transitively), skipping the ~100-singleton
/// `WorkspaceView` harness that would otherwise be required.
fn initialize_minimal_app(app: &mut App, workspace: Workspace) {
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| {
        PrivatePreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(TeamUpdateManager::mock);
}

/// Builds a two-team native workspace: team A (the team currently being
/// viewed, with the test-user as owner/admin plus a zero-usage teammate) and
/// team B (a sibling team with its own member and its own, distinct cost
/// category). `Workspace.members` and `Workspace.billing_cycle_usage` are
/// workspace-wide, spanning both teams, exactly as the server returns them.
fn two_team_workspace() -> Workspace {
    let team_a_uid = ServerId::from_string_lossy(TEAM_A_UID);
    let team_b_uid = ServerId::from_string_lossy(TEAM_B_UID);

    let team_a = Team::from_local_cache(
        team_a_uid,
        "Team A".to_string(),
        None,
        None,
        Some(vec![
            team_member(TEAM_A_ADMIN_UID, TEAM_A_ADMIN_EMAIL, MembershipRole::Owner),
            team_member(
                TEAM_A_MEMBER_UID,
                "a-member@example.com",
                MembershipRole::User,
            ),
        ]),
    );
    let team_b = Team::from_local_cache(
        team_b_uid,
        "Team B".to_string(),
        None,
        None,
        Some(vec![team_member(
            TEAM_B_MEMBER_UID,
            "b-member@example.com",
            MembershipRole::User,
        )]),
    );

    let mut billing_metadata = BillingMetadata::default();
    billing_metadata.tier.usage_visibility_policy = Some(UsageVisibilityPolicy {
        admin_granularity: UsageVisibilityGranularity::FullBreakdown,
        max_prior_cycles: MaxPriorCycles::None,
    });

    let period_start = utc(2026, 6, 27);
    let period_end = utc(2026, 7, 27);
    let entries = vec![
        // Team A: the admin's own BaseLimit usage.
        usage_entry(
            TEAM_A_ADMIN_UID,
            AiCreditsUsageAndCostType::BaseLimit,
            100,
            TEAM_A_UID,
        ),
        // Team B: a distinct cost category (Payg) that must not leak into
        // team A's legend, totals, or member rows.
        usage_entry(
            TEAM_B_MEMBER_UID,
            AiCreditsUsageAndCostType::Payg,
            500,
            TEAM_B_UID,
        ),
    ];

    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "Test Workspace".to_string(),
        stripe_customer_id: None,
        teams: vec![team_a, team_b],
        billing_metadata,
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: Some(BillingCycleUsageData {
            current_period_start: period_start,
            current_period_end: period_end,
            summaries: vec![BillingCycleUsageSummary {
                period_start,
                period_end,
                entries,
            }],
        }),
        has_billing_history: false,
        settings: Default::default(),
        invite_code: None,
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        // Workspace-wide roster spans both teams, mirroring the server.
        members: vec![
            workspace_member(TEAM_A_ADMIN_UID, TEAM_A_ADMIN_EMAIL, MembershipRole::Owner),
            workspace_member(
                TEAM_A_MEMBER_UID,
                "a-member@example.com",
                MembershipRole::User,
            ),
            workspace_member(
                TEAM_B_MEMBER_UID,
                "b-member@example.com",
                MembershipRole::User,
            ),
        ],
        total_requests_used_since_last_refresh: 0,
    }
}

/// Regression test for the composed data flow through
/// `BillingCycleUsageSectionView`: a native workspace with teams A and B,
/// the current window scoped to team A, and team B carrying its own
/// distinct cost category and member. Every surface the section feeds
/// (legend categories, team totals, per-user rows, and the zero-usage
/// roster) must be scoped to team A alone.
#[test]
fn section_scopes_every_surface_to_the_current_team() {
    App::test((), |mut app| async move {
        let workspace = two_team_workspace();
        initialize_minimal_app(&mut app, workspace.clone());

        let (window_id, view_handle) = app
            .add_window::<BillingCycleUsageSectionView, _>(WindowStyle::Normal, |ctx| {
                BillingCycleUsageSectionView::new(ctx)
            });

        let team_a_uid = ServerId::from_string_lossy(TEAM_A_UID);
        UserWorkspaces::handle(&app).update(&mut app, |user_workspaces, ctx| {
            user_workspaces.register_window(window_id, Some(team_a_uid), ctx);
        });

        view_handle.read(&app, |view, app_ctx| {
            // The view must resolve the window's team as team A, and the
            // test user (team A's owner) as an admin of it.
            assert_eq!(
                view.current_team(app_ctx).map(|team| team.uid),
                Some(team_a_uid)
            );
            assert!(view.viewer_is_team_admin(app_ctx));

            // Entries: only team A's should survive, scoped through the
            // view's own composition method (the exact seam that
            // previously left the legend unscoped).
            let entries = view.current_scoped_entries(&workspace, app_ctx);
            assert_eq!(entries.len(), 1, "only team A's entry should remain");
            assert_eq!(entries[0].subject_uid.as_deref(), Some(TEAM_A_ADMIN_UID));
            assert_eq!(entries[0].cost_type, AiCreditsUsageAndCostType::BaseLimit);

            // Legend: team B's Payg category must not leak in.
            let legend_types = legend_cost_types(&entries);
            assert_eq!(legend_types, vec![AiCreditsUsageAndCostType::BaseLimit]);
            assert!(!legend_types.contains(&AiCreditsUsageAndCostType::Payg));

            // Team totals: must reflect only team A's 100 credits, not team
            // B's additional 500.
            let visibility = workspace.resolve_usage_visibility(true);
            let totals = build_team_total_card_summaries(&entries, &visibility);
            let overall = totals
                .iter()
                .find(|s| s.card_key == "__card_overall__")
                .expect("overall card present");
            assert_eq!(overall.total_credits, 100);

            // Members: per-user rows and the zero-usage roster must be
            // narrowed to team A's roster (including the zero-usage
            // teammate), excluding team B's member entirely.
            let members = workspace_members_for_team(&workspace.members, view.current_team(app_ctx).unwrap());
            let member_uids: Vec<_> = members.iter().map(|m| m.uid).collect();
            assert!(member_uids.contains(&UserUid::new(TEAM_A_ADMIN_UID)));
            assert!(
                member_uids.contains(&UserUid::new(TEAM_A_MEMBER_UID)),
                "zero-usage team A member must still appear in the roster"
            );
            assert!(
                !member_uids.contains(&UserUid::new(TEAM_B_MEMBER_UID)),
                "team B's member must not leak into team A's roster"
            );

            // End-to-end: render the actual top-level path (`render_team_usage`
            // is exactly what `render()` dispatches to for a multi-member
            // team) and inspect the rendered text. This is the regression
            // check for the specific bug: `render_legend` previously read the
            // raw, unscoped summary entries directly instead of the same
            // `entries` threaded through every other surface, so team B's
            // "Pay-as-you-go" category and member leaked into the rendered
            // output even though the amounts were hidden.
            let appearance = Appearance::as_ref(app_ctx);
            let rendered = view.render_team_usage(&workspace, appearance, app_ctx);
            let text = rendered.debug_text_content().unwrap_or_default();
            assert!(
                text.contains("Base"),
                "expected team A's Base category in the rendered output, got: {text}"
            );
            assert!(
                !text.contains("Pay-as-you-go"),
                "team B's Pay-as-you-go category must not leak into the rendered output, got: {text}"
            );
            assert!(
                text.contains(TEAM_A_ADMIN_EMAIL) || text.contains("a-member@example.com"),
                "expected team A's own members in the rendered output, got: {text}"
            );
            assert!(
                !text.contains("b-member@example.com"),
                "team B's member must not leak into the rendered output, got: {text}"
            );
        });
    })
}
