use std::future::Future;
use std::time::Duration;

use futures::executor::block_on;
use mockito::{Matcher, Mock};
use vec1::vec1;
use warp_graphql::ai::AgentHarness;
use warp_graphql::managed_secrets::ManagedSecretType;
use warp_managed_secrets::client::{IdentityTokenOptions, ManagedSecretsClient as _, SecretOwner};
use warp_server_client::base_client::TEAM_UID_HEADER;

use super::super::ServerApi;
use crate::server::ids::ServerId;
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::user_workspaces::TeamContextForOperation;

fn request_scope_for_team(team_uid: ServerId) -> RequestTeamScope {
    RequestTeamScope::from_scope(&TeamContextForOperation::new_for_test(team_uid))
}

fn server_api() -> ServerApi {
    let server_api = ServerApi::new_for_test();
    server_api
        .base_client
        .set_ambient_workload_token_for_test("test-workload-token".to_string());
    server_api
}

fn mock_graphql_request(operation: &str, team_uid: Option<&str>) -> Mock {
    let mut server = warp_core::channel::ChannelState::mock_server();
    let request = server
        .mock("POST", "/graphql/v2")
        .match_query(Matcher::UrlEncoded("op".to_string(), operation.to_string()))
        .with_status(200)
        .with_body(r#"{"data":null}"#);
    match team_uid {
        Some(team_uid) => request.match_header(TEAM_UID_HEADER, team_uid).create(),
        None => request
            .match_header(TEAM_UID_HEADER, Matcher::Missing)
            .create(),
    }
}

fn assert_request_sent<T>(request: Mock, future: impl Future<Output = anyhow::Result<T>>) {
    assert!(block_on(future).is_err());
    request.assert();
}

#[test]
fn scoped_managed_secret_requests_send_selected_team_header() {
    let server_api = server_api();
    let team_uid = ServerId::from(17);
    let team_uid_header = team_uid.to_string();
    let request_scope = request_scope_for_team(team_uid);

    assert_request_sent(
        mock_graphql_request("GetManagedSecretConfig", Some(&team_uid_header)),
        server_api.get_managed_secret_configs(&request_scope),
    );
    assert_request_sent(
        mock_graphql_request("CreateManagedSecret", Some(&team_uid_header)),
        server_api.create_managed_secret(
            &request_scope,
            SecretOwner::CurrentUser,
            "API_KEY".to_string(),
            ManagedSecretType::RawValue,
            "encrypted".to_string(),
            None,
        ),
    );
    assert_request_sent(
        mock_graphql_request("DeleteManagedSecret", Some(&team_uid_header)),
        server_api.delete_managed_secret(
            &request_scope,
            SecretOwner::CurrentUser,
            "API_KEY".to_string(),
        ),
    );
    assert_request_sent(
        mock_graphql_request("UpdateManagedSecret", Some(&team_uid_header)),
        server_api.update_managed_secret(
            &request_scope,
            SecretOwner::CurrentUser,
            "API_KEY".to_string(),
            Some("updated".to_string()),
            None,
        ),
    );
    assert_request_sent(
        mock_graphql_request("ListManagedSecrets", Some(&team_uid_header)),
        server_api.list_secrets(&request_scope),
    );
    assert_request_sent(
        mock_graphql_request("ListHarnessAuthSecrets", Some(&team_uid_header)),
        server_api.list_harness_auth_secrets(&request_scope, AgentHarness::ClaudeCode),
    );
}

#[test]
fn resource_authoritative_managed_secret_requests_omit_team_header() {
    let server_api = server_api();

    assert_request_sent(
        mock_graphql_request("TaskSecrets", None),
        server_api.get_task_secrets("task-id".to_string(), "workload-token".to_string()),
    );
    assert_request_sent(
        mock_graphql_request("IssueTaskIdentityToken", None),
        server_api.issue_task_identity_token(IdentityTokenOptions {
            audience: "https://example.com".to_string(),
            requested_duration: Duration::from_secs(300),
            subject_template: vec1!["scoped_principal".to_string()],
        }),
    );
}
