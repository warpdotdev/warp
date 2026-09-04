use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use warp_graphql::mutations::create_simple_integration::CreateSimpleIntegrationOutput;
use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;
use warpui::App;
use warpui::r#async::Timer;

use super::{IntegrationCommandRunner, IntegrationRetryState};
use crate::server::ids::ServerId;
use crate::server::server_api::integrations::MockIntegrationsClient;
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::user_workspaces::{TeamContextForOperation, TeamlessScopeForTest};

#[test]
fn oauth_retry_uses_the_initiating_team_scope() {
    let scope = TeamContextForOperation::new_for_test(ServerId::from(7));
    run_oauth_retry(RequestTeamScope::from_scope(&scope), false);
}

#[test]
fn oauth_retry_uses_the_initiating_teamless_scope() {
    run_oauth_retry(RequestTeamScope::from_scope(&TeamlessScopeForTest), true);
}

fn run_oauth_retry(expected_scope: RequestTeamScope, is_update: bool) {
    App::test((), move |mut app| async move {
        let request_count = Arc::new(AtomicUsize::new(0));
        let observed_request_count = request_count.clone();
        let mut integrations_client = MockIntegrationsClient::new();
        integrations_client
            .expect_create_or_update_simple_integration()
            .times(2)
            .returning(
                move |request_scope,
                      integration_type,
                      request_is_update,
                      _environment_uid,
                      _base_prompt,
                      _model_id,
                      _mcp_servers_json,
                      _remove_mcp_server_names,
                      _worker_host,
                      enabled| {
                    assert_eq!(request_scope, expected_scope);
                    assert_eq!(integration_type, "slack");
                    assert_eq!(request_is_update, is_update);
                    assert!(enabled);
                    let request_index = observed_request_count.fetch_add(1, Ordering::SeqCst);
                    Ok(if request_index == 0 {
                        CreateSimpleIntegrationOutput {
                            auth_url: Some("https://example.com/oauth".to_string()),
                            success: false,
                            message: "Authorization required".to_string(),
                            tx_id: Some(cynic::Id::new("oauth-tx")),
                        }
                    } else {
                        CreateSimpleIntegrationOutput {
                            auth_url: None,
                            success: true,
                            message: "Integration saved".to_string(),
                            tx_id: None,
                        }
                    })
                },
            );
        integrations_client
            .expect_poll_oauth_connect_status()
            .times(1)
            .returning(|tx_id| {
                assert_eq!(tx_id, "oauth-tx");
                Ok(OauthConnectTxStatus::Completed)
            });

        let runner =
            app.add_model(move |_| IntegrationCommandRunner::new(Arc::new(integrations_client)));
        runner.update(&mut app, |runner, ctx| {
            runner.start_create_or_update_flow(
                ctx,
                IntegrationRetryState::new(expected_scope),
                "slack".to_string(),
                None,
                None,
                None,
                None,
                None,
                None,
                true,
                is_update,
            );
        });

        for _ in 0..120 {
            if request_count.load(Ordering::SeqCst) == 2 {
                break;
            }
            Timer::after(Duration::from_millis(50)).await;
        }

        assert_eq!(request_count.load(Ordering::SeqCst), 2);
        futures_lite::future::yield_now().await;
        assert!(app.termination_result().is_none());
    });
}
