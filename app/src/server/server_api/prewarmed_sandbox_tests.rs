#[cfg(not(feature = "skip_login"))]
use std::sync::Arc;

#[cfg(not(feature = "skip_login"))]
use futures::executor::block_on;
#[cfg(not(feature = "skip_login"))]
use mockito::Matcher;
use warp_graphql::queries::execution_config::{CodeForge, ExecutionRepositoryRefType};
#[cfg(not(feature = "skip_login"))]
use warp_server_client::base_client::AMBIENT_WORKLOAD_TOKEN_HEADER;

use super::*;

const CONFIGURATION_JSON: &str = include_str!(
    "../../../../crates/graphql/src/api/mutations/fixtures/prewarmed_sandbox_config.json"
);

#[test]
#[cfg(not(target_family = "wasm"))]
fn control_unavailable_recognizes_native_handshake_410() {
    for status in [410, 403, 503] {
        let response = websocket::tungstenite::http::Response::builder()
            .status(status)
            .body(None)
            .unwrap();
        let error = anyhow::Error::new(websocket::tungstenite::Error::Http(response))
            .context("control handshake failed");
        assert_eq!(is_prewarmed_sandbox_unavailable(&error), status == 410);
    }
}

#[test]
fn control_unavailable_recognizes_an_http_410() {
    for status in [410, 403, 503] {
        let error = anyhow::Error::new(HttpStatusError::new(status, String::new()))
            .context("workload token request failed");
        assert_eq!(is_prewarmed_sandbox_unavailable(&error), status == 410);
    }
}

#[test]
fn control_socket_url_uses_websocket_schemes_and_binds_the_generation() {
    for (http_url, expected) in [
        (
            "https://app.warp.dev/internal/prewarmed-sandboxes/sb/ws",
            "wss://app.warp.dev/internal/prewarmed-sandboxes/sb/ws?generation=abc123",
        ),
        (
            "http://localhost:8080/internal/prewarmed-sandboxes/sb/ws",
            "ws://localhost:8080/internal/prewarmed-sandboxes/sb/ws?generation=abc123",
        ),
    ] {
        assert_eq!(control_socket_url(http_url, "abc123").unwrap(), expected);
    }
    assert!(control_socket_url("ftp://app.warp.dev/ws", "abc123").is_err());
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn native_control_headers_are_sensitive() {
    let request =
        control_request("ws://localhost/control", "access-marker", "workload-marker").unwrap();
    for name in ["authorization", PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER] {
        assert!(request.headers()[name].is_sensitive());
    }
    for debug in [format!("{request:?}"), format!("{request:#?}")] {
        assert!(!debug.contains("access-marker"));
        assert!(!debug.contains("workload-marker"));
    }
}

fn wire_configuration() -> GraphqlConfiguration {
    let response: cynic::GraphQlResponse<PrewarmedSandboxConfig> =
        serde_json::from_str(CONFIGURATION_JSON).unwrap();
    let PrewarmedSandboxConfigResult::PrewarmedSandboxConfiguration(config) =
        response.data.unwrap().prewarmed_sandbox_config
    else {
        panic!("expected a configuration");
    };
    *config
}

pub(crate) fn parse_configuration() -> PrewarmedSandboxConfiguration {
    wire_configuration().try_into().unwrap()
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn configuration_errors_return_after_one_request() {
    let api = authenticated_client();
    for status in [403, 503] {
        let mut server = ChannelState::mock_server();
        let request = server
            .mock("POST", "/graphql/v2?op=PrewarmedSandboxConfig")
            .with_status(status)
            .with_header("Retry-After", "1")
            .with_body(r#"{"error":"configuration unavailable"}"#)
            .expect(1)
            .create();
        drop(server);

        let error = block_on(api.fetch_configuration("sandbox")).unwrap_err();
        assert_eq!(
            match error
                .downcast_ref::<warp_graphql::client::GraphQLError>()
                .unwrap()
            {
                warp_graphql::client::GraphQLError::HttpError { status, .. } => status.as_u16(),
                error => panic!("unexpected GraphQL error: {error}"),
            },
            status as u16
        );
        request.assert();
    }
}

#[test]
fn deserializes_a_server_configuration() {
    let config = parse_configuration();

    assert_eq!(config.generation, "abc123");
    assert_eq!(config.preparation.harness, "oz");

    let repository = &config.preparation.repositories[0];
    assert_eq!(repository.forge, CodeForge::GitHub);
    assert_eq!(repository.owner, "warpdotdev");
    assert_eq!(repository.name, "warp");
    assert_eq!(
        repository.ref_.as_ref().unwrap().type_,
        ExecutionRepositoryRefType::Branch
    );
    assert_eq!(repository.ref_.as_ref().unwrap().value, "main");
    assert_eq!(
        repository
            .clone_from
            .as_ref()
            .map(|repo| repo.repo.as_str()),
        Some("warp-fork")
    );
    assert!(repository.preserve_origin);

    let deferred = &config.preparation.deferred_repositories[0];
    assert_eq!(deferred.code_forge, CodeForge::GitHub);
    assert_eq!(deferred.owner, "warpdotdev");
    assert_eq!(deferred.repo, "warp-server");

    assert_eq!(config.preparation.setup_commands, ["./script/bootstrap"]);
    assert!(config.preparation.mcp_servers.contains_key("linear"));
    assert_eq!(config.preparation.skills, ["warpdotdev/warp:review"]);
    assert_eq!(config.preparation.factory_skill_dirs, [".agents/skills"]);
    assert!(config.preparation.computer_use_enabled);
    let providers = config.preparation.providers.as_ref().unwrap();
    assert_eq!(providers.gcp.as_ref().unwrap().project_number, "123456789");
    assert_eq!(
        providers.aws.as_ref().unwrap().role_arn,
        "arn:aws:iam::123456789012:role/OzAgent"
    );

    assert_eq!(
        config.environment_variables.get("WARP_FACTORY_REPO_DIR"),
        Some(&"factory".to_string())
    );
    assert_eq!(config.git_credentials[0].host, "github.com");
    assert_eq!(config.git_credentials[0].token, "ghs_secret");
    assert_eq!(
        config.git_credentials[0].username.as_deref(),
        Some("x-access-token")
    );
    assert!(config.git_credentials[0].email.is_none());
    assert!(matches!(
        config.managed_secrets.get("API_TOKEN"),
        Some(ManagedSecretValue::RawValue { value }) if value == "value"
    ));
    assert!(matches!(
        config.managed_secrets.get("CLAUDE_AUTH"),
        Some(ManagedSecretValue::AnthropicApiKey { api_key }) if api_key == "sk-ant-key"
    ));
}

#[test]
fn converts_empty_configuration_lists() {
    let mut config = wire_configuration();
    config.spec.repositories.clear();
    config.spec.deferred_repositories.clear();
    config.spec.setup_commands.clear();
    config.spec.mcp_servers_json = "{}".into();
    config.spec.skills.clear();
    config.spec.factory_skill_dirs.clear();
    config.environment_variables.clear();
    config.git_credentials.clear();
    config.managed_secrets.clear();

    let config = PrewarmedSandboxConfiguration::try_from(config).unwrap();
    assert!(config.preparation.repositories.is_empty());
    assert!(config.preparation.deferred_repositories.is_empty());
    assert!(config.preparation.mcp_servers.is_empty());
    assert!(config.environment_variables.is_empty());
    assert!(config.git_credentials.is_empty());
    assert!(config.managed_secrets.is_empty());
    assert_eq!(config.expires_at.to_rfc3339(), "2026-01-01T00:00:00+00:00");
}

#[test]
fn unknown_managed_secret_types_fail_conversion() {
    let mut config = wire_configuration();
    config.managed_secrets[0].value = serde_json::from_value(serde_json::json!({
        "__typename": "ManagedSecretFutureValue"
    }))
    .unwrap();
    let error = PrewarmedSandboxConfiguration::try_from(config).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid pre-warmed sandbox managed secrets"
    );
}

#[test]
fn invalid_mcp_json_errors_do_not_include_configuration_values() {
    for json in ["invalid-secret-json", "\"secret-value\"", "[]", "null"] {
        let mut config = wire_configuration();
        config.spec.mcp_servers_json = json.into();
        let error = PrewarmedSandboxConfiguration::try_from(config).unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "Invalid pre-warmed sandbox MCP server configuration"
        );
    }
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn configuration_uses_default_graphql_timeout_and_authentication() {
    let mut http_client = http_client::Client::new_for_test();
    http_client.set_before_request_fn(Box::new(|request, payload| {
        assert!(request.timeout().is_none());
        assert!(request.headers()["authorization"].is_sensitive());
        for debug in [format!("{request:?}"), format!("{request:#?}")] {
            assert!(!debug.contains("test-workload-token"));
            assert!(!debug.contains("test-access-token"));
        }
        assert!(request.headers()[AMBIENT_WORKLOAD_TOKEN_HEADER].is_sensitive());
        assert!(
            !request
                .headers()
                .contains_key(PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER)
        );
        let payload: serde_json::Value = serde_json::from_str(payload.as_deref().unwrap()).unwrap();
        assert!(payload["variables"]["requestContext"].is_object());
        assert_eq!(
            payload["variables"]["input"],
            serde_json::json!({"sandboxId": "sandbox"})
        );
    }));
    let api = authenticated_client_with_http(http_client);
    let mut server = ChannelState::mock_server();
    let request = server
        .mock("POST", "/graphql/v2?op=PrewarmedSandboxConfig")
        .match_header("authorization", "Bearer test-access-token")
        .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, "test-workload-token")
        .with_body(CONFIGURATION_JSON)
        .create();
    drop(server);
    assert_eq!(
        block_on(api.fetch_configuration("sandbox"))
            .unwrap()
            .generation,
        "abc123"
    );
    request.assert();
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn configuration_rejects_user_errors_unknown_results_and_missing_data() {
    let api = authenticated_client();
    for (body, expected) in [
        (
            serde_json::json!({"data": {"prewarmedSandboxConfig": {
                "__typename": "UserFacingError",
                "error": {"__typename": "UnknownError", "message": "private-server-detail"},
                "responseContext": {"serverVersion": null}
            }}}),
            "Pre-warmed sandbox configuration is unavailable",
        ),
        (
            serde_json::json!({"data": {"prewarmedSandboxConfig": {
                "__typename": "FutureConfig"
            }}}),
            "Unknown pre-warmed sandbox configuration response",
        ),
        (
            serde_json::json!({"data": null, "errors": [{"message": "configuration unavailable"}]}),
            "missing response data for PrewarmedSandboxConfig: configuration unavailable",
        ),
    ] {
        let mut server = ChannelState::mock_server();
        let request = server
            .mock("POST", "/graphql/v2?op=PrewarmedSandboxConfig")
            .with_body(body.to_string())
            .create();
        drop(server);
        let error = block_on(api.fetch_configuration("sandbox")).unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert!(!format!("{error:#}").contains("private-server-detail"));
        request.assert();
    }
}

#[test]
fn configuration_debug_redacts_credentials_and_raw_preparation_values() {
    let mut config = parse_configuration();
    config.managed_secrets.insert(
        "TOKEN".into(),
        ManagedSecretValue::raw_value("managed-secret-value"),
    );
    config.environment_variables.insert(
        "WARP_FACTORY_REPO_CLONE_URL".into(),
        "https://environment-secret-value@example.com/repo.git".into(),
    );
    config.git_credentials[0].token = "git-secret-value".into();
    config.preparation.setup_commands = vec!["login --token setup-secret-value".into()];
    config.preparation.mcp_servers = serde_json::from_value(serde_json::json!({
        "local": {
            "command": "mcp",
            "args": ["--token", "mcp-argument-secret-value"],
            "env": {"TOKEN": "mcp-environment-secret-value"}
        },
        "remote": {
            "url": "https://example.com/mcp",
            "headers": {"Authorization": "Bearer mcp-header-secret-value"}
        }
    }))
    .unwrap();

    for output in [
        format!("{config:?}"),
        format!("{config:#?}"),
        format!("{:?}", config.preparation),
        format!("{:#?}", config.preparation),
    ] {
        for secret in [
            "managed-secret-value",
            "sk-ant-key",
            "environment-secret-value",
            "git-secret-value",
            "setup-secret-value",
            "mcp-argument-secret-value",
            "mcp-environment-secret-value",
            "mcp-header-secret-value",
        ] {
            assert!(!output.contains(secret), "Debug output exposed {secret}");
        }
        assert!(output.contains("mcp_server_count"));
        assert!(output.contains("computer_use_enabled"));
    }
    let output = format!("{config:?}");
    assert!(output.contains("abc123"));
    assert!(output.contains("managed_secret_count: 3"));
    assert!(output.contains("git_credential_count: 1"));
}

#[test]
fn ready_status_sends_the_fingerprint_as_the_manifest_hash() {
    let ready = PrewarmedSandboxStatusInput {
        fingerprint: "abc123".to_string(),
        state: PrewarmedSandboxState::Ready.as_wire_value(),
        manifest_hash: Some("abc123".to_string()),
    };
    assert_eq!(
        serde_json::to_value(&ready).unwrap(),
        serde_json::json!({
            "fingerprint": "abc123",
            "state": "ready",
            "manifest_hash": "abc123",
        })
    );

    let failed = PrewarmedSandboxStatusInput {
        fingerprint: "abc123".to_string(),
        state: PrewarmedSandboxState::Failed.as_wire_value(),
        manifest_hash: None,
    };
    assert_eq!(
        serde_json::to_value(&failed).unwrap(),
        serde_json::json!({ "fingerprint": "abc123", "state": "failed" })
    );
}

#[test]
fn heartbeat_is_the_only_sandbox_message() {
    assert_eq!(
        serde_json::to_value(PrewarmedSandboxControlRequest::Heartbeat {
            generation: "abc123"
        })
        .unwrap(),
        serde_json::json!({ "type": "heartbeat", "generation": "abc123" })
    );
}

#[test]
fn decodes_retirement_with_and_without_a_reason() {
    for (message, expected) in [
        (
            r#"{"type":"retired","reason":"unavailable"}"#,
            Some("unavailable"),
        ),
        (r#"{"type":"retired"}"#, None),
    ] {
        let event: PrewarmedSandboxControlEvent = serde_json::from_str(message).unwrap();
        let PrewarmedSandboxControlEvent::Retired { reason } = event else {
            panic!("expected retirement");
        };
        assert_eq!(reason.as_deref(), expected);
    }
}

/// A server can add message types without breaking a sandbox that is already waiting.
#[test]
fn decodes_messages_this_client_does_not_know_as_unknown() {
    for message in [
        r#"{"type":"offer"}"#,
        r#"{"type":"configuration_updated","generation":"def456"}"#,
    ] {
        let event: PrewarmedSandboxControlEvent = serde_json::from_str(message).unwrap();
        assert!(matches!(event, PrewarmedSandboxControlEvent::Unknown));
    }
}

/// The server pings an idle socket, and a ping must not end the connection.
#[test]
fn control_reader_skips_non_text_frames() {
    let mut reader = PrewarmedSandboxControlReader {
        stream: Box::pin(futures::stream::iter([
            Ok::<_, websocket::Error>(websocket::Message::new_binary(vec![0])),
            Ok(websocket::Message::new_text(
                r#"{"type":"retired","reason":"unavailable"}"#.into(),
            )),
        ])),
    };
    let Some(PrewarmedSandboxControlEvent::Retired { reason }) =
        futures::executor::block_on(reader.next()).unwrap()
    else {
        panic!("expected retirement");
    };
    assert_eq!(reason.as_deref(), Some("unavailable"));
}

#[test]
fn bounds_a_stalled_request() {
    let error = futures::executor::block_on(bounded_request(
        Duration::ZERO,
        futures::future::pending::<anyhow::Result<()>>(),
    ))
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::TimedOut,
    );
}

#[test]
fn request_deadline_preserves_completed_results() {
    let value =
        futures::executor::block_on(bounded_request(REQUEST_TIMEOUT, async { Ok(42) })).unwrap();
    assert_eq!(value, 42);
    let error = futures::executor::block_on(bounded_request::<()>(REQUEST_TIMEOUT, async {
        anyhow::bail!("credential failure")
    }))
    .unwrap_err();
    assert_eq!(error.to_string(), "credential failure");
}

#[test]
fn fresh_assignment_fixture_decodes_nullable_control_fields() {
    let fixture = include_str!("testdata/prewarmed_assignment_fresh_run.json");
    let event: PrewarmedSandboxControlEvent = serde_json::from_str(fixture).unwrap();
    let PrewarmedSandboxControlEvent::Assignment {
        generation,
        execution_config,
        runtime_env,
        api_key,
        expires_at,
    } = event
    else {
        panic!("expected assignment event");
    };
    assert_eq!(generation, "generation-abc");
    assert_eq!(
        execution_config.task_id.inner(),
        "550e8400-e29b-41d4-a716-446655440000"
    );
    assert_eq!(execution_config.execution_id.inner(), "42");
    assert_eq!(runtime_env["WARP_SANDBOX_DEADLINE"], "1767225600");
    assert!(execution_config.conversation_id.is_none());
    assert!(execution_config.repositories[0].ref_.is_none());
    assert!(
        execution_config
            .inference_providers
            .as_ref()
            .unwrap()
            .aws_bedrock
            .as_ref()
            .unwrap()
            .region
            .is_none()
    );
    assert_eq!(api_key, "wk-placeholder-not-a-real-key");
    assert_eq!(expires_at.to_rfc3339(), "2026-01-01T00:00:00+00:00");
}

#[test]
fn assignment_debug_omits_run_credentials_and_embedded_config_values() {
    let mut event: PrewarmedSandboxControlEvent =
        serde_json::from_str(include_str!("testdata/prewarmed_assignment_fresh_run.json")).unwrap();
    if let PrewarmedSandboxControlEvent::Assignment {
        api_key,
        runtime_env,
        execution_config,
        ..
    } = &mut event
    {
        *api_key = "assignment-key-marker".into();
        runtime_env.insert("SECRET".into(), "assignment-env-marker".into());
        execution_config.mcp_servers_json = "assignment-mcp-marker".into();
        execution_config.setup_commands = vec!["assignment-setup-marker".into()];
    }
    for debug in [format!("{event:?}"), format!("{event:#?}")] {
        for marker in [
            "assignment-key-marker",
            "assignment-env-marker",
            "assignment-mcp-marker",
            "assignment-setup-marker",
        ] {
            assert!(!debug.contains(marker));
        }
    }
}

#[cfg(not(feature = "skip_login"))]
fn authenticated_client() -> ServerApi {
    authenticated_client_with_http(http_client::Client::new_for_test())
}

#[cfg(not(feature = "skip_login"))]
fn authenticated_client_with_http(client: http_client::Client) -> ServerApi {
    let auth_state = Arc::new(crate::auth::auth_state::AuthState::new_for_test());
    auth_state.set_credentials(Some(crate::auth::credentials::Credentials::ApiKey {
        key: "test-access-token".to_string(),
        owner_type: None,
    }));
    let (events, _) = async_channel::unbounded();
    let api = ServerApi::new_with_parts(
        Arc::new(client),
        Arc::new(http_client::Client::new_external()),
        auth_state,
        events,
        None,
        None,
        crate::server::telemetry::TelemetryApi::new(),
    );
    api.base_client.set_ambient_workload_token_for_test(
        "test-workload-token".to_string(),
        Some(Utc::now() + chrono::TimeDelta::hours(1)),
    );
    api
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn all_endpoints_use_cached_credentials_and_expected_payloads() {
    let api = authenticated_client();
    let mut server = ChannelState::mock_server();
    let config = server
        .mock("POST", "/graphql/v2?op=PrewarmedSandboxConfig")
        .match_header("authorization", "Bearer test-access-token")
        .match_body(Matcher::PartialJson(serde_json::json!({
            "variables": {
                "input": { "sandboxId": "sandbox" }
            }
        })))
        .with_status(200)
        .with_body(CONFIGURATION_JSON)
        .create();
    let status = server
        .mock("POST", "/internal/prewarmed-sandboxes/sandbox/status")
        .match_header("authorization", "Bearer test-access-token")
        .match_header(
            PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER,
            "test-workload-token",
        )
        .match_body(Matcher::Json(serde_json::json!({
            "fingerprint": "abc123", "state": "ready", "manifest_hash": "abc123",
        })))
        .with_status(204)
        .create();
    drop(server);
    block_on(async {
        assert_eq!(
            api.fetch_configuration("sandbox").await.unwrap().generation,
            "abc123"
        );
        api.report_status("sandbox", "abc123", PrewarmedSandboxState::Ready)
            .await
            .unwrap();
    });
    config.assert();
    status.assert();
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn request_debug_redacts_both_credentials() {
    let api = authenticated_client();
    let mut client = http_client::Client::new_for_test();
    client.set_before_request_fn(Box::new(|request, _| {
        for name in ["authorization", PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER] {
            assert!(request.headers()[name].is_sensitive());
        }
        let debug = format!("{request:?}");
        assert!(!debug.contains("test-access-token"));
        assert!(!debug.contains("test-workload-token"));
    }));
    let mut server = ChannelState::mock_server();
    let request = server
        .mock("POST", "/internal/prewarmed-sandboxes/sandbox/status")
        .match_header("authorization", "Bearer test-access-token")
        .match_header(
            PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER,
            "test-workload-token",
        )
        .with_status(204)
        .expect(1)
        .create();
    drop(server);
    let url = ServerApi::prewarmed_sandbox_url("sandbox", "status");

    block_on(async {
        let response = api
            .prewarmed_sandbox_request(client.post(&url))
            .await
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 204);
    });
    request.assert();
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn rejects_requests_without_access_credentials() {
    let api = ServerApi::new_for_test();
    api.base_client
        .set_ambient_workload_token_for_test("test-workload-token".to_string(), None);
    let error =
        block_on(api.report_status("sandbox", "abc123", PrewarmedSandboxState::Ready)).unwrap_err();
    assert!(
        error.to_string().contains("requires an access token"),
        "{error:#}"
    );
}
