use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cloud_objects::ids::ServerId;
use futures::executor::block_on;
use warp_core::channel::ChannelState;
use warp_server_auth::auth_state::AuthState;

use super::{
    AGENT_SOURCE_HEADER, AMBIENT_WORKLOAD_TOKEN_HEADER, AmbientHeaderPolicy,
    AuthenticatedGraphqlConfig, BaseClient, CLOUD_AGENT_ID_HEADER, GraphqlRoutingConfig,
    HeaderOverride, TEAM_UID_HEADER,
};
use crate::auth::{AuthClient, AuthClientImpl, RequestTeamScope};

struct StaticIapTokenProvider;

impl http_client::iap::IapTokenProvider for StaticIapTokenProvider {
    fn cached_token(&self) -> Option<String> {
        Some("iap-token".to_string())
    }
}

fn client() -> BaseClient {
    let (event_sender, _) = async_channel::unbounded();
    let mut authenticated_headers = HashMap::new();
    authenticated_headers.insert("X-Test-Authenticated".to_string(), "true".to_string());
    BaseClient::new(
        Arc::new(http_client::Client::new()),
        Arc::new(AuthState::new_for_test()),
        event_sender,
        Some("cloud_mode".to_string()),
        GraphqlRoutingConfig {
            path_prefix: Some("/routing-only".to_string()),
        },
        AuthenticatedGraphqlConfig {
            headers: authenticated_headers,
        },
        None,
    )
}

#[test]
fn iap_proxy_auth_header_uses_configured_provider() {
    let (event_sender, _) = async_channel::unbounded();
    let client = BaseClient::new(
        Arc::new(http_client::Client::new()),
        Arc::new(AuthState::new_for_test()),
        event_sender,
        None,
        GraphqlRoutingConfig::default(),
        AuthenticatedGraphqlConfig::default(),
        Some(Arc::new(StaticIapTokenProvider)),
    );

    assert_eq!(
        client.iap_proxy_auth_header(),
        Some((
            http_client::iap::IAP_PROXY_AUTH_HEADER,
            "Bearer iap-token".to_string()
        ))
    );
}

#[test]
fn explicit_token_graphql_options_route_without_authenticated_headers() {
    let client = client();
    client.set_ambient_agent_task_id(Some("ambient-task".to_string()));

    let options = client.graphql_request_options_with_token(Some("token".to_string()));

    assert_eq!(options.path_prefix.as_deref(), Some("/routing-only"));
    assert_eq!(options.auth_token.as_deref(), Some("token"));
    assert!(options.headers.is_empty());
}

#[test]
fn ambient_policy_supports_inherit_override_and_omit() {
    let client = client();
    client.set_ambient_agent_task_id(Some("ambient-task".to_string()));

    let inherited = block_on(client.ambient_headers(AmbientHeaderPolicy {
        workload_token: HeaderOverride::Set("workload".to_string()),
        cloud_agent_id: HeaderOverride::Inherit,
        agent_source: HeaderOverride::Inherit,
    }))
    .unwrap();
    assert!(inherited.contains(&(
        AMBIENT_WORKLOAD_TOKEN_HEADER.to_string(),
        "workload".to_string(),
    )));
    assert!(inherited.contains(&(
        CLOUD_AGENT_ID_HEADER.to_string(),
        "ambient-task".to_string()
    )));
    assert!(inherited.contains(&(AGENT_SOURCE_HEADER.to_string(), "cloud_mode".to_string())));

    let task_scoped = block_on(client.ambient_headers(AmbientHeaderPolicy {
        workload_token: HeaderOverride::Set("workload".to_string()),
        ..AmbientHeaderPolicy::for_task("specific-task")
    }))
    .unwrap();
    assert!(task_scoped.contains(&(
        CLOUD_AGENT_ID_HEADER.to_string(),
        "specific-task".to_string(),
    )));
    assert!(!task_scoped.contains(&(
        CLOUD_AGENT_ID_HEADER.to_string(),
        "ambient-task".to_string()
    )));

    let omitted = block_on(client.ambient_headers(AmbientHeaderPolicy::omit_all())).unwrap();
    assert!(omitted.is_empty());
}

#[test]
fn authenticated_graphql_options_include_configured_and_ambient_headers() {
    let client = client();
    client.set_ambient_agent_task_id(Some("ambient-task".to_string()));

    let options = block_on(client.graphql_request_options(None)).unwrap();

    assert_eq!(options.path_prefix.as_deref(), Some("/routing-only"));
    assert_eq!(
        options
            .headers
            .get("X-Test-Authenticated")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        options
            .headers
            .get(CLOUD_AGENT_ID_HEADER)
            .map(String::as_str),
        Some("ambient-task")
    );
    assert_eq!(
        options.headers.get(AGENT_SOURCE_HEADER).map(String::as_str),
        Some("cloud_mode")
    );
}

#[test]
fn authenticated_graphql_configuration_cannot_override_base_client_owned_headers() {
    let (event_sender, _) = async_channel::unbounded();
    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), "malicious".to_string());
    headers.insert("content-type".to_string(), "text/plain".to_string());
    headers.insert("CONTENT-LENGTH".to_string(), "9999".to_string());
    headers.insert(
        http_client::iap::IAP_PROXY_AUTH_HEADER.to_string(),
        "malicious".to_string(),
    );
    headers.insert(
        CLOUD_AGENT_ID_HEADER.to_ascii_lowercase(),
        "malicious".to_string(),
    );
    headers.insert("x-eval-user-id".to_string(), "1234".to_string());
    let client = BaseClient::new(
        Arc::new(http_client::Client::new()),
        Arc::new(AuthState::new_for_test()),
        event_sender,
        None,
        GraphqlRoutingConfig::default(),
        AuthenticatedGraphqlConfig { headers },
        None,
    );

    let options = block_on(client.graphql_request_options(None)).unwrap();

    assert!(!options.headers.contains_key("authorization"));
    assert!(!options.headers.contains_key("content-type"));
    assert!(!options.headers.contains_key("CONTENT-LENGTH"));
    assert!(
        !options
            .headers
            .contains_key(http_client::iap::IAP_PROXY_AUTH_HEADER)
    );
    assert!(
        !options
            .headers
            .contains_key(&CLOUD_AGENT_ID_HEADER.to_ascii_lowercase())
    );
    assert_eq!(
        options.headers.get("x-eval-user-id").map(String::as_str),
        Some("1234")
    );
}

fn api_key_client(path_prefix: &str) -> (AuthClientImpl, Arc<Mutex<Option<String>>>) {
    let observed_team_uid = Arc::new(Mutex::new(None));
    let observed_team_uid_for_request = observed_team_uid.clone();
    let mut http_client = http_client::Client::new();
    http_client.set_before_request_fn(Box::new(move |request, _| {
        *observed_team_uid_for_request.lock().unwrap() = request
            .headers()
            .get(TEAM_UID_HEADER)
            .map(|value| value.to_str().unwrap().to_string());
    }));
    let auth_state = AuthState::new_logged_out_for_test();
    auth_state.set_remote_server_bearer_token("test-token".to_string());
    let (event_sender, _) = async_channel::unbounded();
    let base_client = BaseClient::new(
        Arc::new(http_client),
        Arc::new(auth_state),
        event_sender,
        None,
        GraphqlRoutingConfig {
            path_prefix: Some(path_prefix.to_string()),
        },
        AuthenticatedGraphqlConfig::default(),
        None,
    );
    *base_client.ambient_workload_token.lock() = Some(warp_isolation_platform::WorkloadToken {
        token: "test-workload-token".to_string(),
        expires_at: None,
    });

    (
        AuthClientImpl::new(Arc::new(base_client)),
        observed_team_uid,
    )
}

fn mock_api_key_list(path_prefix: &str) -> mockito::Mock {
    let mut server = ChannelState::mock_server();
    server
        .mock(
            "POST",
            mockito::Matcher::Regex(format!("^{path_prefix}/graphql/v2")),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"data":{"apiKeys":{"__typename":"APIKeyPropertiesOutput","apiKeys":[],"responseContext":{"serverVersion":null}}}}"#,
        )
        .create()
}

#[test]
fn list_api_keys_sends_selected_team_header() {
    let team_uid = "abcdefghijklmnopqrstuv";
    let request = mock_api_key_list("/api-key-selected");
    let (auth_client, observed_team_uid) = api_key_client("/api-key-selected");
    let scope = RequestTeamScope::from_resolved_team(Some(ServerId::try_from(team_uid).unwrap()));

    let keys = block_on(auth_client.list_api_keys(scope)).unwrap();

    assert!(keys.is_empty());
    assert_eq!(observed_team_uid.lock().unwrap().as_deref(), Some(team_uid));
    request.assert();
}

#[test]
fn list_api_keys_omits_team_header_when_unscoped() {
    let request = mock_api_key_list("/api-key-unscoped");
    let (auth_client, observed_team_uid) = api_key_client("/api-key-unscoped");

    let keys = block_on(auth_client.list_api_keys(RequestTeamScope::unscoped())).unwrap();

    assert!(keys.is_empty());
    assert_eq!(observed_team_uid.lock().unwrap().as_deref(), None);
    request.assert();
}
