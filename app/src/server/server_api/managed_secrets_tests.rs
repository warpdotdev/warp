use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_graphql::managed_secrets::{ManagedSecret, ManagedSecretType};
use warp_graphql::object::{Space, SpaceType};
use warp_managed_secrets::client::{ManagedSecretsClient, SecretOwner};
use warp_server_client::base_client::TEAM_UID_HEADER;

use super::super::ServerApi;
use super::retain_personal_and_team_secrets;

fn secret(name: &str, owner_type: SpaceType, owner_uid: &str) -> ManagedSecret {
    ManagedSecret {
        name: name.to_string(),
        description: None,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
        owner: Space {
            uid: cynic::Id::new(owner_uid),
            type_: owner_type,
        },
        type_: ManagedSecretType::RawValue,
    }
}

#[test]
fn retains_only_personal_secrets_when_no_team_selected() {
    let secrets = vec![
        secret("personal", SpaceType::User, "user-uid"),
        secret("team-a", SpaceType::Team, "team-a-uid"),
    ];

    let retained = retain_personal_and_team_secrets(secrets, None);

    assert_eq!(
        retained.into_iter().map(|s| s.name).collect::<Vec<_>>(),
        vec!["personal".to_string()]
    );
}

#[test]
fn retains_personal_and_selected_team_secrets_but_not_other_teams() {
    let secrets = vec![
        secret("personal", SpaceType::User, "user-uid"),
        secret("team-a", SpaceType::Team, "team-a-uid"),
        secret("team-b", SpaceType::Team, "team-b-uid"),
    ];

    let retained = retain_personal_and_team_secrets(secrets, Some("team-a-uid"));

    assert_eq!(
        retained.into_iter().map(|s| s.name).collect::<Vec<_>>(),
        vec!["personal".to_string(), "team-a".to_string()]
    );
}

/// A minimal GraphQL error response. Its shape doesn't matter to these tests: they only
/// assert on the request mockito received, not on how the client interprets the response.
const GRAPHQL_ERROR_BODY: &str = r#"{"errors":[{"message":"boom"}]}"#;

#[test]
fn list_secrets_sends_no_team_header_when_personal() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, mockito::Matcher::Missing)
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.list_secrets(None));

    mock.assert();
}

#[test]
fn list_secrets_sends_selected_team_header() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.list_secrets(Some("team-a-uid")));

    mock.assert();
}

#[test]
fn list_harness_auth_secrets_sends_selected_team_header() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.list_harness_auth_secrets(
        warp_graphql::ai::AgentHarness::ClaudeCode,
        Some("team-a-uid"),
    ));

    mock.assert();
}

#[test]
fn get_personal_managed_secret_config_sends_no_team_header() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, mockito::Matcher::Missing)
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.get_personal_managed_secret_config());

    mock.assert();
}

#[test]
fn get_team_managed_secret_config_sends_selected_team_header() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.get_team_managed_secret_config("team-a-uid"));

    mock.assert();
}

#[test]
fn create_managed_secret_sends_same_team_uid_in_header_and_body() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .match_body(mockito::Matcher::Regex("team-a-uid".to_string()))
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.create_managed_secret(
        SecretOwner::Team {
            team_uid: "team-a-uid".to_string(),
        },
        "ci-secret".to_string(),
        ManagedSecretType::RawValue,
        "encrypted".to_string(),
        None,
    ));

    mock.assert();
}

#[test]
fn create_managed_secret_sends_no_team_header_for_personal_owner() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, mockito::Matcher::Missing)
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.create_managed_secret(
        SecretOwner::CurrentUser,
        "ci-secret".to_string(),
        ManagedSecretType::RawValue,
        "encrypted".to_string(),
        None,
    ));

    mock.assert();
}

#[test]
fn update_managed_secret_sends_same_team_uid_in_header_and_body() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .match_body(mockito::Matcher::Regex("team-a-uid".to_string()))
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.update_managed_secret(
        SecretOwner::Team {
            team_uid: "team-a-uid".to_string(),
        },
        "ci-secret".to_string(),
        Some("encrypted".to_string()),
        None,
    ));

    mock.assert();
}

#[test]
fn delete_managed_secret_sends_same_team_uid_in_header_and_body() {
    let mut server = ChannelState::mock_server();
    let mock = server
        .mock("POST", mockito::Matcher::Regex(r"^/graphql/v2".to_string()))
        .match_header(TEAM_UID_HEADER, "team-a-uid")
        .match_body(mockito::Matcher::Regex("team-a-uid".to_string()))
        .with_status(200)
        .with_body(GRAPHQL_ERROR_BODY)
        .create();

    let server_api = ServerApi::new_for_test();
    let _ = block_on(server_api.delete_managed_secret(
        SecretOwner::Team {
            team_uid: "team-a-uid".to_string(),
        },
        "ci-secret".to_string(),
    ));

    mock.assert();
}
