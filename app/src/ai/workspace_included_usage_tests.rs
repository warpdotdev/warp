use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use warpui::App;

use super::*;
use crate::server::ids::ServerId;
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::workspace::{Workspace, WorkspaceUid};

fn workspace_uid(workspace: i64) -> WorkspaceUid {
    WorkspaceUid::from(ServerId::from(workspace))
}

fn allowances() -> HashMap<WorkspaceUid, WorkspaceIncludedUsageInfo> {
    let mut second = pool();
    second.included_usage_credits = 2000.0;
    HashMap::from([(workspace_uid(1), pool()), (workspace_uid(2), second)])
}

fn pool() -> WorkspaceIncludedUsageInfo {
    WorkspaceIncludedUsageInfo {
        included_usage_credits: 1234.75,
        usage_credits_used_since_last_refresh: 250.125,
        included_usage_cents: 10_000.0,
        usage_cents_used_since_last_refresh: 1250.25,
        next_refresh_time: (Utc.with_ymd_and_hms(2026, 11, 6, 0, 0, 0).unwrap()
            + Duration::days(1))
        .into(),
    }
}

#[test]
fn included_usage_refreshes_with_grants_and_selects_cached_workspaces() {
    App::test((), |mut app| async move {
        app.update(crate::settings::init_and_register_user_preferences);
        app.add_singleton_model(crate::settings::PrivacySettings::mock);
        app.add_singleton_model(crate::settings::AISettings::new_with_defaults);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|ctx| {
            let workspaces = [1, 2, 3]
                .into_iter()
                .map(|id| {
                    let mut workspace = Workspace::from_local_cache(
                        workspace_uid(id),
                        format!("Scale {id}"),
                        None,
                        None,
                    );
                    workspace.billing_metadata.customer_type = CustomerType::Scale;
                    workspace
                })
                .collect();
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                workspaces,
                ctx,
            )
        });
        let mut client = MockAIClient::new();
        let mut sequence = mockall::Sequence::new();
        client
            .expect_get_request_limit_info()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| {
                Ok(super::super::RequestUsageInfo {
                    request_limit_info: super::super::RequestLimitInfo::default(),
                    bonus_grants: vec![],
                    workspace_allowances: allowances(),
                })
            });
        client
            .expect_get_request_limit_info()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Err(anyhow::anyhow!("Refresh failed")));
        client
            .expect_get_request_limit_info()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| {
                Ok(super::super::RequestUsageInfo {
                    request_limit_info: super::super::RequestLimitInfo::default(),
                    bonus_grants: vec![],
                    workspace_allowances: HashMap::new(),
                })
            });
        let model =
            app.add_singleton_model(|ctx| AIRequestUsageModel::new_for_test(Arc::new(client), ctx));
        assert!(
            model
                .update(&mut app, |model, ctx| model.refresh_request_usage(ctx))
                .await
                .unwrap()
                .is_some()
        );
        for (id, expected) in [(2, Some(2000.0)), (1, Some(1234.75)), (3, None)] {
            UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
                workspaces.set_current_workspace_uid(workspace_uid(id), ctx);
            });
            model.read(&app, |model, ctx| {
                match (model.workspace_included_usage(ctx), expected) {
                    (WorkspaceIncludedUsageState::Ready(info), Some(expected)) => {
                        assert_eq!(info.included_usage_credits, expected);
                    }
                    (WorkspaceIncludedUsageState::Unavailable, None) => {}
                    _ => panic!("Workspace selection should read the cached allowance"),
                }
            });
        }
        assert!(
            model
                .update(&mut app, |model, ctx| model.refresh_request_usage(ctx))
                .await
                .unwrap()
                .is_none()
        );
        model.read(&app, |model, _| {
            assert_eq!(model.workspace_allowances.len(), 2)
        });
        assert!(
            model
                .update(&mut app, |model, ctx| model.refresh_request_usage(ctx))
                .await
                .unwrap()
                .is_some()
        );
        model.read(&app, |model, _| {
            assert!(model.workspace_allowances.is_empty())
        });
    });
}
