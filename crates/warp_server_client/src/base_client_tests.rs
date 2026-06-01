use std::collections::HashMap;
use std::sync::Arc;

use futures::executor::block_on;
use warp_server_auth::auth_state::AuthState;

use super::{
    AGENT_SOURCE_HEADER, AMBIENT_WORKLOAD_TOKEN_HEADER, AmbientHeaderPolicy,
    AuthenticatedGraphqlConfig, BaseClient, CLOUD_AGENT_ID_HEADER, GraphqlRoutingConfig,
    HeaderOverride,
};

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
        false,
    )
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
    headers.insert("X-Eval-User-ID".to_string(), "1234".to_string());
    let client = BaseClient::new(
        Arc::new(http_client::Client::new()),
        Arc::new(AuthState::new_for_test()),
        event_sender,
        None,
        GraphqlRoutingConfig::default(),
        AuthenticatedGraphqlConfig { headers },
        false,
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
        options.headers.get("X-Eval-User-ID").map(String::as_str),
        Some("1234")
    );
}
