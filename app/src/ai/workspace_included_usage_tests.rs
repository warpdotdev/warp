use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use warpui::{App, ModelHandle};

use super::*;
use crate::server::ids::ServerId;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::MockAIClient;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::workspace::Workspace;

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
fn included_usage_distinguishes_loading_from_a_workspace_without_an_allowance() {
    let mut request = WorkspaceIncludedUsageRequest::default();
    let (handle, _) = request.begin(Some(UserUid::new("user"))).unwrap();
    assert!(matches!(
        request.for_workspace(workspace_uid(1)),
        WorkspaceIncludedUsageState::Loading
    ));
    request.complete(&handle, Ok(allowances()));
    assert!(matches!(
        request.for_workspace(workspace_uid(3)),
        WorkspaceIncludedUsageState::Unavailable
    ));
    let WorkspaceIncludedUsageState::Ready(info) = request.for_workspace(workspace_uid(2)) else {
        panic!("Expected the second workspace's allowance");
    };
    assert_eq!(info.included_usage_credits, 2000.0);
}

#[test]
fn included_usage_rejects_queued_results_after_account_switches_back_and_logout() {
    let mut request = WorkspaceIncludedUsageRequest::default();
    let (old, _) = request.begin(Some(UserUid::new("user"))).unwrap();
    request.begin(Some(UserUid::new("another-user")));
    let (current, _) = request.begin(Some(UserUid::new("user"))).unwrap();
    request.complete(&old, Ok(allowances()));
    assert!(matches!(
        request.for_workspace(workspace_uid(1)),
        WorkspaceIncludedUsageState::Loading
    ));
    request.complete(&current, Ok(allowances()));
    assert!(matches!(
        request.for_workspace(workspace_uid(1)),
        WorkspaceIncludedUsageState::Ready(_)
    ));

    let (pending, _) = request.begin(Some(UserUid::new("another-user"))).unwrap();
    request.begin(None);
    request.complete(&pending, Ok(allowances()));
    assert_eq!(request.user_uid, None);
    assert!(request.allowances.is_none());
    assert!(request.abort_handle.is_none());
}

#[test]
fn included_usage_model_caches_all_workspaces_and_retries_independently() {
    App::test((), |mut app| async move {
        app.update(crate::settings::init_and_register_user_preferences);
        app.add_singleton_model(crate::settings::PrivacySettings::mock);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
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
        let mut client = MockWorkspaceClient::new();
        let mut sequence = mockall::Sequence::new();
        client
            .expect_included_usage_by_workspace()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Err(anyhow::anyhow!("Cannot query field includedUsageInfo")));
        client
            .expect_included_usage_by_workspace()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|| Ok(allowances()));
        let model = app.add_singleton_model(|ctx| {
            let mut model = AIRequestUsageModel::new_for_test(Arc::new(MockAIClient::new()), ctx);
            model.workspace_client = Arc::new(client);
            model
        });
        let (sender, events) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &model,
                move |model: ModelHandle<AIRequestUsageModel>, event, ctx| {
                    if matches!(
                        event,
                        AIRequestUsageModelEvent::WorkspaceIncludedUsageUpdated
                    ) && !matches!(
                        model.as_ref(ctx).workspace_included_usage(ctx),
                        WorkspaceIncludedUsageState::Loading,
                    ) {
                        sender.try_send(()).unwrap();
                    }
                },
            );
        });

        model.update(&mut app, |model, ctx| {
            model.refresh_workspace_included_usage(ctx);
            model.refresh_workspace_included_usage(ctx);
            assert!(matches!(
                model.workspace_included_usage(ctx),
                WorkspaceIncludedUsageState::Loading
            ));
        });
        events.recv().await.unwrap();
        model.read(&app, |model, ctx| {
            assert!(matches!(
                model.workspace_included_usage(ctx),
                WorkspaceIncludedUsageState::Unavailable
            ));
            assert_eq!(model.request_limit(), 150);
            assert!(model.bonus_grants().is_empty());
        });

        model.update(&mut app, |model, ctx| {
            model.refresh_workspace_included_usage(ctx)
        });
        UserWorkspaces::handle(&app).update(&mut app, |workspaces, ctx| {
            workspaces.set_current_workspace_uid(workspace_uid(2), ctx);
        });
        events.recv().await.unwrap();
        model.read(&app, |model, ctx| {
            let WorkspaceIncludedUsageState::Ready(info) = model.workspace_included_usage(ctx)
            else {
                panic!("Expected the shared workspace snapshot");
            };
            assert_eq!(info.included_usage_credits, 2000.0);
            assert_eq!(info.usage_credits_used_since_last_refresh, 250.125);
            assert_eq!(info.usage_cents_used_since_last_refresh, 1250.25);
            assert_eq!(model.request_limit(), 150);
            assert!(model.bonus_grants().is_empty());
        });

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
    });
}
