use std::sync::Arc;

use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_graphql::mutations::update_user_settings::UpdateUserSettingsResult;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::LoginToken;

use super::{AuthClient, AuthClientImpl};
use crate::auth::AuthEvent;
use crate::base_client::{AuthenticatedGraphqlConfig, BaseClient, GraphqlRoutingConfig};

#[test]
fn unknown_settings_results_preserve_operation_context() {
    for expected_message in [
        "failed to set telemetry enabled",
        "failed to set crash reporting enabled",
        "failed to set cloud conversation storage enabled",
        "failed to update user settings",
    ] {
        let error = AuthClientImpl::on_settings_updated(
            UpdateUserSettingsResult::Unknown,
            expected_message,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), expected_message);
    }
}

/// Regression test for the review finding that `AuthClientImpl::fetch_user` -
/// the exact call the optimistic startup auth attempt makes - never told
/// `IapManager` about an IAP challenge on its underlying `GetUser` request,
/// because `fetch_user_properties` called `operation.send_request` directly
/// instead of going through `graphql_helpers::send_graphql_request`. Exercises
/// `fetch_user` itself (not `send_graphql_request_with_options` directly) so a
/// regression at either layer would be caught.
#[test]
fn fetch_user_iap_challenge_notifies_iap_manager() {
    let mut server = ChannelState::mock_server();
    let _mock = server
        .mock("POST", "/graphql/v2")
        .match_query(mockito::Matcher::Any)
        .with_status(401)
        .with_header("x-goog-iap-generated-response", "true")
        .with_body("{}")
        .create();

    let (event_sender, event_receiver) = async_channel::unbounded();
    let base_client = Arc::new(BaseClient::new(
        Arc::new(http_client::Client::new()),
        Arc::new(AuthState::new_for_test()),
        event_sender,
        None,
        GraphqlRoutingConfig::default(),
        AuthenticatedGraphqlConfig::default(),
        None,
    ));
    let auth_client = AuthClientImpl::new(base_client);

    let result =
        block_on(auth_client.fetch_user(LoginToken::ApiKey("wk-test-key".to_string()), false));

    assert!(
        result.is_err(),
        "expected the IAP challenge to fail fetch_user"
    );
    match event_receiver.try_recv() {
        Ok(AuthEvent::IapChallengeReceived) => {}
        other => panic!("expected IapChallengeReceived event, got {other:?}"),
    }
}
