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

fn context(workspace: i64) -> IncludedUsageContext {
    (
        UserUid::new("user"),
        WorkspaceUid::from(ServerId::from(workspace)),
    )
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
fn included_usage_distinguishes_loading_from_missing_allowance() {
    let mut request = WorkspaceIncludedUsageRequest::default();
    let (handle, _) = request.begin(Some(context(1))).unwrap();
    assert!(matches!(
        request.state,
        WorkspaceIncludedUsageState::Loading
    ));
    request.complete(&handle, Ok(None));
    assert!(matches!(
        request.state,
        WorkspaceIncludedUsageState::Unavailable
    ));
}

#[test]
fn included_usage_rejects_queued_results_when_workspace_selection_returns_to_original() {
    let mut request = WorkspaceIncludedUsageRequest::default();
    let (old, _) = request.begin(Some(context(1))).unwrap();
    request.begin(Some(context(2)));
    let (current, _) = request.begin(Some(context(1))).unwrap();

    request.complete(&old, Ok(Some(pool())));
    assert!(matches!(
        request.state,
        WorkspaceIncludedUsageState::Loading
    ));
    request.complete(&current, Ok(Some(pool())));
    assert!(matches!(
        request.state,
        WorkspaceIncludedUsageState::Ready(_)
    ));
}

#[test]
fn included_usage_rejects_queued_results_after_account_change_and_logout() {
    let mut request = WorkspaceIncludedUsageRequest::default();
    let (old, _) = request.begin(Some(context(1))).unwrap();
    let (current, _) = request
        .begin(Some((UserUid::new("another-user"), context(1).1)))
        .unwrap();
    request.complete(&old, Ok(Some(pool())));
    assert!(matches!(
        request.state,
        WorkspaceIncludedUsageState::Loading
    ));

    request.begin(None);
    request.complete(&current, Ok(Some(pool())));
    assert!(matches!(request.state, WorkspaceIncludedUsageState::Idle));
    assert_eq!(request.context, None);
}

#[test]
fn included_usage_model_retries_independently_of_personal_allowance_and_grants() {
    App::test((), |mut app| async move {
        app.update(crate::settings::init_and_register_user_preferences);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|ctx| {
            let mut workspace =
                Workspace::from_local_cache(context(1).1, "Scale".to_string(), None, None);
            workspace.billing_metadata.customer_type = CustomerType::Scale;
            UserWorkspaces::mock(
                Arc::new(MockTeamClient::new()),
                Arc::new(MockWorkspaceClient::new()),
                vec![workspace],
                ctx,
            )
        });
        let mut client = MockWorkspaceClient::new();
        let mut sequence = mockall::Sequence::new();
        client
            .expect_included_usage_info()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Err(anyhow::anyhow!("Cannot query field includedUsageInfo")));
        client
            .expect_included_usage_info()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(Some(pool())));
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
        events.recv().await.unwrap();
        model.read(&app, |model, ctx| {
            let WorkspaceIncludedUsageState::Ready(info) = model.workspace_included_usage(ctx)
            else {
                panic!("Expected the shared workspace snapshot");
            };
            assert_eq!(info.included_usage_credits, 1234.75);
            assert_eq!(info.usage_credits_used_since_last_refresh, 250.125);
            assert_eq!(info.usage_cents_used_since_last_refresh, 1250.25);
            assert_eq!(model.request_limit(), 150);
            assert!(model.bonus_grants().is_empty());
        });
    });
}
