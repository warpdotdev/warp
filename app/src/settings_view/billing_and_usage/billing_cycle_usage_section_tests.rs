use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView, ViewHandle};

use super::{BillingCycleUsageAction, BillingCycleUsageSectionView};
use crate::ai::AIRequestUsageModel;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::network::NetworkStatus;
use crate::server::ids::ServerId;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::server_api::ServerApiProvider;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{
    BillingCycleUsageData, BillingCycleUsageSummary, Workspace, WorkspaceUid,
};

fn test_period_ends() -> Vec<DateTime<Utc>> {
    let current_period_end = Utc.with_ymd_and_hms(2026, 6, 13, 0, 0, 0).unwrap();
    (0..3)
        .map(|cycles_ago| current_period_end - Duration::days(30 * cycles_ago))
        .collect()
}

fn workspace_with_cycles(period_ends: &[DateTime<Utc>]) -> Workspace {
    let server_id: ServerId = 1_i64.into();
    let mut workspace = Workspace::from_local_cache(
        WorkspaceUid::from(server_id),
        "Test Workspace".to_string(),
        None,
    );
    workspace.billing_cycle_usage = Some(BillingCycleUsageData {
        current_period_start: period_ends[0] - Duration::days(30),
        current_period_end: period_ends[0],
        summaries: period_ends
            .iter()
            .map(|&period_end| BillingCycleUsageSummary {
                period_start: period_end - Duration::days(30),
                period_end,
                entries: vec![],
            })
            .collect(),
    });
    workspace
}

fn add_section_view(
    app: &mut App,
    workspace: Workspace,
) -> ViewHandle<BillingCycleUsageSectionView> {
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(TeamUpdateManager::mock);
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            vec![workspace],
            ctx,
        )
    });
    let (_, section) = app.add_window(
        WindowStyle::NotStealFocus,
        BillingCycleUsageSectionView::new,
    );
    section
}

#[test]
fn opening_period_menu_highlights_current_period_by_default() {
    App::test((), |mut app| async move {
        let period_ends = test_period_ends();
        let section = add_section_view(&mut app, workspace_with_cycles(&period_ends));

        let period_menu = section.update(&mut app, |section, ctx| {
            section.handle_action(&BillingCycleUsageAction::TogglePeriodMenu, ctx);
            assert!(section.period_menu_open);
            section.period_menu.clone()
        });

        period_menu.read(&app, |menu, _| {
            assert_eq!(menu.items_len(), 3);
            assert_eq!(menu.selected_index(), Some(0));
        });
    });
}

#[test]
fn opening_period_menu_highlights_explicitly_selected_period() {
    App::test((), |mut app| async move {
        let period_ends = test_period_ends();
        let section = add_section_view(&mut app, workspace_with_cycles(&period_ends));

        let period_menu = section.update(&mut app, |section, ctx| {
            section.handle_action(
                &BillingCycleUsageAction::SelectPeriod(Some(period_ends[1])),
                ctx,
            );
            section.handle_action(&BillingCycleUsageAction::TogglePeriodMenu, ctx);
            assert!(section.period_menu_open);
            section.period_menu.clone()
        });

        period_menu.read(&app, |menu, _| {
            assert_eq!(menu.selected_index(), Some(1));
        });
    });
}
