use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_graphql::mutations::update_user_settings::UpdateUserSettingsResult;
use warp_server_auth::auth_state::AuthState;

use super::{AuthClient, AuthClientImpl};
use crate::base_client::{
    AuthenticatedGraphqlConfig, BaseClient, GraphqlRoutingConfig, TEAM_UID_HEADER,
};

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

/// Builds an `AuthClientImpl` whose requests land on the shared test mock server
/// (`ChannelState::mock_server()`), for asserting on the exact headers a real request sends.
fn auth_client_for_test() -> AuthClientImpl {
    let (event_sender, _event_receiver) = async_channel::unbounded();
    let base_client = BaseClient::new(
        std::sync::Arc::new(http_client::Client::new()),
        std::sync::Arc::new(AuthState::new_for_test()),
        event_sender,
        None,
        GraphqlRoutingConfig::default(),
        AuthenticatedGraphqlConfig::default(),
        None,
    );
    AuthClientImpl::new(std::sync::Arc::new(base_client))
}

/// A minimal GraphQL error response. Its shape doesn't matter to these tests: they only
/// assert on the request mockito received, not on how the client interprets the response.
const GRAPHQL_ERROR_BODY: &str = r#"{"errors":[{"message":"boom"}]}"#;

#[test]
fn list_api_keys_sends_no_team_header_when_personal() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, mockito::Matcher::Missing)
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let client = auth_client_for_test();
    let _ = block_on(client.list_api_keys(None));

    mock.assert();
}

#[test]
fn list_api_keys_sends_selected_team_header() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let client = auth_client_for_test();
    let _ = block_on(client.list_api_keys(Some("team-a-uid")));

    mock.assert();
}

#[test]
fn create_api_key_sends_same_team_uid_in_header_and_body() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-b-uid")
        .match_body(mockito::Matcher::Regex("team-b-uid".to_string()))
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let client = auth_client_for_test();
    let _ = block_on(client.create_api_key(
        "ci-key".to_string(),
        Some(cynic::Id::new("team-b-uid")),
        None,
        None,
    ));

    mock.assert();
}

#[test]
fn create_api_key_sends_no_team_header_when_personal() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, mockito::Matcher::Missing)
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let client = auth_client_for_test();
    let _ = block_on(client.create_api_key("ci-key".to_string(), None, None, None));

    mock.assert();
}
